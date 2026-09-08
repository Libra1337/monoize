use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::{Extension, Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use uuid::Uuid;

use crate::app::AppState;
use crate::replica::metering::{
    METERING_INGEST_PATH, ingest_metering_handler, verify_ingest_token,
};
use crate::store_billing::admission_runtime::{
    AdmissionDecision, AdmissionRuntimeError, AdmissionService, ConfirmAdmissionInput,
    IssueAdmissionInput, PublicAdmissionKey,
};

pub const ADMISSION_ISSUE_PATH: &str = "/internal/replica/admission/issue";
pub const ADMISSION_CONFIRM_PATH: &str = "/internal/replica/admission/confirm";
pub const ADMISSION_KEYSET_PATH: &str = "/internal/replica/admission/keyset";

const MAX_ADMISSION_BODY_BYTES: usize = 65_536;
const UNCONFIRMED_REAPER_INTERVAL: Duration = Duration::from_secs(5);
const UNCONFIRMED_REAPER_LIMIT: usize = 100;

#[derive(Clone, Copy)]
struct ReplicaAuthState {
    expected_digest: [u8; 32],
}

#[derive(Clone)]
struct AuthenticatedReplicaId(String);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueRequest {
    audience: String,
    user_id: String,
    request_id: String,
    effective_groups: Vec<String>,
    maximum_nano_usd: String,
    pricing_revision: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmRequest {
    audience: String,
    token_id: String,
    reservation_id: String,
    request_id: String,
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: String,
}

#[derive(Serialize)]
struct KeysetResponse {
    keys: Vec<KeysetEntry>,
}

#[derive(Serialize)]
struct KeysetEntry {
    key_id: String,
    public_key_base64: String,
    state: String,
    activated_at: chrono::DateTime<Utc>,
    verify_until: Option<chrono::DateTime<Utc>>,
}

pub(crate) fn internal_router(expected_digest: [u8; 32]) -> Router<AppState> {
    Router::new()
        .route(ADMISSION_ISSUE_PATH, post(issue_admission))
        .route(ADMISSION_CONFIRM_PATH, post(confirm_admission))
        .route(ADMISSION_KEYSET_PATH, get(admission_keyset))
        .route(METERING_INGEST_PATH, post(ingest_metering_handler))
        .route_layer(middleware::from_fn_with_state(
            ReplicaAuthState { expected_digest },
            authenticate_replica,
        ))
}

pub(crate) fn spawn_unconfirmed_reaper(
    service: Arc<AdmissionService>,
    lease: crate::store_billing::availability::StorePrimaryLease,
    shutdown: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(UNCONFIRMED_REAPER_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            if let Err(error) = lease.validate().await {
                tracing::error!(error = %error, "unconfirmed admission recovery lost Primary lease");
                break;
            }
            if let Err(error) = service
                .recover_unconfirmed(Utc::now(), UNCONFIRMED_REAPER_LIMIT)
                .await
            {
                tracing::warn!(error = %error, "unconfirmed admission recovery failed");
            }
        }
    });
}

async fn authenticate_replica(
    State(auth): State<ReplicaAuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(replica_id) = authenticated_replica_id(request.headers(), &auth.expected_digest)
    else {
        return admission_error(
            StatusCode::UNAUTHORIZED,
            "replica_auth_failed",
            "replica authentication failed",
        );
    };
    request
        .extensions_mut()
        .insert(AuthenticatedReplicaId(replica_id));
    next.run(request).await
}

fn authenticated_replica_id(headers: &HeaderMap, expected_digest: &[u8; 32]) -> Option<String> {
    if !verify_ingest_token(headers, expected_digest) {
        return None;
    }
    let raw = headers.get("X-Monoize-Replica-ID")?.to_str().ok()?;
    let parsed = Uuid::parse_str(raw).ok()?;
    if parsed.get_version_num() != 4 || parsed.to_string() != raw {
        return None;
    }
    Some(raw.to_string())
}

async fn issue_admission(
    State(state): State<AppState>,
    Extension(replica_id): Extension<AuthenticatedReplicaId>,
    request: Request<Body>,
) -> Response {
    if let Err(response) = require_store_primary(&state).await {
        return response;
    }
    let body: IssueRequest = match read_json(request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    if body.audience != replica_id.0 {
        return admission_error(
            StatusCode::FORBIDDEN,
            "replica_audience_mismatch",
            "request audience does not match the authenticated Replica",
        );
    }
    let Some(maximum_nano_usd) = parse_positive_canonical_decimal(&body.maximum_nano_usd) else {
        return admission_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "admission_input_invalid",
            "maximum_nano_usd must be a positive canonical decimal integer string",
        );
    };
    let Some(service) = state.admission_service else {
        return admission_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "admission_storage_error",
            "admission service is unavailable",
        );
    };
    match service
        .issue(IssueAdmissionInput {
            audience: body.audience,
            user_id: body.user_id,
            request_id: body.request_id,
            effective_groups: body.effective_groups,
            maximum_nano_usd,
            pricing_revision: body.pricing_revision,
            issued_at: primary_admission_now(),
        })
        .await
    {
        Ok(AdmissionDecision::Balance) => Json(json!({ "funding": "balance" })).into_response(),
        Ok(AdmissionDecision::Plan(issued)) => Json(json!({
            "funding": "plan",
            "token_id": issued.token_id,
            "reservation_id": issued.reservation_id,
            "compact_jws": issued.compact_jws,
            "issued_at": issued.issued_at,
            "expires_at": issued.expires_at,
            "duplicate": issued.duplicate,
        }))
        .into_response(),
        Err(error) => admission_runtime_error(error),
    }
}

