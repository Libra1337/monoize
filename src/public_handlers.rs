use crate::app::AppState;
use crate::billing_rate_store::{DbBillingRateRecord, select_pricing_profile};
use crate::error::{AppError, AppResult};
use crate::exact_decimal::Multiplier;
use crate::model_registry::ModelCapabilities;
use crate::monoize_routing::{MonoizeChannel, MonoizeProvider};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::{SecondsFormat, Utc};
use rust_decimal::Decimal;
use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Snapshot {
    revision: u64,
    created_at: Instant,
    generated_at: String,
}

static SNAPSHOT: OnceLock<Mutex<Option<Snapshot>>> = OnceLock::new();

fn snapshot(revision: u64) -> Snapshot {
    let storage = SNAPSHOT.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = storage.lock() {
        if let Some(current) = guard.as_ref()
            && current.revision == revision
            && current.created_at.elapsed() < Duration::from_secs(60)
        {
            return current.clone();
        }
        let next = Snapshot {
            revision,
            created_at: Instant::now(),
            generated_at: now_string(),
        };
        *guard = Some(next.clone());
        return next;
    }
    Snapshot {
        revision,
        created_at: Instant::now(),
        generated_at: now_string(),
    }
}

#[derive(Debug, Deserialize)]
pub struct MarketplaceQuery {
    pub q: Option<String>,
    pub group: Option<String>,
    pub model: Option<String>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct RateRange {
    min: String,
    max: String,
    unit: String,
}

#[derive(Debug, Serialize, Clone)]
struct PublicRate {
    usage_class: String,
    unit: String,
    display_rate_nano_usd: String,
    context_tier: Option<String>,
    service_tier: Option<String>,
    modality: Option<String>,
    cache_ttl: Option<String>,
}

#[derive(Debug, Serialize)]
struct MarketplaceItem {
    public_group_name: String,
    model: String,
    capabilities: Vec<String>,
    input_rate_range: Option<RateRange>,
    output_rate_range: Option<RateRange>,
    offer_count: usize,
}

#[derive(Debug, Serialize)]
struct MarketplaceResponse {
    generated_at: String,
    revision: String,
    next_cursor: Option<String>,
    items: Vec<MarketplaceItem>,
}

#[derive(Debug, Serialize)]
struct MarketplaceOffer {
    public_provider_name: String,
    public_channel_name: String,
    api_type: String,
    rates: Vec<PublicRate>,
}

#[derive(Debug, Serialize)]
struct OffersResponse {
    generated_at: String,
    revision: String,
    public_group_name: String,
    model: String,
    next_cursor: Option<String>,
    offers: Vec<MarketplaceOffer>,
}

#[derive(Debug, Serialize, Clone)]
struct PublicStatusProvider {
    public_name: String,
    state: &'static str,
    success_rate_24h_basis_points: Option<u32>,
}

#[derive(Debug, Serialize)]
struct PublicStatusGroup {
    public_name: String,
    state: &'static str,
    insufficient_provider_count: usize,
    providers: Vec<PublicStatusProvider>,
}

#[derive(Debug, Serialize)]
struct PublicStatusResponse {
    generated_at: String,
    data_through: String,
    data_complete: bool,
    groups: Vec<PublicStatusGroup>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "invalid_request", message)
}

fn canonical_text(value: &str, max_bytes: usize, field: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max_bytes {
        return Err(invalid(format!("{field} is empty or too long")));
    }
    if trimmed.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
        || trimmed.contains('\r')
        || trimmed.contains('\n')
        || trimmed.contains('\t')
    {
        return Err(invalid(format!("{field} contains a control character")));
    }
    Ok(trimmed.to_string())
}

fn optional_search(value: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    canonical_text(trimmed, 128, "q").map(Some)
}

fn parse_offset_cursor(value: Option<&str>) -> Result<usize, AppError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    let Some(offset) = value.strip_prefix("o:") else {
        return Err(invalid("invalid cursor"));
    };
    offset
        .parse::<usize>()
        .map_err(|_| invalid("invalid cursor"))
}

fn ascii_search_key(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_uppercase() {
                byte + 32
            } else {
                *byte
            }
        })
        .collect()
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn multiplier_decimal(multiplier: &Multiplier, base: &str) -> Option<String> {
    let base = Decimal::from_str(base).ok()?;
    let multiplier = Decimal::from_str(&multiplier.to_string()).ok()?;
    if base.is_sign_negative() || multiplier.is_sign_negative() {
        return None;
    }
    Some((base * multiplier).normalize().to_string())
}

