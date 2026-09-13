use crate::app::AppState;
use crate::dashboard_handlers::session_helpers::{get_current_user, require_admin};
use crate::error::{AppError, AppResult};
use crate::exact_decimal::Multiplier;
use crate::transforms::TransformRuleConfig;
use crate::users::{
    canonicalize_channel_bindings, format_nano_to_usd, parse_nano_usd, AnalyticsBucketing,
    ApiKeyChannelBinding, CreateApiKeyInput, CreateApiKeyWithLimitError, ModelRedirectRule,
    RequestCaptureMode, UpdateApiKeyInput,
};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct ApiKeyAnalyticsQuery {
    #[serde(default = "default_api_key_analytics_range")]
    pub range: String,
}

fn default_api_key_analytics_range() -> String {
    "24h".to_string()
}

#[derive(Debug, Serialize)]
pub struct ApiKeyAnalyticsTrendPoint {
    pub label: String,
    pub input_tokens: String,
    pub cache_read_tokens: String,
    pub output_tokens: String,
    pub total_tokens: String,
    pub request_count: i64,
    pub consumed_coin_nano: String,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyAnalyticsModelRow {
    pub model: String,
    pub input_tokens: String,
    pub cache_read_tokens: String,
    pub output_tokens: String,
    pub total_tokens: String,
    pub request_count: i64,
    pub consumed_coin_nano: String,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyAnalyticsResponse {
    pub key_id: String,
    pub key_name: String,
    pub range: String,
    pub time_from: String,
    pub time_to: String,
    pub total_tokens: String,
    pub total_input_tokens: String,
    pub total_cache_read_tokens: String,
    pub total_output_tokens: String,
    pub request_count: i64,
    pub consumed_coin_nano: String,
    pub balance_mode: &'static str,
    pub independent_balance_nano: Option<String>,
    pub trend: Vec<ApiKeyAnalyticsTrendPoint>,
    pub models: Vec<ApiKeyAnalyticsModelRow>,
}

pub(super) fn nano_balance_fields(nano_str: &str) -> Result<(String, String), String> {
    let nano = parse_nano_usd(nano_str)?;
    Ok((nano_str.to_string(), format_nano_to_usd(nano)))
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub expires_in_days: Option<i64>,
    #[serde(default)]
    pub sub_account_enabled: bool,
    #[serde(default)]
    pub sub_account_balance_nano_usd: Option<String>,
    #[serde(default)]
    pub model_limits_enabled: bool,
    #[serde(default)]
    pub model_limits: Vec<String>,
    #[serde(default)]
    pub ip_whitelist: Vec<String>,
    #[serde(default)]
    pub group_ids: Vec<String>,
    #[serde(default)]
    pub channel_bindings: Vec<ApiKeyChannelBinding>,
    #[serde(default)]
    pub max_multiplier: Option<Multiplier>,
    #[serde(default)]
    pub transforms: Vec<TransformRuleConfig>,
    #[serde(default)]
    pub model_redirects: Vec<ModelRedirectRule>,
    #[serde(default = "default_true")]
    pub reasoning_envelope_enabled: bool,
    #[serde(default)]
    pub request_capture_mode: RequestCaptureMode,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub key: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub enabled: bool,
    pub sub_account_enabled: bool,
    pub sub_account_balance_nano_usd: String,
    pub sub_account_balance_usd: String,
    pub model_limits_enabled: bool,
    pub model_limits: Vec<String>,
    pub ip_whitelist: Vec<String>,
    pub group_ids: Vec<String>,
    pub channel_bindings: Vec<ApiKeyChannelBinding>,
    pub max_multiplier: Option<Multiplier>,
    pub transforms: Vec<TransformRuleConfig>,
    pub model_redirects: Vec<ModelRedirectRule>,
    pub reasoning_envelope_enabled: bool,
    pub request_capture_mode: RequestCaptureMode,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyCreatedResponse {
    pub id: String,
    pub name: String,
    pub key: String,
    pub key_prefix: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub sub_account_enabled: bool,
    pub sub_account_balance_nano_usd: String,
    pub sub_account_balance_usd: String,
    pub model_limits_enabled: bool,
    pub model_limits: Vec<String>,
    pub ip_whitelist: Vec<String>,
    pub group_ids: Vec<String>,
    pub channel_bindings: Vec<ApiKeyChannelBinding>,
    pub max_multiplier: Option<Multiplier>,
    pub transforms: Vec<TransformRuleConfig>,
    pub model_redirects: Vec<ModelRedirectRule>,
    pub reasoning_envelope_enabled: bool,
    pub request_capture_mode: RequestCaptureMode,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub sub_account_enabled: Option<bool>,
    pub sub_account_balance_nano_usd: Option<String>,
    pub model_limits_enabled: Option<bool>,
    pub model_limits: Option<Vec<String>>,
    pub ip_whitelist: Option<Vec<String>>,
    pub group_ids: Option<Vec<String>>,
    pub channel_bindings: Option<Vec<ApiKeyChannelBinding>>,
    pub max_multiplier: Option<Multiplier>,
    pub transforms: Option<Vec<TransformRuleConfig>>,
    pub model_redirects: Option<Vec<ModelRedirectRule>>,
    pub reasoning_envelope_enabled: Option<bool>,
    pub request_capture_mode: Option<RequestCaptureMode>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyChannelOptionResponse {
    pub channel_id: String,
    pub channel_name: String,
    pub provider_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyChannelConflictResponse {
    pub group_id: String,
    pub group_name: String,
    pub model: String,
    pub options: Vec<ApiKeyChannelOptionResponse>,
}

/// Groups and models for which the key owner would face more than one eligible Channel.
///
/// Only Groups in `account_class` are considered. A key can never route into the other
/// class, so a conflict there is not one the owner can resolve — surfacing it would demand a
/// Channel choice for a Group the owner cannot reach and block key creation outright.
async fn current_channel_conflicts(
    state: &AppState,
    account_class: crate::users::AccountClass,
) -> Result<Vec<ApiKeyChannelConflictResponse>, String> {
    let group_names = state
        .user_store
        .list_groups()
        .await?
        .into_iter()
        .filter(|group| group.account_class == account_class)
        .map(|group| (group.id, group.name))
        .collect::<std::collections::BTreeMap<_, _>>();
    let providers = state.monoize_store.list_providers().await?;
    let mut by_scope =
        std::collections::BTreeMap::<(String, String), Vec<ApiKeyChannelOptionResponse>>::new();
    for provider in providers {
        if !provider.enabled || !provider.channel.enabled {
            continue;
        }
        if !group_names.contains_key(&provider.group_id) {
            continue;
        }
        let unpriced_models = super::providers::provider_pricing_warnings(state, &provider)
            .await
            .map_err(|error| error.message)?
            .into_iter()
            .map(|warning| warning.logical_model)
            .collect::<std::collections::BTreeSet<_>>();
        for model in provider.channel.models.keys() {
            if unpriced_models.contains(model) {
                continue;
            }
            by_scope
                .entry((provider.group_id.clone(), model.clone()))
                .or_default()
                .push(ApiKeyChannelOptionResponse {
                    channel_id: provider.channel.id.clone(),
                    channel_name: provider.channel.name.clone(),
                    provider_name: provider.name.clone(),
                });
        }
    }
    Ok(by_scope
        .into_iter()
        .filter(|(_, options)| options.len() > 1)
        .map(
            |((group_id, model), options)| ApiKeyChannelConflictResponse {
                group_name: group_names
                    .get(&group_id)
                    .cloned()
                    .unwrap_or_else(|| group_id.clone()),
                group_id,
                model,
                options,
            },
        )
        .collect())
}

async fn validate_channel_bindings_for_scope(
    state: &AppState,
    account_class: crate::users::AccountClass,
    group_ids: &[String],
    model_limits_enabled: bool,
    model_limits: &[String],
    bindings: &[ApiKeyChannelBinding],
) -> Result<(), String> {
    let bindings = canonicalize_channel_bindings(bindings)?;
    let conflicts = current_channel_conflicts(state, account_class).await?;
    let in_scope = |conflict: &&ApiKeyChannelConflictResponse| {
        (group_ids.is_empty() || group_ids.iter().any(|id| id == &conflict.group_id))
            && (!model_limits_enabled
                || model_limits.is_empty()
                || model_limits.iter().any(|model| model == &conflict.model))
    };
    let required = conflicts.iter().filter(in_scope).collect::<Vec<_>>();
    if bindings.len() != required.len() {
        return Err("select one Channel for every ambiguous Group and model".to_string());
    }
    for conflict in required {
        let Some(binding) = bindings.iter().find(|binding| {
            binding.group_id == conflict.group_id && binding.model == conflict.model
        }) else {
            return Err(format!(
                "Channel selection required for Group {} and model {}",
                conflict.group_name, conflict.model
            ));
        };
        if !conflict
            .options
            .iter()
            .any(|option| option.channel_id == binding.channel_id)
        {
            return Err(format!(
                "selected Channel is unavailable for Group {} and model {}",
                conflict.group_name, conflict.model
            ));
        }
    }
    Ok(())
}

pub async fn list_api_key_channel_conflicts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let conflicts = current_channel_conflicts(&state, user.account_class)
        .await
        .map_err(|error| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
        })?;
    Ok(Json(conflicts))
}

#[derive(Debug, Deserialize)]
pub struct BatchDeleteApiKeysRequest {
    pub ids: Vec<String>,
}

pub async fn list_my_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;

    let user_store = &state.user_store;

    let keys = user_store
        .list_user_api_keys(&user.id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    let responses = keys
        .into_iter()
        .map(|k| {
            let (nano, usd) = nano_balance_fields(&k.sub_account_balance_nano)?;
            Ok(ApiKeyResponse {
                id: k.id,
                name: k.name,
                key_prefix: k.key_prefix,
                key: k.key,
                created_at: k.created_at.to_rfc3339(),
                expires_at: k.expires_at.map(|d| d.to_rfc3339()),
                last_used_at: k.last_used_at.map(|d| d.to_rfc3339()),
                enabled: k.enabled,
                sub_account_enabled: k.sub_account_enabled,
                sub_account_balance_nano_usd: nano,
                sub_account_balance_usd: usd,
                model_limits_enabled: k.model_limits_enabled,
                model_limits: k.model_limits,
                ip_whitelist: k.ip_whitelist,
                group_ids: k.group_ids,
                channel_bindings: k.channel_bindings,
                max_multiplier: k.max_multiplier,
                transforms: k.transforms,
                model_redirects: k.model_redirects,
                reasoning_envelope_enabled: k.reasoning_envelope_enabled,
                request_capture_mode: k.request_capture_mode,
            })
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    Ok(Json(responses))
}

pub async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateApiKeyRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;

    let user_store = &state.user_store;
    let settings_store = &state.settings_store;

    let max_per_user = settings_store
        .get_api_key_max_per_user()
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    validate_channel_bindings_for_scope(
        &state,
        user.account_class,
        &body.group_ids,
        body.model_limits_enabled,
        &body.model_limits,
        &body.channel_bindings,
    )
    .await
    .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, "invalid_request", error))?;

    let input = CreateApiKeyInput {
        name: body.name,
        expires_in_days: body.expires_in_days,
        sub_account_enabled: body.sub_account_enabled,
        sub_account_balance_nano_usd: body.sub_account_balance_nano_usd,
        model_limits_enabled: body.model_limits_enabled,
        model_limits: body.model_limits,
        ip_whitelist: body.ip_whitelist,
        group_ids: body.group_ids,
        channel_bindings: body.channel_bindings,
        max_multiplier: body.max_multiplier,
        transforms: body.transforms,
        model_redirects: body.model_redirects,
        reasoning_envelope_enabled: body.reasoning_envelope_enabled,
        request_capture_mode: body.request_capture_mode,
    };

    let is_admin = user.role.can_manage_system();

    let (api_key, key) = user_store
        .create_api_key_extended_with_limit(&user.id, input, is_admin, max_per_user)
        .await
        .map_err(|error| match error {
            CreateApiKeyWithLimitError::LimitReached { limit } => AppError::new(
                StatusCode::FORBIDDEN,
                "max_api_keys_reached",
                format!("maximum of {limit} API keys allowed per user"),
            ),
            CreateApiKeyWithLimitError::InvalidRequest(error) => {
                AppError::new(StatusCode::BAD_REQUEST, "invalid_request", error)
            }
        })?;

    let (nano, usd) = nano_balance_fields(&api_key.sub_account_balance_nano)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreatedResponse {
            id: api_key.id,
            name: api_key.name,
            key,
            key_prefix: api_key.key_prefix,
            created_at: api_key.created_at.to_rfc3339(),
            expires_at: api_key.expires_at.map(|d| d.to_rfc3339()),
            sub_account_enabled: api_key.sub_account_enabled,
            sub_account_balance_nano_usd: nano,
            sub_account_balance_usd: usd,
            model_limits_enabled: api_key.model_limits_enabled,
            model_limits: api_key.model_limits,
            ip_whitelist: api_key.ip_whitelist,
            group_ids: api_key.group_ids,
            channel_bindings: api_key.channel_bindings,
            max_multiplier: api_key.max_multiplier,
            transforms: api_key.transforms,
            model_redirects: api_key.model_redirects,
            reasoning_envelope_enabled: api_key.reasoning_envelope_enabled,
            request_capture_mode: api_key.request_capture_mode,
        }),
    ))
}

