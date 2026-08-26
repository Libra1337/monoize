use rust_decimal::{Decimal, RoundingStrategy, prelude::ToPrimitive};
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;
use thiserror::Error;

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

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MoneyError {
    #[error("amount must be a canonical nonnegative integer string")]
    InvalidAmount,
    #[error("exchange rate must be a positive finite decimal")]
    InvalidExchangeRate,
    #[error("amount exceeds the supported decimal range")]
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

pub fn convert_minor(
    amount: i128,
    source: Currency,
    target: Currency,
    cny_per_usd: &str,
) -> Result<i128, MoneyError> {
    let amount = nonnegative_decimal(amount, 0)?;
    let rate = parse_rate(cny_per_usd)?;
    let converted = match (source, target) {
        (Currency::USD, Currency::CNY) => amount.checked_mul(rate),
        (Currency::CNY, Currency::USD) => amount.checked_div(rate),
        _ => Some(amount),
    }
    .ok_or(MoneyError::AmountOverflow)?;

    rounded_nonnegative_i128(converted)
}

pub fn quoted_received_to_nano_usd(
    amount_minor: i128,
    currency: Currency,
    cny_per_usd: &str,
) -> Result<i128, MoneyError> {
    let amount = nonnegative_decimal(amount_minor, 0)?;
    let rate = parse_rate(cny_per_usd)?;
    let nano_per_minor = Decimal::from(10_000_000u64);
    let nano_usd = amount
        .checked_mul(nano_per_minor)
        .and_then(|amount| match currency {
            Currency::USD => Some(amount),
            Currency::CNY => amount.checked_div(rate),
        })
        .ok_or(MoneyError::AmountOverflow)?;

    rounded_nonnegative_i128(nano_usd)
}

pub fn plan_quota_whole_units(
    quota_fen_cny: i128,
    display_currency: Currency,
    cny_per_usd: &str,
) -> Result<i128, MoneyError> {
    let quota_cny = nonnegative_decimal(quota_fen_cny, 2)?;
    let rate = parse_rate(cny_per_usd)?;
    let displayed = match display_currency {
        Currency::CNY => quota_cny,
        Currency::USD => quota_cny
            .checked_div(rate)
            .ok_or(MoneyError::AmountOverflow)?,
    };

    rounded_nonnegative_i128(displayed)
}

pub(crate) fn parse_rate(value: &str) -> Result<Decimal, MoneyError> {
    let mut components = value.split('.');
    let integer = components.next().unwrap_or_default();
    let fraction = components.next();
    let syntax_is_valid = !integer.is_empty()
        && integer.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
        && components.next().is_none();
    if !syntax_is_valid {
        return Err(MoneyError::InvalidExchangeRate);
    }

    let rate = Decimal::from_str(value).map_err(|_| MoneyError::InvalidExchangeRate)?;
    if rate <= Decimal::ZERO {
        return Err(MoneyError::InvalidExchangeRate);
    }
    Ok(rate)
}

fn nonnegative_decimal(value: i128, scale: u32) -> Result<Decimal, MoneyError> {
    if value < 0 {
        return Err(MoneyError::InvalidAmount);
    }
    Decimal::try_from_i128_with_scale(value, scale).map_err(|_| MoneyError::AmountOverflow)
}

fn rounded_nonnegative_i128(value: Decimal) -> Result<i128, MoneyError> {
    value
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i128()
        .filter(|value| *value >= 0)
        .ok_or(MoneyError::AmountOverflow)
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
