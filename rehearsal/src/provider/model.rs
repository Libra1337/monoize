use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalDecimal(String);

impl CanonicalDecimal {
    pub fn parse(value: &str) -> Result<Self, ModelError> {
        if value.is_empty()
            || value.starts_with('+')
            || value.contains(['e', 'E'])
            || fractional_digits(value) > 9
        {
            return Err(ModelError::Decimal);
        }
        let decimal = Decimal::from_str(value).map_err(|_| ModelError::Decimal)?;
        if decimal <= Decimal::ZERO {
            return Err(ModelError::Decimal);
        }
        Ok(Self(decimal.normalize().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CanonicalDecimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn fractional_digits(value: &str) -> usize {
    value
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelKeys {
    pub model_name: String,
    pub name: Vec<u8>,
    pub search: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicNameKey {
    pub name: String,
    pub key: Vec<u8>,
}

impl PublicNameKey {
    pub fn new(value: &str) -> Result<Self, ModelError> {
        let name = value
            .trim_matches(char::is_whitespace)
            .nfc()
            .collect::<String>();
        let scalar_count = name.chars().count();
        if !(1..=64).contains(&scalar_count)
            || name
                .as_bytes()
                .iter()
                .any(|byte| matches!(*byte, 0x00..=0x1f | 0x7f))
        {
            return Err(ModelError::PublicName);
        }
        let key = name.as_bytes().to_vec();
        Ok(Self { name, key })
    }
}

impl ModelKeys {
    pub fn new(value: &str) -> Result<Self, ModelError> {
        let model_name = value.trim_matches(char::is_whitespace).to_owned();
        let name = model_name.as_bytes().to_vec();
        if name.is_empty()
            || name.len() > 256
            || name.iter().any(|byte| matches!(*byte, 0x00..=0x1f | 0x7f))
        {
            return Err(ModelError::ModelName);
        }
        let search = name
            .iter()
            .map(|byte| {
                if byte.is_ascii_uppercase() {
                    byte + 32
                } else {
                    *byte
                }
            })
            .collect();
        Ok(Self {
            model_name,
            name,
            search,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    Decimal,
    ModelName,
    PublicName,
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decimal => formatter.write_str("invalid canonical decimal"),
            Self::ModelName => formatter.write_str("invalid model name"),
            Self::PublicName => formatter.write_str("invalid public name"),
        }
    }
}

impl std::error::Error for ModelError {}
