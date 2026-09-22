//! Platform-side bridge to the standalone Apeiron video studio
//! (`studio-bridge.spec.md`): handoff token minting on `/studio-entry` and the
//! service-authenticated wallet settlement API under `/api/studio-bridge/*`.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use aes_gcm::aead::KeyInit as _;
use sea_orm::ConnectionTrait;
use base64::Engine;
use hmac::{Hmac, Mac as _};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::AppState;

pub const HANDOFF_TTL_SECS: u64 = 120;
const NONCE_WINDOW: usize = 1024;

#[derive(Clone, Debug, Default)]
pub struct StudioBridgeConfig {
    pub secret: Option<String>,
    pub service_token_digest: Option<[u8; 32]>,
    pub apeiron_url: String,
}

impl StudioBridgeConfig {
    pub fn from_env() -> Self {
        let secret = std::env::var("MONOIZE_STUDIO_BRIDGE_SECRET")
            .ok()
            .map(|raw| raw.trim().to_string())
            .filter(|value| !value.is_empty());
        let service_token = std::env::var("MONOIZE_STUDIO_BRIDGE_SERVICE_TOKEN")
            .ok()
            .map(|raw| raw.trim().to_string())
            .filter(|value| !value.is_empty());
        Self {
            service_token_digest: service_token
                .map(|token| crate::replica::metering::sha256_hex_lower(&token)),
            secret,
            apeiron_url: std::env::var("MONOIZE_APEIRON_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8090".to_string())
                .trim()
                .trim_end_matches('/')
                .to_string(),
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn sign_handoff(secret: &str, payload_b64: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(b"apeiron-handoff.v1\n");
    mac.update(payload_b64.as_bytes());
    hex_lower(&mac.finalize().into_bytes())
}

pub fn mint_handoff_token(secret: &str, user_id: &str) -> String {
    let exp = now_unix() + HANDOFF_TTL_SECS;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let payload = json!({ "sub": user_id, "exp": exp, "nonce": nonce });
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string());
    let sig = sign_handoff(secret, &payload_b64);
    format!("v1.{payload_b64}.{sig}")
}

pub struct HandoffClaims {
    pub sub: String,
    pub nonce: String,
}

#[derive(Debug)]
pub enum HandoffError {
    Invalid,
    Expired,
    Replayed,
}

fn nonce_digest(nonce: &str) -> [u8; 32] {
    crate::replica::metering::sha256_hex_lower(nonce)
}

/// SB-5: best-effort single-use window over the most recent nonces.
fn consume_nonce(nonce: &str) -> bool {
    static USED: Mutex<Option<VecDeque<[u8; 32]>>> = Mutex::new(None);
    let digest = nonce_digest(nonce);
    let mut guard = USED.lock().expect("studio bridge nonce lock");
    let window = guard.get_or_insert_with(VecDeque::new);
    if window.contains(&digest) {
        return false;
    }
    if window.len() >= NONCE_WINDOW {
        window.pop_front();
    }
    window.push_back(digest);
    true
}

pub fn verify_handoff_token(secret: &str, token: &str) -> Result<HandoffClaims, HandoffError> {
    let mut parts = token.split('.');
    let version = parts.next().unwrap_or_default();
    let payload_b64 = parts.next().unwrap_or_default();
    let sig_hex = parts.next().unwrap_or_default();
    if version != "v1" || payload_b64.is_empty() || parts.next().is_some() {
        return Err(HandoffError::Invalid);
    }
    let expected = sign_handoff(secret, payload_b64);
    let provided = sig_hex.to_ascii_lowercase();
    if provided.len() != expected.len()
        || !provided.bytes().zip(expected.bytes()).all(|(a, b)| a == b)
    {
        return Err(HandoffError::Invalid);
    }
    let raw = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| HandoffError::Invalid)?;
    let claims: Value = serde_json::from_slice(&raw).map_err(|_| HandoffError::Invalid)?;
    let sub = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(HandoffError::Invalid)?
        .to_string();
    let nonce = claims
        .get("nonce")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(HandoffError::Invalid)?
        .to_string();
    let exp = claims.get("exp").and_then(Value::as_u64).ok_or(HandoffError::Invalid)?;
    if exp <= now_unix() {
        return Err(HandoffError::Expired);
    }
    if !consume_nonce(&nonce) {
        return Err(HandoffError::Replayed);
    }
    Ok(HandoffClaims { sub, nonce })
}

fn bridge_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

fn require_service_token(headers: &HeaderMap, state: &AppState) -> Option<Response> {
    match state.studio_bridge.service_token_digest {
        Some(ref digest)
            if crate::replica::metering::verify_ingest_token(headers, digest) =>
        {
            None
        }
        _ => Some(bridge_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid studio bridge service token",
        )),
    }
}