pub async fn delete_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;

    let user_store = &state.user_store;

    let api_key = user_store
        .get_api_key_for_user(&key_id, &user.id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    api_key
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "API key not found"))?;

    user_store
        .delete_api_key(&key_id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    Ok(Json(json!({ "success": true })))
}

pub async fn get_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;

    let user_store = &state.user_store;

    let api_key = user_store
        .get_api_key_for_user(&key_id, &user.id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    let api_key = api_key
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "API key not found"))?;

    Ok(Json({
        let (nano, usd) = nano_balance_fields(&api_key.sub_account_balance_nano)
            .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
        ApiKeyResponse {
            id: api_key.id,
            name: api_key.name,
            key_prefix: api_key.key_prefix,
            key: api_key.key,
            created_at: api_key.created_at.to_rfc3339(),
            expires_at: api_key.expires_at.map(|d| d.to_rfc3339()),
            last_used_at: api_key.last_used_at.map(|d| d.to_rfc3339()),
            enabled: api_key.enabled,
            sub_account_enabled: api_key.sub_account_enabled,
            sub_account_balance_nano_usd: nano,
            sub_account_balance_usd: usd,
            model_limits_enabled: api_key.model_limits_enabled,
            model_limits: api_key.model_limits,
            ip_whitelist: api_key.ip_whitelist,
            group_ids: api_key.group_ids,
            channel_bindings: api_key.channel_bindings,
            max_multiplier: api_key.max_multiplier,
            transforms: api_key.transforms,
            model_redirects: api_key.model_redirects,
            reasoning_envelope_enabled: api_key.reasoning_envelope_enabled,
            request_capture_mode: api_key.request_capture_mode,
        }
    }))
}

