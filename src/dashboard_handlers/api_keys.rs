use crate::app::AppState;
use crate::dashboard_handlers::session_helpers::{get_current_user, require_admin};
use crate::error::{AppError, AppResult};
use crate::exact_decimal::Multiplier;
use crate::transforms::TransformRuleConfig;
use crate::users::{
    ApiKeyChannelBinding, CreateApiKeyInput, CreateApiKeyWithLimitError, ModelRedirectRule,
    RequestCaptureMode, UpdateApiKeyInput, canonicalize_channel_bindings, format_nano_to_usd,
    parse_nano_usd,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;

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

async fn current_channel_conflicts(
    state: &AppState,
) -> Result<Vec<ApiKeyChannelConflictResponse>, String> {
    let group_names = state
        .user_store
        .list_groups()
        .await?
        .into_iter()
        .map(|group| (group.id, group.name))
        .collect::<std::collections::BTreeMap<_, _>>();
    let providers = state.monoize_store.list_providers().await?;
    let mut by_scope = std::collections::BTreeMap::<
        (String, String),
        Vec<ApiKeyChannelOptionResponse>,
    >::new();
    for provider in providers {
        if !provider.enabled || !provider.channel.enabled {
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
        .map(|((group_id, model), options)| ApiKeyChannelConflictResponse {
            group_name: group_names
                .get(&group_id)
                .cloned()
                .unwrap_or_else(|| group_id.clone()),
            group_id,
            model,
            options,
        })
        .collect())
}

async fn validate_channel_bindings_for_scope(
    state: &AppState,
    group_ids: &[String],
    model_limits_enabled: bool,
    model_limits: &[String],
    bindings: &[ApiKeyChannelBinding],
) -> Result<(), String> {
    let bindings = canonicalize_channel_bindings(bindings)?;
    let conflicts = current_channel_conflicts(state).await?;
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
    get_current_user(&headers, &state).await?;
    let conflicts = current_channel_conflicts(&state)
        .await
        .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error))?;
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

    api_key.ok_or_else(|| AppError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "API key not found",
        ))?;

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

    let api_key = api_key.ok_or_else(|| AppError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "API key not found",
        ))?;

    validate_channel_bindings_for_scope(
        &state,
        body.group_ids.as_deref().unwrap_or(&api_key.group_ids),
        body.model_limits_enabled
            .unwrap_or(api_key.model_limits_enabled),
        body.model_limits.as_deref().unwrap_or(&api_key.model_limits),
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
    use super::current_channel_conflicts;
    use crate::app::{RuntimeConfig, load_state_with_runtime};
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
                        unit_price_nano_usd: Some("1".to_string()),
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

        let conflicts = current_channel_conflicts(&state)
            .await
            .expect("conflicts load");
        assert!(conflicts.is_empty());
    }
}
