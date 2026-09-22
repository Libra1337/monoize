//! AP-H1 / AP-X1: asset listing, streamed content, deletion.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub struct ListQuery {
    pub kind: Option<String>,
}

pub async fn list(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let assets = crate::assets::list_assets(&state, &user.id, query.kind.as_deref())
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(json!({
        "assets": assets.iter().map(|asset| asset.to_json()).collect::<Vec<_>>(),
    })))
}

pub async fn content(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let asset = crate::assets::get_asset(&state, &id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("asset"))?;
    if asset.user_id != user.id && !user.is_admin() {
        return Err(ApiError::not_found("asset"));
    }
    let path = std::path::Path::new(&state.cfg.assets_dir).join(&asset.file_path);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ApiError::internal(format!("open asset: {e}")))?;
    let stream = tokio_util::io::ReaderStream::new(file);
    Ok((
        StatusCode::OK,
        [
            ("content-type", asset.mime_type),
            (
                "cache-control",
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

pub async fn delete_asset(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let asset = crate::assets::get_asset(&state, &id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("asset"))?;
    if asset.user_id != user.id {
        return Err(ApiError::not_found("asset"));
    }
    let outcome = sqlx::query("DELETE FROM assets WHERE id = $1 AND user_id = $2")
        .bind(&id)
        .bind(&user.id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Err(ApiError::not_found("asset"));
    }
    let path = std::path::Path::new(&state.cfg.assets_dir).join(&asset.file_path);
    let _ = tokio::fs::remove_file(&path).await;
    Ok(Json(json!({ "ok": true })))
}