/// TM-AN5a bucket alignment. Each helper truncates a UTC instant down to the start
/// of its bucket unit so a bucket label names the interval the bucket covers.
fn align_down_to_hour(value: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    use chrono::Timelike;
    value
        .with_minute(0)
        .and_then(|value| value.with_second(0))
        .and_then(|value| value.with_nanosecond(0))
        .unwrap_or(value)
}

fn align_down_to_day(value: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    value
        .date_naive()
        .and_time(chrono::NaiveTime::MIN)
        .and_utc()
}

fn align_down_to_month(value: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    use chrono::Datelike;
    let date = value.date_naive();
    date.with_day(1)
        .unwrap_or(date)
        .and_time(chrono::NaiveTime::MIN)
        .and_utc()
}

/// The bucket unit of one API Key analytics range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalyticsBucketUnit {
    Hour,
    Day,
    Month,
}

impl AnalyticsBucketUnit {
    fn bucketing(self) -> AnalyticsBucketing {
        match self {
            // Hours and days have a constant length, so the window is an exact multiple of
            // the unit and equal-duration buckets land on unit boundaries.
            Self::Hour | Self::Day => AnalyticsBucketing::EqualIntervals,
            Self::Month => AnalyticsBucketing::CalendarMonths,
        }
    }

    fn advance(
        self,
        start: chrono::DateTime<chrono::Utc>,
        steps: i64,
    ) -> chrono::DateTime<chrono::Utc> {
        match self {
            Self::Hour => start + chrono::Duration::hours(steps),
            Self::Day => start + chrono::Duration::days(steps),
            Self::Month => add_months(start, steps),
        }
    }
}

/// One resolved analytics range. `time_to` is the exclusive end of the last bucket, aligned up
/// to the unit so every bucket covers exactly the interval its label names.
struct AnalyticsBucketPlan {
    time_from: chrono::DateTime<chrono::Utc>,
    time_to: chrono::DateTime<chrono::Utc>,
    bucket_count: i64,
    unit: AnalyticsBucketUnit,
    label_format: &'static str,
}

