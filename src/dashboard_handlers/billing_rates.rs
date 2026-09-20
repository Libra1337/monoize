use crate::app::AppState;
use crate::billing_rate_store::{
    BillingRateProfileSummary, BillingRateSyncResult, CopyProfileError, DbBillingRateRecord,
    DeleteProfileError, RenameProfileModelError, UpsertBillingRateInput,
};
use crate::dashboard_handlers::session_helpers::require_admin;
use crate::error::{AppError, AppResult};
use crate::settings::PricingProfilePattern;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use sea_orm::ConnectionTrait;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Default, Deserialize)]
pub struct BillingRatesListQuery {
    #[serde(default)]
    pub pricing_profile: Option<String>,
}

pub async fn list_billing_rates(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BillingRatesListQuery>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let rows: Vec<DbBillingRateRecord> = match query
        .pricing_profile
        .as_deref()
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
    {
        Some(profile) => {
            state
                .billing_rate_store
                .list_billing_rates_for_profile(profile)
                .await
        }
        None => state.billing_rate_store.list_billing_rates().await,
    }
    .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize)]
pub struct BillingRateProfilesResponse {
    pub profiles: Vec<BillingRateProfileSummary>,
}

pub async fn list_billing_rate_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let profiles = state
        .billing_rate_store
        .list_pricing_profile_summaries()
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    Ok(Json(BillingRateProfilesResponse { profiles }))
}

#[derive(Debug, Deserialize)]
pub struct CopyPricingProfileRequest {
    pub target_profile: String,
}

#[derive(Debug, Serialize)]
pub struct CopyPricingProfileResponse {
    pub target_profile: String,
    pub copied: usize,
}

#[derive(Debug, Serialize)]
pub struct DeletePricingProfileResponse {
    pub deleted_rates: u64,
    pub deleted_models: u64,
}

/// Deletes every rate row of one pricing profile (MB-A11).
///
/// Reference checks run before the store call so a profile that routes traffic (match
/// rules) or backs a Provider can never lose its prices while still being selected.
pub async fn delete_pricing_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile): Path<String>,
) -> AppResult<Json<DeletePricingProfileResponse>> {
    require_admin(&headers, &state).await?;
    let profile = profile.trim();
    if profile.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "profile must not be empty",
        ));
    }
    let patterns = state
        .settings_store
        .get_pricing_profile_model_patterns()
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    if patterns
        .iter()
        .any(|pattern: &PricingProfilePattern| pattern.pricing_profile == profile)
    {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "pricing_profile_in_use_patterns",
            "profile is referenced by a pricing-profile match rule; remove the rule first",
        ));
    }
    let provider_refs: i64 = state
        .db_pool
        .read()
        .query_one(state.db_pool.stmt(
            "SELECT ( \
                (SELECT COUNT(*) FROM monoize_providers WHERE pricing_profile = $1) \
                + (SELECT COUNT(*) FROM monoize_provider_models WHERE pricing_profile_override = $1) \
             ) AS refs",
            vec![profile.to_string().into()],
        ))
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e.to_string()))?
        .and_then(|row| row.try_get::<i64>("", "refs").ok())
        .unwrap_or(0);
    if provider_refs > 0 {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "pricing_profile_in_use_providers",
            "profile is referenced by a provider; detach the provider first",
        ));
    }
    let (deleted_rates, deleted_models) = state
        .billing_rate_store
        .delete_profile(profile)
        .await
        .map_err(|error| match error {
            DeleteProfileError::NotFound => {
                AppError::new(StatusCode::NOT_FOUND, "not_found", error.to_string())
            }
            DeleteProfileError::PatternsInUse => AppError::new(
                StatusCode::CONFLICT,
                "pricing_profile_in_use_patterns",
                error.to_string(),
            ),
            DeleteProfileError::ProvidersInUse => AppError::new(
                StatusCode::CONFLICT,
                "pricing_profile_in_use_providers",
                error.to_string(),
            ),
            DeleteProfileError::Storage(message) => {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
            }
        })?;
    Ok(Json(DeletePricingProfileResponse {
        deleted_rates,
        deleted_models,
    }))
}

