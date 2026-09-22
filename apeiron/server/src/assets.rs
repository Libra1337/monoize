//! AP-X1: asset persistence and per-user scoped streaming.

use crate::state::SharedState;
use serde_json::{json, Value};
use sqlx::Row;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AssetRow {
    pub id: String,
    pub user_id: String,
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    pub kind: String,
    pub file_path: String,
    pub mime_type: String,
    pub bytes: i64,
    pub meta: Value,
    pub created_at: String,
}

impl AssetRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "run_id": self.run_id,
            "step_id": self.step_id,
            "kind": self.kind,
            "mime_type": self.mime_type,
            "bytes": self.bytes,
            "meta": self.meta,
            "created_at": self.created_at,
        })
    }
}

fn extension_for(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" => "wav",
        "application/x-subrip" | "text/srt" => "srt",
        _ => "bin",
    }
}

pub async fn store_asset(
    state: &SharedState,
    user_id: &str,
    run_id: Option<&str>,
    step_id: Option<&str>,
    kind: &str,
    mime: &str,
    bytes: &[u8],
    meta: Value,
) -> Result<AssetRow, String> {
    if bytes.is_empty() {
        return Err("asset payload is empty".into());
    }
    if bytes.len() as u64 > crate::ASSET_MAX_BYTES {
        return Err("asset exceeds the storage cap".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let folder = format!("{}/{:02}", now.format("%Y"), now.format("%m"));
    let file_name = format!("{id}.{}", extension_for(mime));
    let relative = format!("{folder}/{file_name}");
    let full = PathBuf::from(&state.cfg.assets_dir).join(&relative);
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create asset dir: {e}"))?;
    }
    tokio::fs::write(&full, bytes)
        .await
        .map_err(|e| format!("write asset: {e}"))?;
    sqlx::query(
        "INSERT INTO assets (id, user_id, run_id, step_id, kind, file_path, mime_type, \
         bytes, meta_json, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(run_id)
    .bind(step_id)
    .bind(kind)
    .bind(&relative)
    .bind(mime)
    .bind(bytes.len() as i64)
    .bind(meta.to_string())
    .bind(crate::now_rfc3339())
    .execute(&state.db)
    .await
    .map_err(|e| format!("insert asset row: {e}"))?;
    Ok(AssetRow {
        id,
        user_id: user_id.to_string(),
        run_id: run_id.map(str::to_string),
        step_id: step_id.map(str::to_string),
        kind: kind.to_string(),
        file_path: relative,
        mime_type: mime.to_string(),
        bytes: bytes.len() as i64,
        meta,
        created_at: crate::now_rfc3339(),
    })
}

/// Stream an upstream media response to disk under the byte cap.
pub async fn store_asset_from_response(
    state: &SharedState,
    user_id: &str,
    run_id: Option<&str>,
    step_id: Option<&str>,
    kind: &str,
    fallback_mime: &str,
    response: reqwest::Response,
    meta: Value,
) -> Result<AssetRow, String> {
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|raw| raw.split(';').next().unwrap_or("").trim().to_string())
        .filter(|mime| !mime.is_empty())
        .unwrap_or_else(|| fallback_mime.to_string());
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    let mut buffer = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("read media chunk: {e}"))?;
        if (buffer.len() + chunk.len()) as u64 > crate::ASSET_MAX_BYTES {
            return Err("media stream exceeded the asset cap".into());
        }
        buffer.extend_from_slice(&chunk);
    }
    store_asset(state, user_id, run_id, step_id, kind, &mime, &buffer, meta).await
}

pub async fn get_asset(state: &SharedState, id: &str) -> Result<Option<AssetRow>, String> {
    let row = sqlx::query(
        "SELECT id, user_id, run_id, step_id, kind, file_path, mime_type, bytes, meta_json, \
         created_at FROM assets WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|row| AssetRow {
        id: row.try_get("id").unwrap_or_default(),
        user_id: row.try_get("user_id").unwrap_or_default(),
        run_id: row.try_get("run_id").ok().flatten(),
        step_id: row.try_get("step_id").ok().flatten(),
        kind: row.try_get("kind").unwrap_or_default(),
        file_path: row.try_get("file_path").unwrap_or_default(),
        mime_type: row.try_get("mime_type").unwrap_or_default(),
        bytes: row.try_get("bytes").unwrap_or(0),
        meta: row
            .try_get::<String, _>("meta_json")
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| json!({})),
        created_at: row.try_get("created_at").unwrap_or_default(),
    }))
}

pub async fn list_assets(
    state: &SharedState,
    user_id: &str,
    kind: Option<&str>,
) -> Result<Vec<AssetRow>, String> {
    let rows = match kind {
        Some(kind) => {
            sqlx::query(
                "SELECT id, user_id, run_id, step_id, kind, file_path, mime_type, bytes, \
                 meta_json, created_at FROM assets WHERE user_id = $1 AND kind = $2 \
                 ORDER BY created_at DESC LIMIT 500",
            )
            .bind(user_id)
            .bind(kind)
            .fetch_all(&state.db)
            .await
        }
        None => {
            sqlx::query(
                "SELECT id, user_id, run_id, step_id, kind, file_path, mime_type, bytes, \
                 meta_json, created_at FROM assets WHERE user_id = $1 \
                 ORDER BY created_at DESC LIMIT 500",
            )
            .bind(user_id)
            .fetch_all(&state.db)
            .await
        }
    }
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| AssetRow {
            id: row.try_get("id").unwrap_or_default(),
            user_id: row.try_get("user_id").unwrap_or_default(),
            run_id: row.try_get("run_id").ok().flatten(),
            step_id: row.try_get("step_id").ok().flatten(),
            kind: row.try_get("kind").unwrap_or_default(),
            file_path: row.try_get("file_path").unwrap_or_default(),
            mime_type: row.try_get("mime_type").unwrap_or_default(),
            bytes: row.try_get("bytes").unwrap_or(0),
            meta: row
                .try_get::<String, _>("meta_json")
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_else(|| json!({})),
            created_at: row.try_get("created_at").unwrap_or_default(),
        })
        .collect())
}

pub async fn asset_absolute_path(
    state: &SharedState,
    asset: &AssetRow,
) -> PathBuf {
    PathBuf::from(&state.cfg.assets_dir).join(&asset.file_path)
}

pub async fn read_asset_bytes(state: &SharedState, asset: &AssetRow) -> Result<Vec<u8>, String> {
    let path = asset_absolute_path(state, asset).await;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("read asset {}: {e}", asset.id))?;
    Ok(bytes)
}