#[cfg(test)]
impl AnalyticsBucketPlan {
    fn uses_calendar_months(&self) -> bool {
        self.unit.bucketing() == AnalyticsBucketing::CalendarMonths
    }
}

fn add_months(value: chrono::DateTime<chrono::Utc>, months: i64) -> chrono::DateTime<chrono::Utc> {
    use chrono::Datelike;
    let index = i64::from(value.year()) * 12 + i64::from(value.month()) - 1 + months;
    let year = index.div_euclid(12);
    let month = index.rem_euclid(12) + 1;
    i32::try_from(year)
        .ok()
        .and_then(|year| chrono::NaiveDate::from_ymd_opt(year, month as u32, 1))
        .map(|date| date.and_time(chrono::NaiveTime::MIN).and_utc())
        .unwrap_or(value)
}

fn months_between(from: chrono::DateTime<chrono::Utc>, to: chrono::DateTime<chrono::Utc>) -> i64 {
    use chrono::Datelike;
    (i64::from(to.year()) * 12 + i64::from(to.month()))
        - (i64::from(from.year()) * 12 + i64::from(from.month()))
}

/// Resolves one range name into an exact bucket plan. `first_event` is read only for the
/// `all` range, so the other ranges cost no extra query.
async fn analytics_bucket_plan<F, Fut>(
    range: &str,
    now: chrono::DateTime<chrono::Utc>,
    first_event: F,
) -> AppResult<AnalyticsBucketPlan>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<chrono::DateTime<chrono::Utc>, String>>,
{
    let next_hour = align_down_to_hour(now) + chrono::Duration::hours(1);
    let next_day = align_down_to_day(now) + chrono::Duration::days(1);
    match range {
        "24h" => Ok(AnalyticsBucketPlan {
            time_from: next_hour - chrono::Duration::hours(24),
            time_to: next_hour,
            bucket_count: 24,
            unit: AnalyticsBucketUnit::Hour,
            label_format: "%m-%d %H:00",
        }),
        "7d" | "30d" => {
            let days = if range == "7d" { 7 } else { 30 };
            Ok(AnalyticsBucketPlan {
                time_from: next_day - chrono::Duration::days(days),
                time_to: next_day,
                bucket_count: days,
                unit: AnalyticsBucketUnit::Day,
                label_format: "%m-%d",
            })
        }
        "all" => {
            let first = first_event().await.map_err(|error| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
            })?;
            let first = first.min(now);
            let retained_days = (now - align_down_to_day(first)).num_days().max(0);
            if retained_days > 90 {
                let time_from = align_down_to_month(first);
                let time_to = add_months(align_down_to_month(now), 1);
                let bucket_count = months_between(time_from, time_to).clamp(1, 120);
                Ok(AnalyticsBucketPlan {
                    time_from,
                    // A clamp shortens the window, so the end follows the kept bucket count.
                    time_to: add_months(time_from, bucket_count).min(time_to),
                    bucket_count,
                    unit: AnalyticsBucketUnit::Month,
                    label_format: "%Y-%m",
                })
            } else {
                let bucket_count = (retained_days + 1).clamp(1, 90);
                Ok(AnalyticsBucketPlan {
                    time_from: next_day - chrono::Duration::days(bucket_count),
                    time_to: next_day,
                    bucket_count,
                    unit: AnalyticsBucketUnit::Day,
                    label_format: "%m-%d",
                })
            }
        }
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "range must equal 24h, 7d, 30d, or all",
        )
        .with_param("range")),
    }
}