fn primary_admission_now() -> chrono::DateTime<Utc> {
    chrono::DateTime::from_timestamp(Utc::now().timestamp(), 0)
        .expect("the current UTC Unix timestamp is representable")
}

async fn confirm_admission(
    State(state): State<AppState>,
    Extension(replica_id): Extension<AuthenticatedReplicaId>,
    request: Request<Body>,
) -> Response {
    if let Err(response) = require_store_primary(&state).await {
        return response;
    }
    let body: ConfirmRequest = match read_json(request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    if body.audience != replica_id.0 {
        return admission_error(
            StatusCode::FORBIDDEN,
            "replica_audience_mismatch",
            "request audience does not match the authenticated Replica",
        );
    }
    let Some(service) = state.admission_service else {
        return admission_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "admission_storage_error",
            "admission service is unavailable",
        );
    };
    match service
        .confirm(ConfirmAdmissionInput {
            audience: body.audience,
            token_id: body.token_id,
            reservation_id: body.reservation_id,
            request_id: body.request_id,
            confirmed_at: Utc::now(),
        })
        .await
    {
        Ok(result) => Json(json!({
            "confirmed": true,
            "duplicate": result.duplicate,
        }))
        .into_response(),
        Err(error) => admission_runtime_error(error),
    }
}

async fn require_store_primary(state: &AppState) -> Result<(), Response> {
    state.validate_store_primary_lease().await.map_err(|error| {
        tracing::warn!(error = %error, "plan admission rejected by Store Primary lease");
        admission_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "store_primary_unavailable",
            "Store Primary is unavailable",
        )
    })
}

async fn admission_keyset(State(state): State<AppState>, request: Request<Body>) -> Response {
    let bytes = match to_bytes(request.into_body(), 1).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return admission_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "admission_input_invalid",
                "keyset requests must not contain a body",
            );
        }
    };
    if !bytes.is_empty() {
        return admission_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "admission_input_invalid",
            "keyset requests must not contain a body",
        );
    }
    let Some(service) = state.admission_service else {
        return admission_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "admission_storage_error",
            "admission service is unavailable",
        );
    };
    match service.public_keyset(Utc::now()).await {
        Ok(keys) => {
            let mut response = Json(KeysetResponse {
                keys: keys.into_iter().map(KeysetEntry::from).collect(),
            })
            .into_response();
            response.headers_mut().insert(
                CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            response
        }
        Err(error) => admission_runtime_error(error),
    }
}

async fn read_json<T: DeserializeOwned>(request: Request<Body>) -> Result<T, Response> {
    let valid_content_type = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<mime::Mime>().ok())
        .is_some_and(|value| value.essence_str() == "application/json");
    if !valid_content_type {
        return Err(admission_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "admission_content_type_invalid",
            "Content-Type must be application/json",
        ));
    }
    let bytes = to_bytes(request.into_body(), MAX_ADMISSION_BODY_BYTES)
        .await
        .map_err(|_| {
            admission_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "admission_request_too_large",
                "admission request exceeds 65536 bytes",
            )
        })?;
    serde_json::from_slice(&bytes).map_err(|_| {
        admission_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "admission_input_invalid",
            "admission request body is invalid",
        )
    })
}

fn parse_positive_canonical_decimal(value: &str) -> Option<i128> {
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.parse::<i128>().ok().filter(|value| *value > 0)
}

fn admission_runtime_error(error: AdmissionRuntimeError) -> Response {
    let code = error.code();
    let status = match code {
        "admission_input_invalid" | "admission_terminal_digest_invalid" => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        "plan_quota_exhausted" | "plan_request_unbounded" => StatusCode::PAYMENT_REQUIRED,
        "plan_payment_hold" | "plan_quota_violation_blocked" => StatusCode::LOCKED,
        "admission_issue_conflict"
        | "admission_binding_mismatch"
        | "admission_confirmation_expired"
        | "admission_terminal_conflict" => StatusCode::CONFLICT,
        "admission_token_not_found" => StatusCode::NOT_FOUND,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    tracing::warn!(code, detail = %error, "Primary admission request failed");
    admission_error(status, code, public_admission_message(code))
}

pub(crate) fn public_admission_message(code: &str) -> &'static str {
    match code {
        "plan_quota_exhausted" => "plan quota exhausted",
        "plan_request_unbounded" => "plan request has no finite billing bound",
        "plan_payment_hold" => "plan admission is blocked by a payment hold",
        "plan_quota_violation_blocked" => "plan admission is blocked by a quota violation",
        "admission_token_not_found" => "admission token is not found",
        "admission_issue_conflict"
        | "admission_binding_mismatch"
        | "admission_confirmation_expired"
        | "admission_terminal_conflict" => "admission request conflicts with stored state",
        _ => "plan admission is unavailable",
    }
}

fn admission_error(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorEnvelope {
            error: ErrorBody {
                code,
                message: message.into(),
            },
        }),
    )
        .into_response()
}

impl From<PublicAdmissionKey> for KeysetEntry {
    fn from(value: PublicAdmissionKey) -> Self {
        Self {
            key_id: value.key_id,
            public_key_base64: value.public_key_base64,
            state: value.state,
            activated_at: value.activated_at,
            verify_until: value.verify_until,
        }
    }
}