/// Copies a pricing profile's rates under a new name (MB-A7).
///
/// Profile names must stay disjoint across account classes (PP-ENT6), so giving both classes
/// the same prices requires two named copies of the rate set.
pub async fn copy_pricing_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile): Path<String>,
    Json(body): Json<CopyPricingProfileRequest>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let target = body.target_profile.trim().to_string();
    let copied = state
        .billing_rate_store
        .copy_profile(&profile, &target)
        .await
        .map_err(|error| match error {
            CopyProfileError::InvalidTarget | CopyProfileError::SameProfile => AppError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                error.to_string(),
            ),
            CopyProfileError::SourceNotFound => {
                AppError::new(StatusCode::NOT_FOUND, "not_found", error.to_string())
            }
            CopyProfileError::TargetNotEmpty => AppError::new(
                StatusCode::CONFLICT,
                "pricing_profile_not_empty",
                error.to_string(),
            ),
            CopyProfileError::Storage(message) => {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
            }
        })?;
    Ok(Json(CopyPricingProfileResponse {
        target_profile: target,
        copied,
    }))
}

#[derive(Debug, Deserialize)]
pub struct RenameProfileModelRequest {
    pub target_model: String,
}

/// Renames a model inside one pricing profile (MB-A9).
///
/// A profile's model name is not fixed: an operator who renames an upstream model, or who
/// serves the same prices under a second alias, carries the priced rows across without
/// retyping every usage class. Rows the model registry owns (`model_metadata:` mirrors) stay
/// under the former name and are reported as `synchronized_retained` (MB-A9c).
pub async fn rename_pricing_profile_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((profile, model)): Path<(String, String)>,
    Json(body): Json<RenameProfileModelRequest>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let outcome = state
        .billing_rate_store
        .rename_profile_model(&profile, &model, &body.target_model)
        .await
        .map_err(|error| match error {
            RenameProfileModelError::InvalidTarget | RenameProfileModelError::SameModel => {
                AppError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    error.to_string(),
                )
            }
            RenameProfileModelError::SourceNotFound => {
                AppError::new(StatusCode::NOT_FOUND, "not_found", error.to_string())
            }
            RenameProfileModelError::TargetNotEmpty => AppError::new(
                StatusCode::CONFLICT,
                "pricing_profile_model_not_empty",
                error.to_string(),
            ),
            RenameProfileModelError::Storage(message) => {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
            }
        })?;
    Ok(Json(outcome))
}

pub async fn upsert_billing_rate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UpsertBillingRateInput>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let id = id.strip_prefix('/').unwrap_or(&id);
    let row = state
        .billing_rate_store
        .upsert_billing_rate(id, input)
        .await
        .map_err(|e| AppError::new(StatusCode::BAD_REQUEST, "invalid_request", e))?;
    Ok(Json(row))
}

pub async fn delete_billing_rate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let id = id.strip_prefix('/').unwrap_or(&id);
    let deleted = state
        .billing_rate_store
        .delete_billing_rate(id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    if !deleted {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "billing rate not found",
        ));
    }
    Ok(Json(json!({ "success": true })))
}

pub async fn sync_billing_rates_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let result: BillingRateSyncResult = state
        .billing_rate_store
        .sync_catalog()
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    Ok(Json(result))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingProfilePatternsResponse {
    pub patterns: Vec<PricingProfilePattern>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePricingProfilePatternsRequest {
    pub patterns: Vec<PricingProfilePattern>,
}

pub async fn get_pricing_profile_patterns(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let patterns = state
        .settings_store
        .get_pricing_profile_model_patterns()
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    Ok(Json(PricingProfilePatternsResponse { patterns }))
}

pub async fn update_pricing_profile_patterns(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdatePricingProfilePatternsRequest>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    for pattern in &body.patterns {
        if pattern.pattern.trim().is_empty() || pattern.pricing_profile.trim().is_empty() {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "pattern and pricing_profile must not be empty",
            ));
        }
    }
    let _update_guard = state.settings_update_lock.lock().await;
    state
        .settings_store
        .set_pricing_profile_model_patterns(&body.patterns)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    state
        .monoize_runtime
        .write()
        .await
        .pricing_profile_model_patterns = body.patterns.clone();
    Ok(Json(PricingProfilePatternsResponse {
        patterns: body.patterns,
    }))
}