async fn model_rates(
    state: &AppState,
    model: &str,
    provider_type: &str,
    multiplier: &Multiplier,
) -> Vec<PublicRate> {
    let metadata_profile = state
        .model_registry_store
        .list_model_metadata_pricing_profiles(&[model.to_string()])
        .await
        .ok()
        .and_then(|profiles| profiles.get(model).cloned());
    let profile = {
        let runtime = state.monoize_runtime.read().await;
        select_pricing_profile(&runtime.pricing_profile_model_patterns, model)
            .map(str::to_string)
            .or(metadata_profile)
    };
    let Some(profile) = profile else {
        return Vec::new();
    };
    let Ok(rates) = state
        .billing_rate_store
        .list_matching_rates(&profile, Some(provider_type), model)
        .await
    else {
        return Vec::new();
    };
    rates
        .into_iter()
        .filter_map(|rate| public_rate(rate, multiplier))
        .collect()
}

fn public_rate(rate: DbBillingRateRecord, multiplier: &Multiplier) -> Option<PublicRate> {
    Some(PublicRate {
        usage_class: rate.usage_class,
        unit: rate.unit,
        display_rate_nano_usd: multiplier_decimal(multiplier, &rate.unit_price_nano_usd)?,
        context_tier: rate.context_tier,
        service_tier: rate.service_tier,
        modality: rate.modality,
        cache_ttl: rate.cache_ttl,
    })
}

fn rate_range(rates: &[PublicRate], usage_class: &str) -> Option<RateRange> {
    let mut values = rates
        .iter()
        .filter(|rate| rate.usage_class == usage_class)
        .filter_map(|rate| {
            Decimal::from_str(&rate.display_rate_nano_usd)
                .ok()
                .map(|v| (v, rate))
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    let (_, first) = values.first()?.clone();
    let (_, last) = values.last()?.clone();
    Some(RateRange {
        min: first.display_rate_nano_usd.clone(),
        max: last.display_rate_nano_usd.clone(),
        unit: first.unit.clone(),
    })
}

fn groups_by_id(
    state: &AppState,
) -> impl std::future::Future<Output = Result<HashMap<String, String>, String>> + '_ {
    async move {
        Ok(state
            .user_store
            .list_groups()
            .await?
            .into_iter()
            .map(|group| (group.id, group.name))
            .collect())
    }
}

fn provider_group_names(
    provider: &MonoizeProvider,
    groups: &HashMap<String, String>,
) -> Vec<String> {
    groups.get(&provider.group_id).cloned().into_iter().collect()
}

fn channels_for_model<'a>(provider: &'a MonoizeProvider, model: &str) -> Vec<&'a MonoizeChannel> {
    provider
        .channels
        .iter()
        .filter(|channel| {
            channel.enabled && channel.weight > 0 && channel.models.contains_key(model)
        })
        .collect()
}

fn capability_labels(capabilities: &ModelCapabilities) -> Vec<String> {
    let mut labels = Vec::new();
    if capabilities.supports_streaming {
        labels.push("streaming".to_string());
    }
    if capabilities.supports_tools {
        labels.push("tools".to_string());
    }
    if capabilities.supports_structured_output {
        labels.push("structured_output".to_string());
    }
    if capabilities.supports_reasoning_controls.supported {
        labels.push("reasoning".to_string());
    }
    if capabilities.supports_image_input.supported {
        labels.push("image_input".to_string());
    }
    if capabilities.supports_file_input.supported {
        labels.push("file_input".to_string());
    }
    if capabilities.supports_image_output.supported {
        labels.push("image_output".to_string());
    }
    labels
}

