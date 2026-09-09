//! Sales codes, commission accrual, discounts, retroactive claims, and withdrawals.
//!
//! Governed by `spec/sales-commission.spec.md`. Amounts here are Coin minor units, which
//! equal CNY fen because Coin is pegged at `1 C = 1 CNY` (SC-0.3).

use rsa::rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

/// Maximum configurable commission rate in basis points (SC-1.1).
pub const MAX_COMMISSION_RATE_BP: i64 = 2000;

/// Default commission rate in basis points (SC-1.1).
pub const DEFAULT_COMMISSION_RATE_BP: i64 = 500;

/// Minimum withdrawal in Coin minor units, i.e. 100 CNY (SC-5.1).
pub const MIN_WITHDRAWAL_MINOR: i128 = 10_000;

/// Basis-point denominator.
const BP_DENOMINATOR: i128 = 10_000;

/// Sales-code alphabet: Crockford Base32 without `I`, `L`, `O`, and `U` (SC-D1b).
///
/// Those four are excluded because they are the characters a person transcribing a code by
/// eye confuses with `1`, `1`, `0`, and `V`. This is the same 32-character alphabet the
/// redemption codes of `store-billing.spec.md` SB-R-1 use, so an operator reads both kinds of
/// code under one rule. Its length being exactly 32 is what makes the `byte & 31` mapping
/// below uniform over the alphabet.
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Sales-code length in characters (SC-D1b).
const CODE_LENGTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SalesError {
    /// The rate is outside `[0, MAX_COMMISSION_RATE_BP]`.
    RateOutOfRange,
    /// The discount exceeds the commission rate, which would make commission negative.
    DiscountAboveRate,
    /// An amount is not a canonical nonnegative integer, or overflowed.
    InvalidAmount,
}

/// The four amounts an order carries once a code is applied (SC-1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesAmounts {
    /// Face value: what the buyer receives, unaffected by the discount.
    pub base_minor: i128,
    /// What the discount removes from the payable amount.
    pub discount_minor: i128,
    /// What the buyer pays.
    pub payment_minor: i128,
    /// What the agent earns.
    pub commission_minor: i128,
}

/// Computes the SC-1.3 amounts for one order.
///
/// The discount and the commission are both taken from the face value, and the discount is
/// subtracted from the commission rate rather than from platform revenue. Platform revenue is
/// `payment_minor - commission_minor`, which is independent of the discount up to one minor
/// unit of floor rounding (SC-1.4): at a 500 bp rate a 100 CNY face value yields 95 CNY
/// whether the discount is 0% or 5%.
pub fn compute_amounts(
    base_minor: i128,
    commission_rate_bp: i64,
    discount_bp: i64,
) -> Result<SalesAmounts, SalesError> {
    if !(0..=MAX_COMMISSION_RATE_BP).contains(&commission_rate_bp) {
        return Err(SalesError::RateOutOfRange);
    }
    if !(0..=commission_rate_bp).contains(&discount_bp) {
        return Err(SalesError::DiscountAboveRate);
    }
    if base_minor <= 0 {
        return Err(SalesError::InvalidAmount);
    }

    let discount_minor = base_minor
        .checked_mul(i128::from(discount_bp))
        .ok_or(SalesError::InvalidAmount)?
        / BP_DENOMINATOR;
    let payment_minor = base_minor
        .checked_sub(discount_minor)
        .ok_or(SalesError::InvalidAmount)?;
    let commission_minor = base_minor
        .checked_mul(i128::from(commission_rate_bp - discount_bp))
        .ok_or(SalesError::InvalidAmount)?
        / BP_DENOMINATOR;

    // A discount that consumed the whole commission must still leave something payable.
    if payment_minor <= 0 {
        return Err(SalesError::InvalidAmount);
    }
    Ok(SalesAmounts {
        base_minor,
        discount_minor,
        payment_minor,
        commission_minor,
    })
}

/// Commission for a retroactive claim, which carries no discount (SC-4.3).
pub fn claim_commission(base_minor: i128, commission_rate_bp: i64) -> Result<i128, SalesError> {
    compute_amounts(base_minor, commission_rate_bp, 0).map(|amounts| amounts.commission_minor)
}

/// Generates one sales code from a cryptographically secure source (SC-D1b).
pub fn generate_code() -> String {
    let mut random = [0_u8; CODE_LENGTH];
    OsRng.fill_bytes(&mut random);
    random
        .iter()
        .map(|byte| char::from(CODE_ALPHABET[usize::from(byte & 31)]))
        .collect()
}