pub async fn get_api_key_analytics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Query(query): Query<ApiKeyAnalyticsQuery>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let api_key = state
        .user_store
        .get_api_key_by_id(&key_id)
        .await
        .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error))?
        .filter(|key| key.user_id == user.id || user.role.can_manage_users())
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "API key not found"))?;

    let now = chrono::Utc::now();
    let plan = analytics_bucket_plan(&query.range, now, || async {
        let first_ms = state
            .user_store
            .get_api_key_analytics_start(&api_key.user_id, &api_key.id)
            .await?;
        Ok(first_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
            .unwrap_or(api_key.created_at))
    })
    .await?;
    let AnalyticsBucketPlan {
        time_from,
        time_to,
        bucket_count,
        unit,
        label_format,
    } = plan;
    let today_start = now.date_naive().and_time(chrono::NaiveTime::MIN).and_utc();
    let raw = state
        .user_store
        .get_dashboard_analytics_bucketed(
            Some(&api_key.user_id),
            Some(&api_key.id),
            None,
            None,
            &time_from.to_rfc3339(),
            &time_to.to_rfc3339(),
            &today_start.to_rfc3339(),
            bucket_count,
            unit.bucketing(),
        )
        .await
        .map_err(|error| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
        })?;

    let mut trend = (0..bucket_count)
        .map(|index| {
            // TM-AN5a: the label names the exact interval the bucket aggregates, so it is
            // derived by advancing whole units rather than by dividing the window.
            let label = unit
                .advance(time_from, index)
                .format(label_format)
                .to_string();
            ApiKeyAnalyticsTrendPoint {
                label,
                input_tokens: "0".to_string(),
                cache_read_tokens: "0".to_string(),
                output_tokens: "0".to_string(),
                total_tokens: "0".to_string(),
                request_count: 0,
                consumed_coin_nano: "0".to_string(),
            }
        })
        .collect::<Vec<_>>();
    let mut models = BTreeMap::<String, (i128, i128, i128, i64, i128)>::new();
    for row in &raw.model_buckets {
        let index = row.bucket_idx.clamp(0, bucket_count - 1) as usize;
        let point = &mut trend[index];
        let input = point.input_tokens.parse::<i128>().unwrap_or(0) + row.input_tokens;
        let cache = point.cache_read_tokens.parse::<i128>().unwrap_or(0) + row.cache_read_tokens;
        let output = point.output_tokens.parse::<i128>().unwrap_or(0) + row.output_tokens;
        let cost = point.consumed_coin_nano.parse::<i128>().unwrap_or(0) + row.cost_nano;
        point.input_tokens = input.to_string();
        point.cache_read_tokens = cache.to_string();
        point.output_tokens = output.to_string();
        point.total_tokens = input
            .checked_add(output)
            .ok_or_else(|| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "API key analytics token aggregate overflow",
                )
            })?
            .to_string();
        point.request_count = point
            .request_count
            .checked_add(row.call_count)
            .ok_or_else(|| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "API key analytics request aggregate overflow",
                )
            })?;
        point.consumed_coin_nano = cost.to_string();

        let entry = models.entry(row.model.clone()).or_insert((0, 0, 0, 0, 0));
        entry.0 = entry.0.checked_add(row.input_tokens).ok_or_else(|| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "API key analytics input aggregate overflow",
            )
        })?;
        entry.1 = entry.1.checked_add(row.cache_read_tokens).ok_or_else(|| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "API key analytics cache aggregate overflow",
            )
        })?;
        entry.2 = entry.2.checked_add(row.output_tokens).ok_or_else(|| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "API key analytics output aggregate overflow",
            )
        })?;
        entry.3 = entry.3.checked_add(row.call_count).ok_or_else(|| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "API key analytics call aggregate overflow",
            )
        })?;
        entry.4 = entry.4.checked_add(row.cost_nano).ok_or_else(|| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "API key analytics cost aggregate overflow",
            )
        })?;
    }
    let mut model_rows = models
        .into_iter()
        .map(
            |(model, (input, cache, output, calls, cost))| ApiKeyAnalyticsModelRow {
                model,
                input_tokens: input.to_string(),
                cache_read_tokens: cache.to_string(),
                output_tokens: output.to_string(),
                total_tokens: input.checked_add(output).unwrap_or(i128::MAX).to_string(),
                request_count: calls,
                consumed_coin_nano: cost.to_string(),
            },
        )
        .collect::<Vec<_>>();
    model_rows.sort_by(|left, right| {
        let left_total = left.total_tokens.parse::<i128>().unwrap_or(0);
        let right_total = right.total_tokens.parse::<i128>().unwrap_or(0);
        right_total
            .cmp(&left_total)
            .then_with(|| left.model.cmp(&right.model))
    });

    Ok(Json(ApiKeyAnalyticsResponse {
        key_id: api_key.id,
        key_name: api_key.name,
        range: query.range,
        time_from: time_from.to_rfc3339(),
        time_to: time_to.to_rfc3339(),
        total_tokens: raw.total_tokens.to_string(),
        total_input_tokens: raw.total_input_tokens.to_string(),
        total_cache_read_tokens: raw.total_cache_read_tokens.to_string(),
        total_output_tokens: raw.total_output_tokens.to_string(),
        request_count: raw.total_calls,
        consumed_coin_nano: raw.total_cost_nano_usd.to_string(),
        balance_mode: if api_key.sub_account_enabled {
            "independent"
        } else {
            "wallet"
        },
        independent_balance_nano: api_key
            .sub_account_enabled
            .then_some(api_key.sub_account_balance_nano),
        trend,
        models: model_rows,
    }))
}

pub async fn update_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(body): Json<UpdateApiKeyRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;

    let user_store = &state.user_store;

    let api_key = user_store
        .get_api_key_for_user(&key_id, &user.id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    let api_key = api_key
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "API key not found"))?;

    validate_channel_bindings_for_scope(
        &state,
        user.account_class,
        body.group_ids.as_deref().unwrap_or(&api_key.group_ids),
        body.model_limits_enabled
            .unwrap_or(api_key.model_limits_enabled),
        body.model_limits
            .as_deref()
            .unwrap_or(&api_key.model_limits),
        body.channel_bindings
            .as_deref()
            .unwrap_or(&api_key.channel_bindings),
    )
    .await
    .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, "invalid_request", error))?;

    let input = UpdateApiKeyInput {
        name: body.name,
        enabled: body.enabled,
        sub_account_enabled: body.sub_account_enabled,
        sub_account_balance_nano_usd: body.sub_account_balance_nano_usd,
        model_limits_enabled: body.model_limits_enabled,
        model_limits: body.model_limits,
        ip_whitelist: body.ip_whitelist,
        group_ids: body.group_ids,
        channel_bindings: body.channel_bindings,
        max_multiplier: body.max_multiplier,
        transforms: body.transforms,
        model_redirects: body.model_redirects,
        reasoning_envelope_enabled: body.reasoning_envelope_enabled,
        request_capture_mode: body.request_capture_mode,
        expires_at: body.expires_at,
    };

    let is_admin = user.role.can_manage_system();

    let updated_key = user_store
        .update_api_key(&key_id, input, is_admin)
        .await
        .map_err(|e| AppError::new(StatusCode::BAD_REQUEST, "invalid_request", e))?;

    let (nano, usd) = nano_balance_fields(&updated_key.sub_account_balance_nano)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    Ok(Json(ApiKeyResponse {
        id: updated_key.id,
        name: updated_key.name,
        key_prefix: updated_key.key_prefix,
        key: updated_key.key,
        created_at: updated_key.created_at.to_rfc3339(),
        expires_at: updated_key.expires_at.map(|d| d.to_rfc3339()),
        last_used_at: updated_key.last_used_at.map(|d| d.to_rfc3339()),
        enabled: updated_key.enabled,
        sub_account_enabled: updated_key.sub_account_enabled,
        sub_account_balance_nano_usd: nano,
        sub_account_balance_usd: usd,
        model_limits_enabled: updated_key.model_limits_enabled,
        model_limits: updated_key.model_limits,
        ip_whitelist: updated_key.ip_whitelist,
        group_ids: updated_key.group_ids,
        channel_bindings: updated_key.channel_bindings,
        max_multiplier: updated_key.max_multiplier,
        transforms: updated_key.transforms,
        model_redirects: updated_key.model_redirects,
        reasoning_envelope_enabled: updated_key.reasoning_envelope_enabled,
        request_capture_mode: updated_key.request_capture_mode,
    }))
}

