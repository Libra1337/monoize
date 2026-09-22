//! HTTP API surface (AP-H1..H4) and router assembly.

pub mod admin;
pub mod assets_api;
pub mod auth_api;
pub mod misc;
pub mod projects;
pub mod runs;
pub mod sse;
pub mod worker;

use crate::state::SharedState;
use axum::routing::{get, post, put};
use axum::Router;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

pub fn router(state: SharedState) -> Router {
    let api = Router::new()
        .route("/healthz", get(misc::healthz))
        // Auth
        .route("/auth/exchange", post(auth_api::exchange))
        .route("/auth/logout", post(auth_api::logout))
        .route("/me", get(auth_api::me))
        // Projects
        .route("/projects", get(projects::list).post(projects::create))
        .route(
            "/projects/{id}",
            get(projects::get_project).delete(projects::delete_project),
        )
        .route("/projects/{id}/graph", put(projects::save_graph))
        .route("/projects/{id}/run", post(projects::start_run))
        // Runs
        .route("/runs/oneclick", post(runs::oneclick))
        .route("/estimate/oneclick", get(runs::estimate_oneclick))
        .route("/runs", get(runs::list))
        .route("/runs/{id}", get(runs::get_run))
        .route("/runs/{id}/cancel", post(runs::cancel_run))
        // Assets
        .route("/assets", get(assets_api::list))
        .route(
            "/assets/{id}",
            axum::routing::delete(assets_api::delete_asset),
        )
        .route("/assets/{id}/content", get(assets_api::content))
        // Catalog
        .route("/templates", get(misc::templates))
        .route("/node-packs", get(misc::node_packs))
        .route("/catalog/nodes", get(misc::node_catalog))
        // SSE
        .route("/events", get(sse::events))
        // Admin
        .route(
            "/admin/providers",
            get(admin::list_providers).post(admin::create_provider),
        )
        .route(
            "/admin/providers/{id}",
            put(admin::update_provider).delete(admin::delete_provider),
        )
        .route("/admin/settings", get(admin::get_settings).put(admin::put_settings))
        .route("/admin/runs", get(admin::list_runs))
        .route("/admin/runs/{id}/cancel", post(admin::cancel_run))
        .route(
            "/admin/templates",
            get(admin::list_templates_admin).post(admin::create_template),
        )
        .route(
            "/admin/templates/{id}",
            put(admin::update_template).delete(admin::delete_template),
        )
        // Worker namespace
        .route("/worker/claim", post(worker::claim))
        .route("/worker/jobs/{id}/heartbeat", post(worker::heartbeat))
        .route("/worker/jobs/{id}/result", post(worker::result))
        .route("/worker/jobs/{id}/fail", post(worker::fail))
        .route("/worker/files/{asset_id}", get(worker::file));

    Router::new()
        .nest("/api", api)
        .fallback(get(crate::frontend::serve_frontend))
        .layer(RequestBodyLimitLayer::new(250 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
