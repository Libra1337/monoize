use crate::app::AppState;
use crate::billing_rate_store::DbBillingRateRecord;
use crate::error::{AppError, AppResult};
use crate::exact_decimal::Multiplier;
use crate::model_registry::ModelCapabilities;
use crate::monoize_routing::{
    MonoizeChannel, MonoizeProvider, effective_model_multiplier, effective_pricing_profile,
};
use crate::public_name::canonicalize_public_name;
use crate::settings::normalize_pricing_model_key;
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

#[derive(Debug, Clone)]
struct PublicProviderNames {
    provider: String,
    channel: String,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "invalid_request", message)
}

fn marketplace_source_error(error: impl std::fmt::Display) -> AppError {
    tracing::error!(error = %error, "public Marketplace source failed");
    AppError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "marketplace_source_invalid",
        "public catalog is temporarily unavailable",
    )
}

fn status_source_error(error: impl std::fmt::Display) -> AppError {
    tracing::error!(error = %error, "public Status source failed");
    AppError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "status_source_invalid",
        "public status is temporarily unavailable",
    )
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
    upstream_model: &str,
    logical_model: &str,
    provider_type: &str,
    pricing_profile: &str,
    multiplier: &Multiplier,
) -> Vec<PublicRate> {
    let reasoning_suffix_map = {
        let runtime = state.monoize_runtime.read().await;
        runtime.reasoning_suffix_map.clone()
    };
    let upstream_model = normalize_pricing_model_key(upstream_model, &reasoning_suffix_map);
    let logical_model = normalize_pricing_model_key(logical_model, &reasoning_suffix_map);
    let Ok(upstream_rates) = state
        .billing_rate_store
        .list_matching_rates(pricing_profile, Some(provider_type), &upstream_model)
        .await
    else {
        return Vec::new();
    };
    let models_differ = logical_model != upstream_model;
    let logical_rates = if models_differ
        && !crate::dashboard_handlers::provider_dashboard_rate_matrix_is_complete(&upstream_rates)
    {
        let Ok(rates) = state
            .billing_rate_store
            .list_matching_rates(pricing_profile, Some(provider_type), &logical_model)
            .await
        else {
            return Vec::new();
        };
        rates
    } else {
        Vec::new()
    };
    let rates = select_complete_marketplace_rates(upstream_rates, logical_rates, models_differ);
    rates
        .into_iter()
        .filter_map(|rate| public_rate(rate, multiplier))
        .collect()
}

fn select_complete_marketplace_rates(
    upstream_rates: Vec<DbBillingRateRecord>,
    logical_rates: Vec<DbBillingRateRecord>,
    models_differ: bool,
) -> Vec<DbBillingRateRecord> {
    if crate::dashboard_handlers::provider_dashboard_rate_matrix_is_complete(&upstream_rates) {
        return upstream_rates;
    }
    if models_differ
        && crate::dashboard_handlers::provider_dashboard_rate_matrix_is_complete(&logical_rates)
    {
        return logical_rates;
    }
    Vec::new()
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
        let rows = state
            .db_pool
            .read()
            .query_all(state.db_pool.stmt(
                "SELECT id, public_name AS group_public_name FROM monoize_groups",
                vec![],
            ))
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|row| {
                let id = row.try_get("", "id").map_err(|error| error.to_string())?;
                let public_name = row
                    .try_get("", "group_public_name")
                    .map_err(|error| error.to_string())?;
                Ok((id, public_name))
            })
            .collect::<Result<HashMap<_, _>, String>>()?;
        Ok(rows)
    }
}

async fn public_provider_names_by_id(
    state: &AppState,
) -> Result<HashMap<String, PublicProviderNames>, String> {
    state
        .db_pool
        .read()
        .query_all(state.db_pool.stmt(
            "SELECT id, public_name, channel_public_name FROM monoize_providers",
            vec![],
        ))
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|row| {
            let id = row.try_get("", "id").map_err(|error| error.to_string())?;
            let provider = row
                .try_get("", "public_name")
                .map_err(|error| error.to_string())?;
            let channel = row
                .try_get("", "channel_public_name")
                .map_err(|error| error.to_string())?;
            Ok((id, PublicProviderNames { provider, channel }))
        })
        .collect()
}