/// SB-6: browser entry point; redirects into the standalone Apeiron site with a
/// fresh single-use handoff token.
pub async fn studio_entry(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(secret) = state.studio_bridge.secret.clone() else {
        return bridge_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "bridge_disabled",
            "MONOIZE_STUDIO_BRIDGE_SECRET is not configured",
        );
    };
    let user = match crate::dashboard_handlers::session_helpers::get_current_user(&headers, &state).await {
        Ok(user) => user,
        Err(_) => {
            return Redirect::to("/login?next=%2Fstudio-entry").into_response();
        }
    };
    let token = mint_handoff_token(&secret, &user.id);
    Redirect::to(&format!(
        "{}/handoff?token={}",
        state.studio_bridge.apeiron_url, token
    ))
    .into_response()
}

#[derive(Deserialize)]
pub struct ExchangeBody {
    pub token: String,
}

async fn resolve_user(state: &AppState, user_id: &str) -> Result<crate::users::User, Response> {
    let user = state
        .user_store
        .get_user_by_id(user_id)
        .await
        .map_err(|_| {
            bridge_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "user lookup failed",
            )
        })?
        .filter(|user| user.enabled)
        .ok_or_else(|| {
            bridge_error(
                StatusCode::NOT_FOUND,
                "user_not_found",
                "user does not exist or is disabled",
            )
        })?;
    Ok(user)
}

