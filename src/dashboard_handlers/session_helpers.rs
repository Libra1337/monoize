use crate::app::AppState;
use crate::error::{AppError, AppResult};
use crate::users::{User, UserStore};
use axum::http::{HeaderMap, StatusCode};

pub(super) fn is_valid_username(username: &str) -> bool {
    (3..=22).contains(&username.len())
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(super) fn is_reserved_internal_username(username: &str) -> bool {
    UserStore::is_reserved_internal_username(username)
}

pub(super) fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| extract_session_from_cookie(headers))
}

fn extract_session_from_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .map(|c| c.trim())
                .find(|c| c.starts_with("monoize_session="))
                .and_then(|c| c.strip_prefix("monoize_session="))
                .filter(|v| !v.is_empty())
                .map(|s| s.to_string())
        })
}

/// Resolves the session user when one is present and usable, and yields `None` otherwise.
///
/// Endpoints that serve both anonymous visitors and signed-in users need the caller's
/// identity without turning a missing or stale cookie into an error. Every failure mode of
/// [`get_current_user`] — absent token, expired session, deleted or disabled user, storage
/// error — collapses to `None` so the caller falls back to its anonymous behaviour.
pub(crate) async fn optional_current_user(headers: &HeaderMap, state: &AppState) -> Option<User> {
    let token = extract_session_token(headers)?;
    let user_store = &state.user_store;
    let session = user_store.get_session_by_token(&token).await.ok()??;
    let user = user_store.get_user_by_id(&session.user_id).await.ok()??;
    user.enabled.then_some(user)
}

pub(crate) async fn get_current_user(headers: &HeaderMap, state: &AppState) -> AppResult<User> {
    let token = extract_session_token(headers).ok_or_else(|| {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing dashboard session",
        )
    })?;

    let user_store = &state.user_store;

    let session = user_store
        .get_session_by_token(&token)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?
        .ok_or_else(|| {
            AppError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "invalid or expired session",
            )
        })?;

    let user = user_store
        .get_user_by_id(&session.user_id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?
        .ok_or_else(|| AppError::new(StatusCode::UNAUTHORIZED, "unauthorized", "user not found"))?;

    if !user.enabled {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "user account is disabled",
        ));
    }

    Ok(user)
}

pub(super) async fn require_admin(headers: &HeaderMap, state: &AppState) -> AppResult<User> {
    let user = get_current_user(headers, state).await?;
    if !user.role.can_manage_users() {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "admin access required",
        ));
    }
    Ok(user)
}