fn public_names_for_provider<'a>(
    names: &'a HashMap<String, PublicProviderNames>,
    provider: &MonoizeProvider,
) -> Option<&'a PublicProviderNames> {
    names.get(&provider.id)
}

fn provider_group_names(
    provider: &MonoizeProvider,
    groups: &HashMap<String, String>,
) -> Vec<String> {
    groups
        .get(&provider.group_id)
        .cloned()
        .into_iter()
        .collect()
}

fn channels_for_model<'a>(provider: &'a MonoizeProvider, model: &str) -> Vec<&'a MonoizeChannel> {
    std::iter::once(&provider.channel)
        .filter(|channel| channel.enabled && channel.models.contains_key(model))
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

fn marketplace_group_filter(raw: &str) -> AppResult<Vec<u8>> {
    canonicalize_public_name(raw)
        .map(|name| name.key)
        .map_err(invalid)
}

fn exact_marketplace_model(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim_matches(char::is_whitespace);
    if trimmed != raw {
        return Err(invalid(
            "model must not contain leading or trailing whitespace",
        ));
    }
    canonical_text(raw, 256, "model")
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
        .map(marketplace_group_filter)
        .transpose()?;
    let groups = groups_by_id(&state)
        .await
        .map_err(marketplace_source_error)?;
    let providers = state
        .monoize_store
        .list_providers()
        .await
        .map_err(marketplace_source_error)?;
    let public_provider_names = public_provider_names_by_id(&state)
        .await
        .map_err(marketplace_source_error)?;
    let model_capabilities = state
        .model_registry_store
        .list_models()
        .await
        .map_err(marketplace_source_error)?
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
        let public_names = public_names_for_provider(&public_provider_names, &provider)
            .ok_or_else(|| marketplace_source_error("Provider public name missing"))?;
        let group_names = provider_group_names(&provider, &groups);
        for group_name in group_names {
            if group_filter
                .as_deref()
                .is_some_and(|filter| filter != group_name.as_bytes())
            {
                continue;
            }
            for channel in std::iter::once(&provider.channel) {
                if !channel.enabled {
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
                    let Some(profile) = effective_pricing_profile(&provider, entry) else {
                        continue;
                    };
                    let multiplier = effective_model_multiplier(&provider, entry);
                    let upstream_model = entry.redirect.as_deref().unwrap_or(model);
                    let provider_type = crate::monoize_routing::resolve_effective_api_type(
                        &provider.api_type_overrides,
                        channel.provider_type,
                        model,
                    );
                    let rates = model_rates(
                        &state,
                        upstream_model,
                        model,
                        provider_type.as_str(),
                        profile,
                        &multiplier,
                    )
                    .await;
                    if rates.is_empty() {
                        continue;
                    }
                    items
                        .entry((group_name.clone(), model.clone()))
                        .or_default()
                        .push((
                            public_names.provider.clone(),
                            public_names.channel.clone(),
                            rates,
                        ));
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
    let bytes = serde_json::to_vec(&response).map_err(marketplace_source_error)?;
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
        .and_then(|value| canonicalize_public_name(value).map_err(invalid))?;
    let model = query
        .model
        .as_deref()
        .ok_or_else(|| invalid("model is required"))
        .and_then(exact_marketplace_model)?;
    let limit = query.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(invalid("limit must be between 1 and 50"));
    }
    let groups = groups_by_id(&state)
        .await
        .map_err(marketplace_source_error)?;
    let providers = state
        .monoize_store
        .list_providers()
        .await
        .map_err(marketplace_source_error)?;
    let public_provider_names = public_provider_names_by_id(&state)
        .await
        .map_err(marketplace_source_error)?;
    let mut offers = Vec::new();
    for provider in providers {
        let public_names = public_names_for_provider(&public_provider_names, &provider)
            .ok_or_else(|| marketplace_source_error("Provider public name missing"))?;
        if !provider_group_names(&provider, &groups)
            .iter()
            .any(|name| name.as_bytes() == group.key.as_slice())
        {
            continue;
        }
        for channel in channels_for_model(&provider, &model) {
            let Some(entry) = channel.models.get(&model) else {
                continue;
            };
            let Some(profile) = effective_pricing_profile(&provider, entry) else {
                continue;
            };
            let multiplier = effective_model_multiplier(&provider, entry);
            let upstream_model = entry.redirect.as_deref().unwrap_or(&model);
            let provider_type = crate::monoize_routing::resolve_effective_api_type(
                &provider.api_type_overrides,
                channel.provider_type,
                &model,
            );
            let rates = model_rates(
                &state,
                upstream_model,
                &model,
                provider_type.as_str(),
                profile,
                &multiplier,
            )
            .await;
            if rates.is_empty() {
                continue;
            }
            offers.push(MarketplaceOffer {
                public_provider_name: public_names.provider.clone(),
                public_channel_name: public_names.channel.clone(),
                api_type: provider_type.as_str().to_string(),
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
        public_group_name: group.value,
        model,
        next_cursor: (offset + page_offers.len() < total_offers)
            .then(|| format!("o:{}", offset + page_offers.len())),
        offers: page_offers,
    };
    let bytes = serde_json::to_vec(&response).map_err(marketplace_source_error)?;
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
    let groups = groups_by_id(&state).await.map_err(status_source_error)?;
    let providers = state
        .monoize_store
        .list_providers()
        .await
        .map_err(status_source_error)?;
    let public_provider_names = public_provider_names_by_id(&state)
        .await
        .map_err(status_source_error)?;
    let since = Utc::now().timestamp_millis() - 86_400_000;
    let mut grouped: BTreeMap<String, Vec<PublicStatusProvider>> = BTreeMap::new();
    let mut data_complete = true;
    for provider in providers {
        let public_names = public_names_for_provider(&public_provider_names, &provider)
            .ok_or_else(|| status_source_error("Provider public name missing"))?;
        let group_names = provider_group_names(&provider, &groups);
        let row = state.db_pool.read().query_one(state.db_pool.stmt("SELECT COUNT(*) AS total, SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS successes FROM request_logs WHERE provider_id = $1 AND created_at_unix_ms >= $2", vec![provider.id.clone().into(), since.into()])).await.map_err(status_source_error)?;
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
            public_name: public_names.provider.clone(),
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
    let bytes = serde_json::to_vec(&response).map_err(status_source_error)?;
    Ok(crate::public_api::cacheable_json_response(
        &headers,
        bytes,
        "public, max-age=15",
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        ascii_search_key, canonical_text, exact_marketplace_model, marketplace_group_filter,
        public_status, select_complete_marketplace_rates,
    };
    use crate::app::{AppState, RuntimeConfig, load_state_with_runtime};
    use crate::billing_rate_store::DbBillingRateRecord;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;
    use sea_orm::{ConnectionTrait, Value as SeaValue};

    async fn make_state() -> AppState {
        load_state_with_runtime(RuntimeConfig {
            listen: "127.0.0.1:0".to_string(),
            metrics_path: "/metrics".to_string(),
            database_dsn: "sqlite::memory:".to_string(),
            request_log_spool_dir: None,
            node: crate::node_config::NodeSettings::primary_default(),
        })
        .await
        .expect("state loads")
    }

    fn token_rate(id: &str, usage_class: &str) -> DbBillingRateRecord {
        DbBillingRateRecord {
            id: id.to_string(),
            source: "test".to_string(),
            pricing_profile: "profile".to_string(),
            model_pattern: None,
            provider_type: None,
            rate_kind: "token".to_string(),
            usage_class: usage_class.to_string(),
            unit: "token".to_string(),
            unit_price_nano_usd: "1".to_string(),
            context_tier: None,
            service_tier: None,
            modality: None,
            cache_ttl: None,
            match_json: serde_json::json!({}),
            priority: 0,
            enabled: true,
            raw_json: serde_json::json!({}),
            updated_at: chrono::Utc::now(),
        }
    }

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

    #[test]
    fn marketplace_group_filter_uses_nfc_public_name_key() {
        assert_eq!(
            marketplace_group_filter("  Cafe\u{301}  ").unwrap(),
            "Caf\u{e9}".as_bytes()
        );
        assert!(marketplace_group_filter("bad\nname").is_err());
    }

    #[test]
    fn marketplace_offer_model_rejects_outer_whitespace() {
        assert_eq!(exact_marketplace_model("gpt-4o").unwrap(), "gpt-4o");
        assert!(exact_marketplace_model(" gpt-4o").is_err());
        assert!(exact_marketplace_model("gpt-4o\u{3000}").is_err());
    }

    #[test]
    fn marketplace_rates_fall_back_when_upstream_matrix_is_incomplete() {
        let upstream = vec![token_rate("upstream-input", "input_uncached")];
        let logical = vec![
            token_rate("logical-input", "input_uncached"),
            token_rate("logical-output", "output"),
        ];

        let selected = select_complete_marketplace_rates(upstream, logical, true);

        assert_eq!(selected.len(), 2);
        assert!(selected.iter().all(|rate| rate.id.starts_with("logical-")));
    }

    #[tokio::test]
    async fn public_status_uses_only_persisted_public_names() {
        let state = make_state().await;
        let group_id = state
            .user_store
            .default_group_id()
            .await
            .expect("default group exists");
        let provider = state
            .monoize_store
            .create_provider(
                serde_json::from_value(serde_json::json!({
                    "name": "Internal Provider",
                    "confirm_public_exposure": true,
                    "group_id": group_id,
                    "channel": {
                        "name": "Internal Channel",
                        "provider_type": "responses",
                        "base_url": "https://example.invalid",
                        "api_key": "secret",
                        "models": {}
                    }
                }))
                .expect("provider payload deserializes"),
            )
            .await
            .expect("provider creates");

        state
            .db_pool
            .write()
            .await
            .execute(state.db_pool.stmt(
                "UPDATE monoize_groups SET public_name = $1, public_name_key = $2 WHERE id = $3",
                vec![
                    "Public Group".into(),
                    SeaValue::Bytes(Some(Box::new(b"Public Group".to_vec()))),
                    provider.group_id.clone().into(),
                ],
            ))
            .await
            .expect("group public name updates");
        state
            .db_pool
            .write()
            .await
            .execute(state.db_pool.stmt(
                "UPDATE monoize_providers SET public_name = $1, public_name_key = $2, channel_public_name = $3, channel_public_name_key = $4 WHERE id = $5",
                vec![
                    "Public Provider".into(),
                    SeaValue::Bytes(Some(Box::new(b"Public Provider".to_vec()))),
                    "Public Channel".into(),
                    SeaValue::Bytes(Some(Box::new(b"Public Channel".to_vec()))),
                    provider.id.into(),
                ],
            ))
            .await
            .expect("Provider public names update");

        let response = public_status(State(state), HeaderMap::new())
            .await
            .expect("status succeeds")
            .into_response();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body reads")
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).expect("body is JSON");

        assert_eq!(body["groups"][0]["public_name"], "Public Group");
        assert_eq!(
            body["groups"][0]["providers"][0]["public_name"],
            "Public Provider"
        );
        assert!(!body.to_string().contains("Internal Provider"));
        assert!(!body.to_string().contains("Internal Channel"));
    }
}