/// SB-7: swap a handoff token for the caller's identity and wallet snapshot.
pub async fn bridge_exchange(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ExchangeBody>,
) -> Response {
    if let Some(denied) = require_service_token(&headers, &state) {
        return denied;
    }
    let Some(secret) = state.studio_bridge.secret.clone() else {
        return bridge_error(
            StatusCode::NOT_FOUND,
            "bridge_disabled",
            "MONOIZE_STUDIO_BRIDGE_SECRET is not configured",
        );
    };
    let claims = match verify_handoff_token(&secret, &body.token) {
        Ok(claims) => claims,
        Err(HandoffError::Expired) => {
            return bridge_error(
                StatusCode::GONE,
                "expired_token",
                "handoff token expired",
            )
        }
        Err(HandoffError::Replayed) => {
            return bridge_error(
                StatusCode::CONFLICT,
                "replayed_token",
                "handoff token already exchanged",
            )
        }
        Err(HandoffError::Invalid) => {
            return bridge_error(StatusCode::BAD_REQUEST, "invalid_token", "malformed token")
        }
    };
    let user = match resolve_user(&state, &claims.sub).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let balance = state
        .user_store
        .get_user_balance_uncached(&user.id)
        .await
        .ok()
        .flatten();
    let role = serde_json::to_value(&user.role)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "user".to_string());
    (
        StatusCode::OK,
        Json(json!({
            "user_id": user.id,
            "username": user.username,
            "display_name": user.email.clone().unwrap_or_else(|| user.username.clone()),
            "role": role,
            "balance_nano_usd": balance
                .as_ref()
                .map(|b| b.balance_nano_usd.to_string())
                .unwrap_or_else(|| user.balance_nano_usd.clone()),
            "balance_unlimited": user.balance_unlimited,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct BalanceQuery {
    pub user_id: String,
}

/// SB-8.
pub async fn bridge_balance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BalanceQuery>,
) -> Response {
    if let Some(denied) = require_service_token(&headers, &state) {
        return denied;
    }
    let user = match resolve_user(&state, &query.user_id).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let balance = state
        .user_store
        .get_user_balance_uncached(&user.id)
        .await
        .ok()
        .flatten();
    (
        StatusCode::OK,
        Json(json!({
            "balance_nano_usd": balance
                .as_ref()
                .map(|b| b.balance_nano_usd.to_string())
                .unwrap_or_else(|| user.balance_nano_usd.clone()),
            "balance_unlimited": user.balance_unlimited,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct SettleBody {
    pub user_id: String,
    pub amount_nano_usd: i64,
    pub idempotency_key: String,
    #[serde(default)]
    pub meta: Value,
}

impl SettleBody {
    fn validate(&self) -> Option<Response> {
        if self.amount_nano_usd < 0 {
            return Some(bridge_error(
                StatusCode::BAD_REQUEST,
                "invalid_params",
                "amount_nano_usd must be >= 0",
            ));
        }
        if self.idempotency_key.trim().is_empty()
            || self.idempotency_key.len() > 128
            || !self.idempotency_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Some(bridge_error(
                StatusCode::BAD_REQUEST,
                "invalid_params",
                "idempotency_key must be 1..=128 printable ASCII characters",
            ));
        }
        if !self.meta.is_object() && !self.meta.is_null() {
            return Some(bridge_error(
                StatusCode::BAD_REQUEST,
                "invalid_params",
                "meta must be an object",
            ));
        }
        None
    }
}

enum OpOutcome {
    Applied { duplicate: bool },
    InProgress,
    Failed { code: &'static str, status: StatusCode },
}

/// SB-9 step 1–2: reserve the idempotency key; `None` means this caller owns
/// the operation and must run the settlement.
async fn reserve_operation(state: &AppState, body: &SettleBody, kind: &str) -> Option<OpOutcome> {
    let now = chrono::Utc::now().to_rfc3339();
    {
        let guard = state.db_pool.write().await;
        let insert = guard
            .execute(state.db_pool.stmt(
                "INSERT INTO studio_bridge_ops \
                 (idempotency_key, user_id, kind, amount_nano_usd, status, error, created_at) \
                 VALUES ($1, $2, $3, $4, 'pending', NULL, $5) \
                 ON CONFLICT (idempotency_key) DO NOTHING",
                vec![
                    body.idempotency_key.clone().into(),
                    body.user_id.clone().into(),
                    kind.to_string().into(),
                    body.amount_nano_usd.to_string().into(),
                    now.into(),
                ],
            ))
            .await;
        match insert {
            Ok(result) if result.rows_affected() == 1 => return None,
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "studio bridge op reserve failed");
                return Some(OpOutcome::Failed {
                    code: "internal_error",
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                });
            }
        }
    }
    let existing = {
        let guard = state.db_pool.read();
        guard
            .query_one(state.db_pool.stmt(
                "SELECT status, error FROM studio_bridge_ops WHERE idempotency_key = $1",
                vec![body.idempotency_key.clone().into()],
            ))
            .await
    };
    match existing {
        Ok(Some(row)) => {
            let status = row
                .try_get::<String>("", "status")
                .unwrap_or_else(|_| "pending".to_string());
            match status.as_str() {
                "applied" => Some(OpOutcome::Applied { duplicate: true }),
                "pending" => Some(OpOutcome::InProgress),
                _ => {
                    let error = row
                        .try_get::<Option<String>>("", "error")
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| "internal_error".to_string());
                    if error == "insufficient_balance" {
                        Some(OpOutcome::Failed {
                            code: "insufficient_balance",
                            status: StatusCode::CONFLICT,
                        })
                    } else {
                        Some(OpOutcome::Failed {
                            code: "internal_error",
                            status: StatusCode::INTERNAL_SERVER_ERROR,
                        })
                    }
                }
            }
        }
        _ => Some(OpOutcome::Failed {
            code: "internal_error",
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }),
    }
}

async fn mark_operation(
    state: &AppState,
    body: &SettleBody,
    status: &str,
    error: Option<&str>,
) {
    let guard = state.db_pool.write().await;
    let result = guard
        .execute(state.db_pool.stmt(
            "UPDATE studio_bridge_ops SET status = $1, error = $2 WHERE idempotency_key = $3",
            vec![
                status.to_string().into(),
                error.map(str::to_string).into(),
                body.idempotency_key.clone().into(),
            ],
        ))
        .await;
    if let Err(error) = result {
        tracing::warn!(%error, "studio bridge op mark failed");
    }
}

async fn settle(
    state: &AppState,
    headers: &HeaderMap,
    body: SettleBody,
    kind: &str,
) -> Response {
    if let Some(denied) = require_service_token(&headers, &state) {
        return denied;
    }
    if let Some(denied) = body.validate() {
        return denied;
    }
    if body.amount_nano_usd == 0 {
        return (
            StatusCode::OK,
            Json(json!({ "applied": true, "duplicate": false })),
        )
            .into_response();
    }
    let user = match resolve_user(&state, &body.user_id).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match reserve_operation(&state, &body, kind).await {
        None => {}
        Some(OpOutcome::Applied { duplicate }) => {
            return (
                StatusCode::OK,
                Json(json!({ "applied": true, "duplicate": duplicate })),
            )
                .into_response()
        }
        Some(OpOutcome::InProgress) => {
            return bridge_error(
                StatusCode::CONFLICT,
                "operation_in_progress",
                "idempotency key is pending on another settlement",
            )
        }
        Some(OpOutcome::Failed { code, status }) => {
            return bridge_error(status, code, code)
        }
    }

    let mut meta = if body.meta.is_object() {
        body.meta.clone()
    } else {
        Value::Object(Default::default())
    };
    if let Value::Object(map) = &mut meta {
        map.entry("request_id".to_string())
            .or_insert_with(|| Value::String(body.idempotency_key.clone()));
    }
    let amount = body.amount_nano_usd as i128;
    let outcome = if kind == "debit" {
        state
            .user_store
            .studio_charge_balance(&user.id, amount, "apeiron_charge", &meta)
            .await
    } else {
        state
            .user_store
            .studio_refund_balance(&user.id, amount, "apeiron_refund", &meta)
            .await
    };
    match outcome {
        Ok(()) => {
            mark_operation(&state, &body, "applied", None).await;
            (
                StatusCode::OK,
                Json(json!({ "applied": true, "duplicate": false })),
            )
                .into_response()
        }
        Err(error) if error == "insufficient_balance" => {
            mark_operation(&state, &body, "failed", Some("insufficient_balance")).await;
            bridge_error(
                StatusCode::CONFLICT,
                "insufficient_balance",
                "wallet balance is insufficient",
            )
        }
        Err(error) => {
            tracing::warn!(%error, "studio bridge settlement failed");
            mark_operation(&state, &body, "failed", Some("internal_error")).await;
            bridge_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "settlement failed",
            )
        }
    }
}

/// SB-9.
pub async fn bridge_debit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SettleBody>,
) -> Response {
    settle(&state, &headers, body, "debit").await
}

/// SB-10.
pub async fn bridge_refund(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SettleBody>,
) -> Response {
    settle(&state, &headers, body, "refund").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_token_roundtrip_and_replay() {
        let secret = "test-secret";
        let token = mint_handoff_token(secret, "user-1");
        let claims = verify_handoff_token(secret, &token).expect("valid token");
        assert_eq!(claims.sub, "user-1");
        // SB-5: the nonce window rejects a second exchange of the same token.
        assert!(matches!(
            verify_handoff_token(secret, &token),
            Err(HandoffError::Replayed)
        ));
    }

    #[test]
    fn handoff_token_rejects_bad_signature_and_version() {
        let secret = "test-secret";
        let token = mint_handoff_token(secret, "user-1");
        let tampered = format!("v2.{}", &token[3..]);
        assert!(matches!(
            verify_handoff_token(secret, &tampered),
            Err(HandoffError::Invalid)
        ));
        assert!(matches!(
            verify_handoff_token("other-secret", &token),
            Err(HandoffError::Invalid)
        ));
    }
}
