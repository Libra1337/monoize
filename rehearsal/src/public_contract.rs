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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferRate {
    pub usage_class: String,
    pub unit: String,
    pub display_rate_nano_usd: String,
    pub context_tier: Option<String>,
    pub service_tier: Option<String>,
    pub modality: Option<String>,
    pub cache_ttl: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderOffer {
    pub public_provider_name: String,
    pub public_channel_name: String,
    pub api_type: String,
    pub rates: Vec<OfferRate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferResponse {
    pub generated_at: String,
    pub revision: String,
    pub public_group_name: String,
    pub model: String,
    pub next_cursor: Option<String>,
    pub offers: Vec<ProviderOffer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStateDto {
    Operational,
    MinorDegradation,
    MajorDegradation,
    Unavailable,
    InsufficientData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusProvider {
    pub public_name: String,
    pub state: HealthStateDto,
    pub success_rate_24h_basis_points: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusGroup {
    pub public_name: String,
    pub state: HealthStateDto,
    pub insufficient_provider_count: u64,
    pub providers: Vec<StatusProvider>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusResponse {
    pub generated_at: String,
    pub data_through: String,
    pub data_complete: bool,
    pub groups: Vec<StatusGroup>,
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
