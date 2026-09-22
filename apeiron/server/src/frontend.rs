//! Embedded SPA serving (same convention as the platform frontend).

use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
#[cfg(embed_frontend)]
use include_dir::{include_dir, Dir};

#[cfg(embed_frontend)]
static FRONTEND_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../web/dist");

#[cfg(embed_frontend)]
pub async fn serve_frontend(uri: Uri, _headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = FRONTEND_DIR.get_file(path) {
        let mime = match path.rsplit('.').next().unwrap_or("") {
            "js" => "text/javascript",
            "css" => "text/css",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "woff2" => "font/woff2",
            "json" => "application/json",
            "html" => "text/html",
            _ => "application/octet-stream",
        };
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return (
            StatusCode::OK,
            [
                ("content-type", mime),
                ("cache-control", cache),
            ],
            file.contents(),
        )
            .into_response();
    }
    // SPA history fallback.
    match FRONTEND_DIR.get_file("index.html") {
        Some(index) => (
            StatusCode::OK,
            [
                ("content-type", "text/html"),
                ("cache-control", "no-store"),
            ],
            index.contents(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "frontend not embedded").into_response(),
    }
}

#[cfg(not(embed_frontend))]
pub async fn serve_frontend(_uri: Uri, _headers: HeaderMap) -> Response {
    (
        StatusCode::NOT_FOUND,
        "frontend not embedded; use the Vite dev server (cd apeiron/web && bun run dev)",
    )
        .into_response()
}
