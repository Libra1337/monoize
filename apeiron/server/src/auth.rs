//! AP-A1..A4: handoff exchange, session minting, mirror user resolution.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::http::HeaderMap;
use serde_json::{json, Value};
use sqlx::Row;

pub const SESSION_COOKIE: &str = "apeiron_session";
const SESSION_TTL_DAYS: i64 = 7;

#[derive(Clone, Debug)]
pub struct MirrorUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub balance_nano_usd: String,
    pub balance_unlimited: bool,
    pub balance_synced_at: String,
}

impl MirrorUser {
    pub fn is_admin(&self) -> bool {
        matches!(self.role.as_str(), "admin" | "super_admin")
    }
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "username": self.username,
            "display_name": self.display_name,
            "role": self.role,
        })
    }
}

pub fn hash_token(token: &str) -> String {
    crate::sha256_hex(token)
}

fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    if let Some(raw) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some(token) = raw.strip_prefix("Bearer ") {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| {
            raw.split(';').find_map(|part| {
                let part = part.trim();
                part.strip_prefix(&format!("{SESSION_COOKIE}="))
                    .map(str::to_string)
            })
        })
}

async fn purge_expired(db: &crate::db::Pool) {
    let now = crate::now_rfc3339();
    let _ = sqlx::query("DELETE FROM sessions WHERE expires_at < $1")
        .bind(&now)
        .execute(db)
        .await;
}

pub async fn current_user(
    state: &SharedState,
    headers: &HeaderMap,
) -> ApiResult<MirrorUser> {
    let Some(token) = extract_session_token(headers) else {
        return Err(ApiError::unauthorized());
    };
    let digest = hash_token(&token);
    purge_expired(&state.db).await;
    let row = sqlx::query(
        "SELECT u.id, u.username, u.display_name, u.role, u.balance_nano_usd, \
         u.balance_unlimited, u.balance_synced_at \
         FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.token_hash = $1 AND s.expires_at >= $2",
    )
    .bind(&digest)
    .bind(crate::now_rfc3339())
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(ApiError::unauthorized)?;
    Ok(MirrorUser {
        id: row.try_get("id").unwrap_or_default(),
        username: row.try_get("username").unwrap_or_default(),
        display_name: row.try_get("display_name").unwrap_or_default(),
        role: row.try_get("role").unwrap_or_else(|_| "user".into()),
        balance_nano_usd: row
            .try_get::<String, _>("balance_nano_usd")
            .unwrap_or_else(|_| "0".into()),
        balance_unlimited: row.try_get::<i64, _>("balance_unlimited").unwrap_or(0) != 0,
        balance_synced_at: row
            .try_get("balance_synced_at")
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into()),
    })
}

pub async fn require_admin(
    state: &SharedState,
    headers: &HeaderMap,
) -> ApiResult<MirrorUser> {
    let user = current_user(state, headers).await?;
    if !user.is_admin() {
        return Err(ApiError::forbidden());
    }
    Ok(user)
}

pub async fn upsert_mirror_user(
    state: &SharedState,
    bridge_user: &crate::bridge::BridgeUser,
) -> Result<(), String> {
    let now = crate::now_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, display_name, role, balance_nano_usd, \
         balance_unlimited, balance_synced_at) VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (id) DO UPDATE SET username = excluded.username, \
         display_name = excluded.display_name, role = excluded.role, \
         balance_nano_usd = excluded.balance_nano_usd, \
         balance_unlimited = excluded.balance_unlimited, \
         balance_synced_at = excluded.balance_synced_at",
    )
    .bind(&bridge_user.user_id)
    .bind(&bridge_user.username)
    .bind(&bridge_user.display_name)
    .bind(&bridge_user.role)
    .bind(&bridge_user.balance_nano_usd)
    .bind(bridge_user.balance_unlimited as i64)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn create_session(
    state: &SharedState,
    user_id: &str,
) -> Result<String, String> {
    let token = format!(
        "apeiron_session_{}",
        uuid::Uuid::new_v4().simple().to_string()
    );
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::days(SESSION_TTL_DAYS);
    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(hash_token(&token))
    .bind(now.to_rfc3339())
    .bind(expires.to_rfc3339())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(token)
}

pub async fn delete_session(
    state: &SharedState,
    headers: &HeaderMap,
) -> Result<(), String> {
    if let Some(token) = extract_session_token(headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(hash_token(&token))
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={};{}",
        SESSION_TTL_DAYS * 86400,
        if secure { " Secure;" } else { "" }
    )
}

pub fn clear_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
}