pub async fn list_marketplace(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<MarketplaceQuery>,
) -> AppResult<impl IntoResponse> {
    if !crate::public_api::admit(&headers) {
        return Ok(crate::public_api::rate_limited_response());
    }
    let offset = parse_offset_cursor(query.cursor.as_deref())?;
    let limit = query.limit.unwrap_or(24);
    if !(1..=50).contains(&limit) {
        return Err(invalid("limit must be between 1 and 50"));
    }
    let search = optional_search(query.q.as_deref())?;
    let search_key = search.as_deref().map(ascii_search_key);
    let group_filter = query
        .group
        .as_deref()
        .map(|value| canonical_text(value, 64, "group"))
        .transpose()?;
    let groups = groups_by_id(&state).await.map_err(|error| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
    })?;
    let providers = state
        .monoize_store
        .list_providers()
        .await
        .map_err(|error| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
        })?;
    let model_capabilities = state
        .model_registry_store
        .list_models()
        .await
        .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error))?
        .into_iter()
        .map(|record| {
            (
                record.logical_model,
                capability_labels(&record.capabilities),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut items: BTreeMap<(String, String), Vec<(String, String, Vec<PublicRate>)>> =
        BTreeMap::new();
    for provider in providers {
        let group_names = provider_group_names(&provider, &groups);
        for group_name in group_names {
            if group_filter
                .as_deref()
                .is_some_and(|filter| filter != group_name)
            {
                continue;
            }
            for channel in &provider.channels {
                if !channel.enabled || channel.weight <= 0 {
                    continue;
                }
                for (model, entry) in &channel.models {
                    if search_key.as_deref().is_some_and(|needle| {
                        !ascii_search_key(model)
                            .windows(needle.len())
                            .any(|window| window == needle)
                    }) {
                        continue;
                    }
                    let multiplier = entry.multiplier;
                    let rates =
                        model_rates(&state, model, channel.provider_type.as_str(), &multiplier)
                            .await;
                    if rates.is_empty() {
                        continue;
                    }
                    items
                        .entry((group_name.clone(), model.clone()))
                        .or_default()
                        .push((provider.name.clone(), channel.name.clone(), rates));
                }
            }
        }
    }
    let total_items = items.len();
    let mut output = Vec::new();
    for ((group_name, model), offers) in items.into_iter().skip(offset).take(limit as usize) {
        let rates = offers
            .iter()
            .flat_map(|offer| offer.2.clone())
            .collect::<Vec<_>>();
        let capabilities = model_capabilities.get(&model).cloned().unwrap_or_default();
        output.push(MarketplaceItem {
            public_group_name: group_name,
            model,
            capabilities,
            input_rate_range: rate_range(&rates, "input_uncached"),
            output_rate_range: rate_range(&rates, "output"),
            offer_count: offers.len(),
        });
    }
    let snapshot = snapshot(
        state
            .routing_config_revision
            .load(std::sync::atomic::Ordering::Acquire),
    );
    let response = MarketplaceResponse {
        generated_at: snapshot.generated_at,
        revision: snapshot.revision.to_string(),
        next_cursor: (offset + output.len() < total_items)
            .then(|| format!("o:{}", offset + output.len())),
        items: output,
    };
    let bytes = serde_json::to_vec(&response).map_err(|error| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            error.to_string(),
        )
    })?;
    Ok(crate::public_api::cacheable_json_response(
        &headers,
        bytes,
        "public, max-age=15, stale-while-revalidate=30",
    ))
}

pub async fn marketplace_offers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<MarketplaceQuery>,
) -> AppResult<impl IntoResponse> {
    if !crate::public_api::admit(&headers) {
        return Ok(crate::public_api::rate_limited_response());
    }
    let offset = parse_offset_cursor(query.cursor.as_deref())?;
    let group = query
        .group
        .as_deref()
        .ok_or_else(|| invalid("group is required"))
        .and_then(|value| canonical_text(value, 64, "group"))?;
    let model = query
        .model
        .as_deref()
        .ok_or_else(|| invalid("model is required"))
        .and_then(|value| canonical_text(value, 256, "model"))?;
    let limit = query.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(invalid("limit must be between 1 and 50"));
    }
    let groups = groups_by_id(&state).await.map_err(|error| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
    })?;
    let providers = state
        .monoize_store
        .list_providers()
        .await
        .map_err(|error| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
        })?;
    let mut offers = Vec::new();
    for provider in providers {
        if !provider_group_names(&provider, &groups)
            .iter()
            .any(|name| name == &group)
        {
            continue;
        }
        for channel in channels_for_model(&provider, &model) {
            let Some(entry) = channel.models.get(&model) else {
                continue;
            };
            let rates = model_rates(
                &state,
                &model,
                channel.provider_type.as_str(),
                &entry.multiplier,
            )
            .await;
            if rates.is_empty() {
                continue;
            }
            offers.push(MarketplaceOffer {
                public_provider_name: provider.name.clone(),
                public_channel_name: channel.name.clone(),
                api_type: channel.provider_type.as_str().to_string(),
                rates,
            });
        }
    }
    offers.sort_by(|left, right| {
        left.public_provider_name
            .as_bytes()
            .cmp(right.public_provider_name.as_bytes())
            .then_with(|| {
                left.public_channel_name
                    .as_bytes()
                    .cmp(right.public_channel_name.as_bytes())
            })
    });
    if offers.is_empty() {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "marketplace_model_not_found",
            "model not found",
        ));
    }
    let snapshot = snapshot(
        state
            .routing_config_revision
            .load(std::sync::atomic::Ordering::Acquire),
    );
    let total_offers = offers.len();
    let page_offers = offers
        .into_iter()
        .skip(offset)
        .take(limit as usize)
        .collect::<Vec<_>>();
    let response = OffersResponse {
        generated_at: snapshot.generated_at,
        revision: snapshot.revision.to_string(),
        public_group_name: group,
        model,
        next_cursor: (offset + page_offers.len() < total_offers)
            .then(|| format!("o:{}", offset + page_offers.len())),
        offers: page_offers,
    };
    let bytes = serde_json::to_vec(&response).map_err(|error| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            error.to_string(),
        )
    })?;
    Ok(crate::public_api::cacheable_json_response(
        &headers,
        bytes,
        "public, max-age=15, stale-while-revalidate=30",
    ))
}

