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

/// Minimum withdrawal in Coin minor units (SC-5.1).
///
/// One minor unit: any positive balance is withdrawable. A higher floor would strand a small
/// balance the agent earned but could never take.
pub const MIN_WITHDRAWAL_MINOR: i128 = 1;

/// Minimum face value of an order carrying a sales code, i.e. 1 CNY (SC-2.7a).
///
/// At the 500 bp default this is the smallest face value whose commission is still nonzero
/// and still representable in two decimal places: 1 CNY yields exactly 5 minor units. One
/// minor unit would yield `floor(1 * 500 / 10000) = 0`, crediting nothing for a real sale.
pub const MIN_CODED_ORDER_MINOR: i128 = 100;

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
    /// The face value is below `MIN_CODED_ORDER_MINOR` (SC-2.7a).
    AmountTooSmall,
}

/// The four amounts an order carries once a code is applied (SC-1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesAmounts {
    /// Face value: what the buyer receives and pays, unaffected by the discount.
    pub base_minor: i128,
    /// What the agent earns: the discount rate applied to the face value.
    pub discount_minor: i128,
    /// What the buyer pays: always the full face value (SC-1.3).
    pub payment_minor: i128,
    /// What the agent earns: equal to `discount_minor`.
    pub commission_minor: i128,
}

/// Computes the SC-1.3 amounts for one order.
///
/// The buyer always pays the full face value; the sales code never reduces the payment or
/// the credited balance. The agent earns `floor(base * discount_bp / 10000)`, so the
/// discount rate is the agent's commission rate on that order, and platform revenue is
/// `payment_minor - commission_minor` (SC-1.4). The global `commission_rate_bp` no longer
/// enters the arithmetic; it only caps `discount_bp`.
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
    // SC-2.7a: below 1 CNY a 1% commission floors to zero, so the sale would credit nothing.
    if base_minor < MIN_CODED_ORDER_MINOR {
        return Err(SalesError::AmountTooSmall);
    }

    let commission_minor = base_minor
        .checked_mul(i128::from(discount_bp))
        .ok_or(SalesError::InvalidAmount)?
        / BP_DENOMINATOR;

    // The commission is paid out of the payment, so it can never exceed the face value.
    if commission_minor > base_minor {
        return Err(SalesError::InvalidAmount);
    }
    Ok(SalesAmounts {
        base_minor,
        discount_minor: commission_minor,
        payment_minor: base_minor,
        commission_minor,
    })
}

/// Commission for a retroactive claim (SC-4.3).
///
/// The buyer already paid the full face value, so a claim credits the agent's own discount
/// rate — the same rate a coded order would have credited — bounded by the global cap.
pub fn claim_commission(
    base_minor: i128,
    commission_rate_bp: i64,
    agent_discount_bp: i64,
) -> Result<i128, SalesError> {
    compute_amounts(base_minor, commission_rate_bp, agent_discount_bp)
        .map(|amounts| amounts.commission_minor)
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

/// Generates the one-time password returned at agent creation (SC-7.2).
///
/// 20 characters over the 32-character code alphabet is 100 bits of entropy. It reuses that
/// alphabet so the operator can read the password aloud under the same rules as the code.
pub fn generate_password() -> String {
    let mut random = [0_u8; 20];
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

    /// SC-1.3 worked example, and SC-1.4: the buyer always pays the face value and the
    /// commission comes out of the payment, so platform revenue falls only by the commission.
    #[test]
    fn the_buyer_pays_the_face_value_and_funds_the_commission() {
        let base = 10_000; // 100 CNY

        let without = compute_amounts(base, 500, 0).expect("no discount");
        assert_eq!(without.payment_minor, 10_000);
        assert_eq!(without.commission_minor, 0);
        assert_eq!(without.payment_minor - without.commission_minor, 10_000);

        let with_one_percent = compute_amounts(base, 500, 100).expect("1% rate");
        assert_eq!(with_one_percent.discount_minor, 100);
        assert_eq!(with_one_percent.payment_minor, 10_000);
        assert_eq!(with_one_percent.commission_minor, 100);
        assert_eq!(
            with_one_percent.payment_minor - with_one_percent.commission_minor,
            9_900
        );

        let at_the_cap = compute_amounts(base, 500, 500).expect("5% rate");
        assert_eq!(at_the_cap.payment_minor, 10_000);
        assert_eq!(at_the_cap.commission_minor, 500);
        assert_eq!(at_the_cap.payment_minor - at_the_cap.commission_minor, 9_500);

        // The buyer always receives the face value and pays it in full.
        for amounts in [without, with_one_percent, at_the_cap] {
            assert_eq!(amounts.base_minor, base);
            assert_eq!(amounts.payment_minor, base);
        }

        // The reported example: a 5 CNY recharge at a 1% sales rate pays 5 CNY and earns
        // the agent exactly 0.05 CNY, with nothing deducted at payment time.
        let example = compute_amounts(500, 500, 100).expect("5 CNY at 1%");
        assert_eq!(example.payment_minor, 500);
        assert_eq!(example.commission_minor, 5);
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
        // 199 fen at a 5% rate is 9.95 fen of commission.
        let odd = compute_amounts(199, 500, 500).expect("199 fen at 5%");
        assert_eq!(odd.commission_minor, 9);

        // A rate that floors to zero earns nothing but never changes the payable amount.
        let sub_fen_rate = compute_amounts(100, 500, 1).expect("0.01% of 1 CNY");
        assert_eq!(sub_fen_rate.discount_minor, 0);
        assert_eq!(sub_fen_rate.payment_minor, 100);
    }

    /// SC-2.7a: 1 CNY is the smallest face value whose commission is nonzero and expressible
    /// in two decimal places at a 1% rate. Anything smaller would credit the agent nothing
    /// for a real sale.
    #[test]
    fn one_yuan_is_the_smallest_face_value_that_earns_anything() {
        let one_yuan = compute_amounts(MIN_CODED_ORDER_MINOR, 500, 100).expect("1 CNY at 1%");
        assert_eq!(one_yuan.commission_minor, 1); // 0.01 CNY
        assert_eq!(one_yuan.payment_minor, 100);

        for below in [1, 50, MIN_CODED_ORDER_MINOR - 1] {
            assert_eq!(
                compute_amounts(below, 500, 0),
                Err(SalesError::AmountTooSmall),
                "{below} minor units must be refused"
            );
        }
    }

    #[test]
    fn a_zero_or_negative_face_value_is_rejected() {
        assert_eq!(compute_amounts(0, 500, 0), Err(SalesError::InvalidAmount));
        assert_eq!(compute_amounts(-100, 500, 0), Err(SalesError::InvalidAmount));
    }

    /// SC-4.3: a claim credits the agent's own rate, bounded by the configured cap.
    #[test]
    fn a_claim_earns_the_agents_own_rate() {
        assert_eq!(claim_commission(10_000, 500, 100), Ok(100));
        assert_eq!(claim_commission(10_000, 300, 300), Ok(300));
        assert_eq!(claim_commission(10_000, 500, 0), Ok(0));
        assert_eq!(
            claim_commission(10_000, 300, 400),
            Err(SalesError::DiscountAboveRate)
        );
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