pub async fn batch_delete_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BatchDeleteApiKeysRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;

    if body.ids.len() > crate::users::UserStore::api_key_batch_delete_max_ids() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!(
                "batch delete accepts at most {} ids",
                crate::users::UserStore::api_key_batch_delete_max_ids()
            ),
        ));
    }

    let user_store = &state.user_store;

    let ids_to_delete = user_store
        .filter_user_api_key_ids(&user.id, &body.ids)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    let deleted_count = user_store
        .batch_delete_api_keys(&ids_to_delete)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;

    Ok(Json(
        json!({ "success": true, "deleted_count": deleted_count }),
    ))
}

pub async fn get_apikey_presets(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    Ok(Json(crate::presets::apikey_presets()))
}

#[derive(Debug, Deserialize)]
pub struct TransferToSubAccountRequest {
    pub amount_nano_usd: Option<String>,
    pub amount_usd: Option<String>,
}

pub async fn transfer_to_sub_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(body): Json<TransferToSubAccountRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;

    let amount_nano = if let Some(nano_str) = &body.amount_nano_usd {
        parse_nano_usd(nano_str)
            .map_err(|e| AppError::new(StatusCode::BAD_REQUEST, "invalid_request", e))?
    } else if let Some(usd_str) = &body.amount_usd {
        crate::users::parse_usd_to_nano(usd_str)
            .map_err(|e| AppError::new(StatusCode::BAD_REQUEST, "invalid_request", e))?
    } else {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "amount_nano_usd or amount_usd is required",
        ));
    };

    if amount_nano <= 0 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "transfer amount must be positive",
        ));
    }

    let is_admin = user.role.can_manage_system();
    let api_key = state
        .user_store
        .get_api_key_by_id(&key_id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "API key not found"))?;

    if api_key.user_id != user.id && !is_admin {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "API key not found",
        ));
    }

    let (key_balance, user_balance) = state
        .user_store
        .transfer_to_sub_account(&key_id, &api_key.user_id, amount_nano)
        .await
        .map_err(|e| match e.kind {
            crate::users::BillingErrorKind::InsufficientBalance => AppError::new(
                StatusCode::PAYMENT_REQUIRED,
                "insufficient_balance",
                e.message,
            ),
            _ => AppError::new(StatusCode::BAD_REQUEST, "invalid_request", e.message),
        })?;

    Ok(Json(json!({
        "success": true,
        "api_key_balance_nano_usd": key_balance.to_string(),
        "user_balance_nano_usd": user_balance.to_string(),
    })))
}

#[cfg(test)]
mod tests {
    use super::{
        add_months, align_down_to_day, align_down_to_hour, align_down_to_month,
        analytics_bucket_plan, current_channel_conflicts, months_between, AnalyticsBucketPlan,
        AnalyticsBucketUnit,
    };
    use crate::app::{load_state_with_runtime, RuntimeConfig};
    use crate::billing_rate_store::UpsertBillingRateInput;
    use crate::monoize_routing::CreateMonoizeProviderInput;
    use crate::users::CreateGroupInput;
    use serde_json::json;

    #[tokio::test]
    async fn channel_conflicts_ignore_models_without_complete_pricing() {
        let state = load_state_with_runtime(RuntimeConfig {
            listen: "127.0.0.1:0".to_string(),
            metrics_path: "/metrics".to_string(),
            database_dsn: "sqlite::memory:".to_string(),
            request_log_spool_dir: None,
            node: crate::node_config::NodeSettings::primary_default(),
        })
        .await
        .expect("state loads");
        let group = state
            .user_store
            .create_group(CreateGroupInput {
                confirm_public_exposure: true,
                name: "conflict-group".to_string(),
                description: String::new(),
                user_selectable: true,
                sort_order: 1,
                account_class: Default::default(),
            })
            .await
            .expect("Group creates");

        for (name, profile) in [
            ("priced-provider", "conflict-priced"),
            ("unpriced-provider", "conflict-unpriced"),
        ] {
            let input: CreateMonoizeProviderInput = serde_json::from_value(json!({
                "name": name,
                "confirm_public_exposure": true,
                "group_id": group.id,
                "pricing_profile": profile,
                "channel": {
                    "name": format!("{name}-channel"),
                    "provider_type": "responses",
                    "base_url": "https://example.com",
                    "api_key": "secret",
                    "models": { "gpt-conflict": { "redirect": null } }
                }
            }))
            .expect("Provider input decodes");
            state
                .monoize_store
                .create_provider(input)
                .await
                .expect("Provider creates");
        }

        for usage_class in ["input_uncached", "output"] {
            state
                .billing_rate_store
                .upsert_billing_rate(
                    &format!("conflict-priced-{usage_class}"),
                    UpsertBillingRateInput {
                        source: Some("test".to_string()),
                        pricing_profile: Some("conflict-priced".to_string()),
                        model_pattern: Some(Some("gpt-conflict".to_string())),
                        provider_type: Some(Some("responses".to_string())),
                        rate_kind: Some("token".to_string()),
                        usage_class: Some(usage_class.to_string()),
                        unit: Some("token".to_string()),
                        unit_price_nano: Some("1".to_string()),
                        unit_price_currency: None,
                        context_tier: Some(None),
                        service_tier: Some(None),
                        modality: Some(None),
                        cache_ttl: Some(None),
                        match_json: Some(json!({})),
                        priority: Some(0),
                        enabled: Some(true),
                        raw_json: Some(json!({ "fixture": true })),
                    },
                )
                .await
                .expect("rate creates");
        }

        let conflicts = current_channel_conflicts(&state, crate::users::AccountClass::Standard)
            .await
            .expect("conflicts load");
        assert!(conflicts.is_empty());
    }