/// Normalizes a submitted code for comparison (SC-D1b).
///
/// Comparison uppercases and drops ASCII hyphens and spaces so that a code read aloud and
/// typed back with grouping still resolves. A character outside the alphabet leaves the
/// result unresolvable rather than being silently mapped, so a typo fails loudly.
pub fn normalize_code(input: &str) -> Option<String> {
    let normalized: String = input
        .chars()
        .filter(|character| !matches!(character, '-' | ' '))
        .map(|character| character.to_ascii_uppercase())
        .collect();
    if normalized.len() != CODE_LENGTH
        || !normalized
            .bytes()
            .all(|byte| CODE_ALPHABET.contains(&byte))
    {
        return None;
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SC-1.3 worked example, and SC-1.4: platform revenue does not move with the discount.
    #[test]
    fn discount_is_funded_by_commission_not_by_the_platform() {
        let base = 10_000; // 100 CNY

        let without = compute_amounts(base, 500, 0).expect("no discount");
        assert_eq!(without.payment_minor, 10_000);
        assert_eq!(without.commission_minor, 500);
        assert_eq!(without.payment_minor - without.commission_minor, 9_500);

        let with_one_percent = compute_amounts(base, 500, 100).expect("1% discount");
        assert_eq!(with_one_percent.discount_minor, 100);
        assert_eq!(with_one_percent.payment_minor, 9_900);
        assert_eq!(with_one_percent.commission_minor, 400);
        assert_eq!(
            with_one_percent.payment_minor - with_one_percent.commission_minor,
            9_500
        );

        let at_the_cap = compute_amounts(base, 500, 500).expect("5% discount");
        assert_eq!(at_the_cap.payment_minor, 9_500);
        assert_eq!(at_the_cap.commission_minor, 0);
        assert_eq!(at_the_cap.payment_minor - at_the_cap.commission_minor, 9_500);

        // The buyer always receives the face value.
        for amounts in [without, with_one_percent, at_the_cap] {
            assert_eq!(amounts.base_minor, base);
        }
    }

    /// SC-1.2: the bound follows the configured rate, not a hardcoded 500.
    #[test]
    fn discount_may_not_exceed_the_configured_rate() {
        assert_eq!(
            compute_amounts(10_000, 300, 400),
            Err(SalesError::DiscountAboveRate)
        );
        assert!(compute_amounts(10_000, 300, 300).is_ok());
        assert_eq!(
            compute_amounts(10_000, MAX_COMMISSION_RATE_BP + 1, 0),
            Err(SalesError::RateOutOfRange)
        );
        assert_eq!(
            compute_amounts(10_000, 500, -1),
            Err(SalesError::DiscountAboveRate)
        );
    }

    /// Floor division must never round a share up, or the platform pays the rounding.
    #[test]
    fn rounding_never_favours_the_agent() {
        // 1 fen at 5% is 0.05 fen, which floors to zero commission.
        let tiny = compute_amounts(1, 500, 0).expect("one fen");
        assert_eq!(tiny.commission_minor, 0);
        assert_eq!(tiny.payment_minor, 1);

        // 199 fen at 5% is 9.95 fen.
        let odd = compute_amounts(199, 500, 0).expect("199 fen");
        assert_eq!(odd.commission_minor, 9);

        // A discount that floors to zero must not reduce the payable amount.
        let sub_fen_discount = compute_amounts(100, 500, 1).expect("0.01% of 1 CNY");
        assert_eq!(sub_fen_discount.discount_minor, 0);
        assert_eq!(sub_fen_discount.payment_minor, 100);
    }

    #[test]
    fn a_zero_or_negative_face_value_is_rejected() {
        assert_eq!(
            compute_amounts(0, 500, 0),
            Err(SalesError::InvalidAmount)
        );
        assert_eq!(
            compute_amounts(-100, 500, 0),
            Err(SalesError::InvalidAmount)
        );
    }

    #[test]
    fn a_claim_earns_the_undiscounted_commission() {
        assert_eq!(claim_commission(10_000, 500), Ok(500));
        assert_eq!(claim_commission(10_000, 300), Ok(300));
    }

    /// SC-D1b: the alphabet excludes the four characters that are misread when transcribed.
    #[test]
    fn generated_codes_use_the_unambiguous_alphabet() {
        for _ in 0..64 {
            let code = generate_code();
            assert_eq!(code.len(), CODE_LENGTH, "{code}");
            assert!(
                code.bytes().all(|byte| CODE_ALPHABET.contains(&byte)),
                "{code}"
            );
            assert!(!code.contains(['I', 'L', 'O', 'U']), "{code}");
            // A generated code must survive its own normalization unchanged.
            assert_eq!(normalize_code(&code).as_deref(), Some(code.as_str()));
        }
    }

    #[test]
    fn normalization_accepts_grouping_and_case_but_not_typos() {
        assert_eq!(normalize_code("abcd2345").as_deref(), Some("ABCD2345"));
        assert_eq!(normalize_code("ABCD-2345").as_deref(), Some("ABCD2345"));
        assert_eq!(normalize_code(" ABCD 2345 ").as_deref(), Some("ABCD2345"));

        // Wrong length, and characters outside the alphabet.
        assert_eq!(normalize_code("ABCD234"), None);
        assert_eq!(normalize_code("ABCD23456"), None);
        assert_eq!(normalize_code("ABCD234I"), None);
        assert_eq!(normalize_code("ABCD234O"), None);
        assert_eq!(normalize_code(""), None);
    }
}