pub async fn public_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    if !crate::public_api::admit(&headers) {
        return Ok(crate::public_api::rate_limited_response());
    }
    let groups = groups_by_id(&state).await.map_err(|error| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
    })?;
    let providers = state
        .monoize_store
        .list_providers()
        .await
        .map_err(|error| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
        })?;
    let since = Utc::now().timestamp_millis() - 86_400_000;
    let mut grouped: BTreeMap<String, Vec<PublicStatusProvider>> = BTreeMap::new();
    let mut data_complete = true;
    for provider in providers {
        let group_names = provider_group_names(&provider, &groups);
        let row = state.db_pool.read().query_one(state.db_pool.stmt("SELECT COUNT(*) AS total, SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS successes FROM request_logs WHERE provider_id = $1 AND created_at_unix_ms >= $2", vec![provider.id.clone().into(), since.into()])).await.map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error.to_string()))?;
        let total = row
            .as_ref()
            .and_then(|row| row.try_get::<i64>("", "total").ok())
            .unwrap_or(0);
        if total == 0 {
            data_complete = false;
        }
        let successes = row
            .as_ref()
            .and_then(|row| row.try_get::<Option<i64>>("", "successes").ok())
            .flatten()
            .unwrap_or(0);
        let rate = (total > 0)
            .then(|| ((successes.max(0) * 10_000 / total.max(1)).clamp(0, 10_000)) as u32);
        let state_name = match rate {
            None => "insufficient_data",
            Some(value) if value >= 9_900 => "operational",
            Some(value) if value >= 9_000 => "minor_degradation",
            _ => "major_degradation",
        };
        let entry = PublicStatusProvider {
            public_name: provider.name,
            state: state_name,
            success_rate_24h_basis_points: rate,
        };
        for group_name in group_names {
            grouped.entry(group_name).or_default().push(entry.clone());
        }
    }
    let output = grouped
        .into_iter()
        .map(|(name, mut providers)| {
            providers.sort_by(|left, right| {
                left.public_name
                    .as_bytes()
                    .cmp(right.public_name.as_bytes())
            });
            let insufficient = providers
                .iter()
                .filter(|provider| provider.state == "insufficient_data")
                .count();
            let state = if providers.is_empty() {
                "insufficient_data"
            } else if providers
                .iter()
                .any(|provider| provider.state == "major_degradation")
            {
                "major_degradation"
            } else if providers
                .iter()
                .any(|provider| provider.state == "minor_degradation")
            {
                "minor_degradation"
            } else if insufficient > 0 {
                "insufficient_data"
            } else {
                "operational"
            };
            PublicStatusGroup {
                public_name: name,
                state,
                insufficient_provider_count: insufficient,
                providers,
            }
        })
        .collect();
    let snapshot = snapshot(
        state
            .routing_config_revision
            .load(std::sync::atomic::Ordering::Acquire),
    );
    let response = PublicStatusResponse {
        generated_at: snapshot.generated_at.clone(),
        data_through: snapshot.generated_at,
        data_complete,
        groups: output,
    };
    let bytes = serde_json::to_vec(&response).map_err(|error| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            error.to_string(),
        )
    })?;
    Ok(crate::public_api::cacheable_json_response(
        &headers,
        bytes,
        "public, max-age=15",
    ))
}

#[cfg(test)]
mod tests {
    use super::{ascii_search_key, canonical_text};

    #[test]
    fn search_key_only_folds_ascii() {
        assert_eq!(ascii_search_key("GPT-4o/模型"), "gpt-4o/模型".as_bytes());
    }

    #[test]
    fn public_filter_rejects_controls_and_trims_unicode_space() {
        assert_eq!(
            canonical_text("  group-a  ", 64, "group").unwrap(),
            "group-a"
        );
        assert!(canonical_text("group\n-a", 64, "group").is_err());
        assert!(canonical_text("", 64, "group").is_err());
    }
}