    /// TM-CH-6: conflicts are scoped to the caller's account class. A standard Group with two
    /// Channels on the same model is a real conflict for a standard caller but not for an
    /// enterprise caller, whose key can never route into that Group. Surfacing it would demand
    /// an impossible Channel choice and block every enterprise key.
    #[tokio::test]
    async fn channel_conflicts_are_scoped_to_the_callers_account_class() {
        let state = load_state_with_runtime(RuntimeConfig {
            listen: "127.0.0.1:0".to_string(),
            metrics_path: "/metrics".to_string(),
            database_dsn: "sqlite::memory:".to_string(),
            request_log_spool_dir: None,
            node: crate::node_config::NodeSettings::primary_default(),
        })
        .await
        .expect("state loads");

        for (name, account_class) in [
            ("standard-group", crate::users::AccountClass::Standard),
            ("enterprise-group", crate::users::AccountClass::Enterprise),
        ] {
            let group = state
                .user_store
                .create_group(CreateGroupInput {
                    confirm_public_exposure: true,
                    name: name.to_string(),
                    description: String::new(),
                    user_selectable: true,
                    sort_order: 1,
                    account_class,
                })
                .await
                .expect("Group creates");

            for (provider_name, profile) in [
                (format!("{name}-a"), format!("{name}-priced-a")),
                (format!("{name}-b"), format!("{name}-priced-b")),
            ] {
                let input: CreateMonoizeProviderInput = serde_json::from_value(json!({
                    "name": provider_name,
                    "confirm_public_exposure": true,
                    "group_id": group.id,
                    "enabled": true,
                    "pricing_profile": profile,
                    "channel": {
                        "name": format!("{provider_name}-channel"),
                        "provider_type": "responses",
                        "base_url": "https://example.com",
                        "api_key": "secret",
                        "enabled": true,
                        "models": { "gpt-scoped": { "redirect": null } }
                    }
                }))
                .expect("Provider input decodes");
                state
                    .monoize_store
                    .create_provider(input)
                    .await
                    .expect("Provider creates");
            }

            for usage_class in ["input_uncached", "output"] {
                for profile in [format!("{name}-priced-a"), format!("{name}-priced-b")] {
                    state
                        .billing_rate_store
                        .upsert_billing_rate(
                            &format!("{profile}-{usage_class}"),
                            UpsertBillingRateInput {
                                source: Some("test".to_string()),
                                pricing_profile: Some(profile.clone()),
                                model_pattern: Some(Some("gpt-scoped".to_string())),
                                provider_type: Some(Some("responses".to_string())),
                                rate_kind: Some("token".to_string()),
                                usage_class: Some(usage_class.to_string()),
                                unit: Some("token".to_string()),
                                unit_price_nano: Some("1".to_string()),
                        unit_price_currency: None,
                                context_tier: Some(None),
                                service_tier: Some(None),
                                modality: Some(None),
                                cache_ttl: Some(None),
                                match_json: Some(json!({})),
                                priority: Some(0),
                                enabled: Some(true),
                                raw_json: Some(json!({ "fixture": true })),
                            },
                        )
                        .await
                        .expect("rate creates");
                }
            }
        }

        let standard = current_channel_conflicts(&state, crate::users::AccountClass::Standard)
            .await
            .expect("standard conflicts load");
        assert_eq!(
            standard.len(),
            1,
            "the standard group has a two-Channel conflict on gpt-scoped"
        );
        assert_eq!(standard[0].group_name, "standard-group");

        let enterprise = current_channel_conflicts(&state, crate::users::AccountClass::Enterprise)
            .await
            .expect("enterprise conflicts load");
        assert_eq!(
            enterprise.len(),
            1,
            "the enterprise group has its own conflict"
        );
        assert_eq!(enterprise[0].group_name, "enterprise-group");
    }

    #[test]
    fn analytics_bucket_starts_align_to_their_labelled_unit() {
        // TM-AN5a: a mid-period instant must truncate down, otherwise a bucket
        // covering 10:37-11:37 would carry the label `10:00`.
        let mid = chrono::DateTime::parse_from_rfc3339("2026-09-08T10:37:41.523Z")
            .expect("fixed instant")
            .with_timezone(&chrono::Utc);

        assert_eq!(
            align_down_to_hour(mid).to_rfc3339(),
            "2026-09-08T10:00:00+00:00"
        );
        assert_eq!(
            align_down_to_day(mid).to_rfc3339(),
            "2026-09-08T00:00:00+00:00"
        );
        assert_eq!(
            align_down_to_month(mid).to_rfc3339(),
            "2026-09-01T00:00:00+00:00"
        );

        // An already-aligned instant is unchanged, so alignment is idempotent.
        let aligned = align_down_to_hour(mid);
        assert_eq!(align_down_to_hour(aligned), aligned);
        let day = align_down_to_day(mid);
        assert_eq!(align_down_to_day(day), day);
        let month = align_down_to_month(mid);
        assert_eq!(align_down_to_month(month), month);
    }

    fn fixed_instant(value: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(value)
            .expect("fixed instant")
            .with_timezone(&chrono::Utc)
    }

    async fn plan_for(range: &str, now: chrono::DateTime<chrono::Utc>) -> AnalyticsBucketPlan {
        analytics_bucket_plan(range, now, || async { Ok(now) })
            .await
            .expect("range resolves")
    }

