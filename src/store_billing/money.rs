use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

const NANO_USD_PER_CENT: i128 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Currency {
    CNY,
    USD,
}

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "CNY" => Ok(Self::CNY),
            "USD" => Ok(Self::USD),
            _ => Err(serde::de::Error::custom("invalid_currency")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Money {
    pub currency: Currency,
    pub minor: String,
}

impl Money {
    pub fn new(currency: Currency, minor: impl Into<String>) -> Result<Self, MoneyError> {
        let minor = minor.into();
        parse_minor(&minor)?;
        Ok(Self { currency, minor })
    }

    pub fn minor_value(&self) -> Result<i128, MoneyError> {
        parse_minor(&self.minor)
    }
}

impl<'de> Deserialize<'de> for Money {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireMoney {
            currency: Currency,
            minor: String,
        }

        let wire = WireMoney::deserialize(deserializer)?;
        Money::new(wire.currency, wire.minor).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeRateRational {
    decimal: String,
    numerator: i128,
    denominator: i128,
}

impl ExchangeRateRational {
    pub fn parse(value: &str) -> Result<Self, MoneyError> {
        let (integer, fraction) = match value.split_once('.') {
            Some((integer, fraction)) => (integer, Some(fraction)),
            None => (value, None),
        };
        if integer.is_empty()
            || !integer.bytes().all(|byte| byte.is_ascii_digit())
            || (integer.len() > 1 && integer.starts_with('0'))
            || fraction.is_some_and(|digits| {
                digits.is_empty()
                    || digits.len() > 18
                    || !digits.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(MoneyError::InvalidExchangeRate);
        }

        let scale = fraction.map_or(0_u32, |digits| digits.len() as u32);
        let denominator = checked_pow10(scale).ok_or(MoneyError::AmountOverflow)?;
        let whole = integer
            .parse::<i128>()
            .map_err(|_| MoneyError::AmountOverflow)?;
        let fractional = fraction.unwrap_or_default().parse::<i128>().unwrap_or(0);
        let numerator = whole
            .checked_mul(denominator)
            .and_then(|value| value.checked_add(fractional))
            .ok_or(MoneyError::AmountOverflow)?;
        if numerator == 0 {
            return Err(MoneyError::InvalidExchangeRate);
        }
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            decimal: value.to_string(),
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    pub const fn numerator(&self) -> i128 {
        self.numerator
    }

    pub const fn denominator(&self) -> i128 {
        self.denominator
    }

    pub fn decimal(&self) -> &str {
        &self.decimal
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MoneyError {
    #[error("amount must be a canonical nonnegative integer string")]
    InvalidAmount,
    #[error("exchange rate must be a positive canonical decimal")]
    InvalidExchangeRate,
    #[error("amount exceeds the supported integer range")]
    AmountOverflow,
}

pub fn parse_minor(value: &str) -> Result<i128, MoneyError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(MoneyError::InvalidAmount);
    }
    value.parse().map_err(|_| MoneyError::AmountOverflow)
}

pub(crate) fn parse_signed_minor(value: &str) -> Result<i128, MoneyError> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty()
        || value == "-0"
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return Err(MoneyError::InvalidAmount);
    }
    value.parse().map_err(|_| MoneyError::AmountOverflow)
}

pub fn convert_minor(
    amount: i128,
    source: Currency,
    target: Currency,
    cny_per_usd: &str,
) -> Result<i128, MoneyError> {
    let rate = ExchangeRateRational::parse(cny_per_usd)?;
    convert_minor_rational(amount, source, target, &rate)
}

pub fn convert_minor_rational(
    amount: i128,
    source: Currency,
    target: Currency,
    rate: &ExchangeRateRational,
) -> Result<i128, MoneyError> {
    require_nonnegative(amount)?;
    match (source, target) {
        (Currency::USD, Currency::CNY) => {
            checked_round_product_ratio(amount, rate.numerator, rate.denominator)
        }
        (Currency::CNY, Currency::USD) => {
            checked_round_product_ratio(amount, rate.denominator, rate.numerator)
        }
        _ => Ok(amount),
    }
}

pub fn quoted_received_to_nano_usd(
    amount_minor: i128,
    currency: Currency,
    cny_per_usd: &str,
) -> Result<i128, MoneyError> {
    let rate = ExchangeRateRational::parse(cny_per_usd)?;
    match currency {
        Currency::USD => amount_minor
            .checked_mul(NANO_USD_PER_CENT)
            .ok_or(MoneyError::AmountOverflow),
        Currency::CNY => cny_fen_to_nano_usd(amount_minor, &rate),
    }
}

pub fn cny_fen_to_nano_usd(
    amount_fen: i128,
    rate: &ExchangeRateRational,
) -> Result<i128, MoneyError> {
    require_nonnegative(amount_fen)?;
    let scaled_denominator = rate
        .denominator
        .checked_mul(NANO_USD_PER_CENT)
        .ok_or(MoneyError::AmountOverflow)?;
    checked_round_product_ratio(amount_fen, scaled_denominator, rate.numerator)
}

pub fn nano_usd_to_cny_fen(
    amount_nano_usd: i128,
    rate: &ExchangeRateRational,
) -> Result<i128, MoneyError> {
    require_nonnegative(amount_nano_usd)?;
    let denominator = rate
        .denominator
        .checked_mul(NANO_USD_PER_CENT)
        .ok_or(MoneyError::AmountOverflow)?;
    checked_round_product_ratio(amount_nano_usd, rate.numerator, denominator)
}

pub fn plan_quota_whole_units(
    quota_fen_cny: i128,
    display_currency: Currency,
    cny_per_usd: &str,
) -> Result<i128, MoneyError> {
    require_nonnegative(quota_fen_cny)?;
    let rate = ExchangeRateRational::parse(cny_per_usd)?;
    match display_currency {
        Currency::CNY => round_nonnegative_ratio(quota_fen_cny, 100),
        Currency::USD => {
            let denominator = rate
                .numerator
                .checked_mul(100)
                .ok_or(MoneyError::AmountOverflow)?;
            checked_round_product_ratio(quota_fen_cny, rate.denominator, denominator)
        }
    }
}

pub(crate) fn parse_rate(value: &str) -> Result<ExchangeRateRational, MoneyError> {
    ExchangeRateRational::parse(value)
}

fn checked_round_product_ratio(
    value: i128,
    multiplier: i128,
    denominator: i128,
) -> Result<i128, MoneyError> {
    let numerator = value
        .checked_mul(multiplier)
        .ok_or(MoneyError::AmountOverflow)?;
    round_nonnegative_ratio(numerator, denominator)
}

fn round_nonnegative_ratio(numerator: i128, denominator: i128) -> Result<i128, MoneyError> {
    if numerator < 0 || denominator <= 0 {
        return Err(MoneyError::InvalidAmount);
    }
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder != 0 && remainder >= denominator - remainder {
        quotient.checked_add(1).ok_or(MoneyError::AmountOverflow)
    } else {
        Ok(quotient)
    }
}

fn require_nonnegative(value: i128) -> Result<(), MoneyError> {
    if value < 0 {
        Err(MoneyError::InvalidAmount)
    } else {
        Ok(())
    }
}

const fn gcd(mut left: i128, mut right: i128) -> i128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

const fn checked_pow10(scale: u32) -> Option<i128> {
    let mut value = 1_i128;
    let mut index = 0;
    while index < scale {
        value = match value.checked_mul(10) {
            Some(next) => next,
            None => return None,
        };
        index += 1;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::{
        Currency, Money, convert_minor, parse_minor, plan_quota_whole_units,
        quoted_received_to_nano_usd,
    };

    #[test]
    fn parses_only_canonical_nonnegative_minor_units() {
        assert_eq!(parse_minor("0").unwrap(), 0);
        assert_eq!(parse_minor("123456789").unwrap(), 123_456_789);

        for invalid in ["", "00", "01", "+1", "-1", "1.0", " 1", "1 "] {
            assert!(parse_minor(invalid).is_err(), "accepted {invalid:?}");
        }
        assert_eq!(
            parse_minor(&"9".repeat(100)),
            Err(super::MoneyError::AmountOverflow)
        );
    }

    #[test]
    fn unsupported_currency_has_a_stable_error_marker() {
        let error = serde_json::from_str::<Currency>(r#""EUR""#).unwrap_err();
        assert!(error.to_string().contains("invalid_currency"));
    }

    #[test]
    fn money_serializes_currency_as_the_api_codes() {
        let money = Money::new(Currency::CNY, "123").unwrap();
        assert_eq!(
            serde_json::to_string(&money).unwrap(),
            r#"{"currency":"CNY","minor":"123"}"#
        );
        assert_eq!(
            serde_json::from_str::<Money>(r#"{"currency":"USD","minor":"45"}"#).unwrap(),
            Money::new(Currency::USD, "45").unwrap()
        );
    }

    #[test]
    fn converts_minor_units_at_the_exact_decimal_rate() {
        assert_eq!(
            convert_minor(100, Currency::USD, Currency::CNY, "6.7370").unwrap(),
            674
        );
        assert_eq!(
            convert_minor(6_737, Currency::CNY, Currency::USD, "6.7370").unwrap(),
            1_000
        );
    }

    #[test]
    fn conversion_rounds_half_away_from_zero() {
        assert_eq!(
            convert_minor(1, Currency::USD, Currency::CNY, "0.5").unwrap(),
            1
        );
        assert_eq!(
            convert_minor(1, Currency::CNY, Currency::USD, "2").unwrap(),
            1
        );
    }

    #[test]
    fn converts_the_quoted_received_amount_directly_to_nano_usd() {
        assert_eq!(
            quoted_received_to_nano_usd(125, Currency::USD, "6.7370").unwrap(),
            1_250_000_000
        );
        assert_eq!(
            quoted_received_to_nano_usd(6_737, Currency::CNY, "6.7370").unwrap(),
            10_000_000_000
        );
    }

    #[test]
    fn plan_quota_display_uses_whole_units() {
        assert_eq!(
            plan_quota_whole_units(2_000, Currency::CNY, "6.7370").unwrap(),
            20
        );
        assert_eq!(
            plan_quota_whole_units(2_000, Currency::USD, "6.7370").unwrap(),
            3
        );
        assert_eq!(
            plan_quota_whole_units(6_800, Currency::USD, "6.7370").unwrap(),
            10
        );
    }
}
