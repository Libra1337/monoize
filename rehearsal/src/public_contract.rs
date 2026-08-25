use serde::{Deserialize, Serialize};

const MAX_PUBLIC_RESPONSE_BYTES: usize = 1_048_576;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteResponse {
    pub site_name: String,
    pub site_description: String,
    pub api_base_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateRange {
    pub min: String,
    pub max: String,
    pub unit: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketplaceItem {
    pub public_group_name: String,
    pub model: String,
    pub capabilities: Vec<String>,
    pub input_rate_range: Option<RateRange>,
    pub output_rate_range: Option<RateRange>,
    pub offer_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketplaceListResponse {
    pub generated_at: String,
    pub revision: String,
    pub next_cursor: Option<String>,
    pub items: Vec<MarketplaceItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicResponseError {
    Encode,
    TooLarge,
}

pub fn encode_public<T: Serialize>(value: &T) -> Result<Vec<u8>, PublicResponseError> {
    serde_json::to_vec(value).map_err(|_| PublicResponseError::Encode)
}

pub fn encode_marketplace_bounded(
    value: MarketplaceListResponse,
) -> Result<Vec<u8>, PublicResponseError> {
    let bytes = encode_public(&value)?;
    if bytes.len() > MAX_PUBLIC_RESPONSE_BYTES {
        return Err(PublicResponseError::TooLarge);
    }
    Ok(bytes)
}