    /// TM-AN5a: every bucket must cover exactly the interval its label names. A window ending
    /// at an unaligned `now` would divide into buckets about 61 minutes wide while labelling
    /// them on whole hours.
    #[tokio::test]
    async fn analytics_buckets_cover_exactly_the_interval_their_label_names() {
        let now = fixed_instant("2026-09-08T10:37:41.523Z");

        let last_day = plan_for("24h", now).await;
        assert_eq!(last_day.bucket_count, 24);
        assert_eq!(last_day.unit, AnalyticsBucketUnit::Hour);
        assert_eq!(last_day.time_to.to_rfc3339(), "2026-09-08T11:00:00+00:00");
        assert_eq!(last_day.time_from.to_rfc3339(), "2026-09-07T11:00:00+00:00");
        // The window is an exact multiple of the unit, so the equal-duration SQL buckets land
        // on the same boundaries the labels name.
        let range_ms = last_day.time_to.timestamp_millis() - last_day.time_from.timestamp_millis();
        assert_eq!(range_ms % last_day.bucket_count, 0);
        assert_eq!(range_ms / last_day.bucket_count, 3_600_000);
        for index in 0..last_day.bucket_count {
            let start = last_day.unit.advance(last_day.time_from, index);
            assert_eq!(
                start.timestamp_millis(),
                last_day.time_from.timestamp_millis() + index * 3_600_000
            );
            assert_eq!(start.timestamp_subsec_millis(), 0);
        }
        assert_eq!(
            last_day
                .unit
                .advance(last_day.time_from, 1)
                .format(last_day.label_format)
                .to_string(),
            "09-07 12:00"
        );

        let week = plan_for("7d", now).await;
        assert_eq!(week.bucket_count, 7);
        assert_eq!(week.unit, AnalyticsBucketUnit::Day);
        assert_eq!(week.time_to.to_rfc3339(), "2026-09-09T00:00:00+00:00");
        assert_eq!(week.time_from.to_rfc3339(), "2026-09-02T00:00:00+00:00");
        let week_ms = week.time_to.timestamp_millis() - week.time_from.timestamp_millis();
        assert_eq!(week_ms % week.bucket_count, 0);
        assert_eq!(week_ms / week.bucket_count, 86_400_000);

        let month = plan_for("30d", now).await;
        assert_eq!(month.bucket_count, 30);
        assert_eq!(month.time_to.to_rfc3339(), "2026-09-09T00:00:00+00:00");
        let month_ms = month.time_to.timestamp_millis() - month.time_from.timestamp_millis();
        assert_eq!(month_ms % month.bucket_count, 0);
        assert_eq!(month_ms / month.bucket_count, 86_400_000);
    }

    /// Calendar months have unequal lengths, so the month range must use calendar bucketing
    /// instead of an equal-duration split.
    #[tokio::test]
    async fn analytics_month_buckets_follow_the_calendar() {
        let now = fixed_instant("2026-09-08T10:37:41.523Z");
        let first = fixed_instant("2026-01-17T04:05:06Z");
        let plan = analytics_bucket_plan("all", now, || async { Ok(first) })
            .await
            .expect("range resolves");

        assert_eq!(plan.unit, AnalyticsBucketUnit::Month);
        assert!(plan.uses_calendar_months());
        assert_eq!(plan.time_from.to_rfc3339(), "2026-01-01T00:00:00+00:00");
        assert_eq!(plan.time_to.to_rfc3339(), "2026-10-01T00:00:00+00:00");
        assert_eq!(plan.bucket_count, 9);

        let labels = (0..plan.bucket_count)
            .map(|index| {
                plan.unit
                    .advance(plan.time_from, index)
                    .format(plan.label_format)
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                "2026-01", "2026-02", "2026-03", "2026-04", "2026-05", "2026-06", "2026-07",
                "2026-08", "2026-09",
            ]
        );
        // February is shorter than January, so an equal-duration split would drift.
        assert_eq!(
            plan.unit.advance(plan.time_from, 1).to_rfc3339(),
            "2026-02-01T00:00:00+00:00"
        );
        assert_eq!(
            plan.unit.advance(plan.time_from, 2).to_rfc3339(),
            "2026-03-01T00:00:00+00:00"
        );
    }

    /// A retained span at or below 90 days keeps daily buckets.
    #[tokio::test]
    async fn analytics_short_history_keeps_daily_buckets() {
        let now = fixed_instant("2026-09-08T10:37:41.523Z");
        let first = fixed_instant("2026-09-01T23:59:59Z");
        let plan = analytics_bucket_plan("all", now, || async { Ok(first) })
            .await
            .expect("range resolves");

        assert_eq!(plan.unit, AnalyticsBucketUnit::Day);
        assert!(!plan.uses_calendar_months());
        assert_eq!(plan.bucket_count, 8);
        assert_eq!(plan.time_to.to_rfc3339(), "2026-09-09T00:00:00+00:00");
        assert_eq!(plan.time_from.to_rfc3339(), "2026-09-01T00:00:00+00:00");
    }

    #[test]
    fn month_arithmetic_crosses_year_boundaries_in_both_directions() {
        let december = fixed_instant("2026-12-14T09:00:00Z");
        assert_eq!(
            add_months(december, 1).to_rfc3339(),
            "2027-01-01T00:00:00+00:00"
        );
        assert_eq!(
            add_months(december, -12).to_rfc3339(),
            "2025-12-01T00:00:00+00:00"
        );
        // A 31st never overflows into the next month, because every bucket starts on day 1.
        let january31 = fixed_instant("2026-01-31T23:00:00Z");
        assert_eq!(
            add_months(january31, 1).to_rfc3339(),
            "2026-02-01T00:00:00+00:00"
        );
        assert_eq!(
            months_between(
                fixed_instant("2026-01-01T00:00:00Z"),
                fixed_instant("2026-10-01T00:00:00Z")
            ),
            9
        );
    }
}
