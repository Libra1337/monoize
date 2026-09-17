use crate::app::AppState;
use crate::dashboard_handlers::session_helpers::{get_current_user, require_admin};
use crate::error::{AppError, AppResult};
use crate::users::{
    Announcement, AnnouncementStoreError, CreateAnnouncementInput, MarkAnnouncementsReadInput,
    UpdateAnnouncementInput,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Serialize;

#[derive(Debug, serde::Deserialize)]
pub struct AnnouncementsQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AnnouncementsResponse {
    pub announcements: Vec<Announcement>,
    pub unread_count: i64,
}

#[derive(Debug, Serialize)]
pub struct UnreadCountResponse {
    pub unread_count: i64,
}

#[derive(Debug, Serialize)]
pub struct AdminAnnouncementsResponse {
    pub announcements: Vec<Announcement>,
}

fn map_announcement_error(error: AnnouncementStoreError) -> AppError {
    match error {
        AnnouncementStoreError::NotFound => {
            AppError::new(StatusCode::NOT_FOUND, "not_found", "announcement not found")
        }
        AnnouncementStoreError::InvalidTitle => AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_announcement_title",
            "announcement title must be 1-200 characters after trimming",
        ),
        AnnouncementStoreError::InvalidContent => AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_announcement_content",
            "announcement content must be 1-5000 characters after trimming",
        ),
        AnnouncementStoreError::InvalidType => AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_announcement_type",
            "announcement type must be info, success, warning, or error",
        ),
        AnnouncementStoreError::Storage(message) => {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
        }
    }
}

fn internal(message: String) -> AppError {
    AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
}

/// AN-4: enabled announcements with per-user read state and unread count.
pub async fn list_announcements(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AnnouncementsQuery>,
) -> AppResult<Json<AnnouncementsResponse>> {
    let user = get_current_user(&headers, &state).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let result = state
        .user_store
        .list_announcements_for_user(&user.id, limit)
        .await
        .map_err(internal)?;
    Ok(Json(AnnouncementsResponse {
        announcements: result.announcements,
        unread_count: result.unread_count,
    }))
}

/// AN-5: mark announcements read; an empty id list marks every enabled one.
pub async fn mark_announcements_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MarkAnnouncementsReadInput>,
) -> AppResult<Json<UnreadCountResponse>> {
    let user = get_current_user(&headers, &state).await?;
    let unread_count = state
        .user_store
        .mark_announcements_read(&user.id, &body.ids)
        .await
        .map_err(internal)?;
    Ok(Json(UnreadCountResponse { unread_count }))
}

/// AN-6.
pub async fn list_announcements_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<AdminAnnouncementsResponse>> {
    require_admin(&headers, &state).await?;
    let announcements = state
        .user_store
        .list_announcements_admin()
        .await
        .map_err(internal)?;
    Ok(Json(AdminAnnouncementsResponse { announcements }))
}

/// AN-7.
pub async fn create_announcement(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateAnnouncementInput>,
) -> AppResult<(StatusCode, Json<Announcement>)> {
    let admin = require_admin(&headers, &state).await?;
    let announcement = state
        .user_store
        .create_announcement(body, &admin.id)
        .await
        .map_err(map_announcement_error)?;
    Ok((StatusCode::CREATED, Json(announcement)))
}

/// AN-8.
pub async fn update_announcement(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(announcement_id): Path<String>,
    Json(body): Json<UpdateAnnouncementInput>,
) -> AppResult<Json<Announcement>> {
    require_admin(&headers, &state).await?;
    let announcement = state
        .user_store
        .update_announcement(&announcement_id, body)
        .await
        .map_err(map_announcement_error)?;
    Ok(Json(announcement))
}

/// AN-9.
pub async fn delete_announcement(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(announcement_id): Path<String>,
) -> AppResult<StatusCode> {
    require_admin(&headers, &state).await?;
    state
        .user_store
        .delete_announcement(&announcement_id)
        .await
        .map_err(map_announcement_error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[cfg(test)]
mod tests {
    use super::{create_announcement, delete_announcement, list_announcements};
    use crate::app::{AppState, RuntimeConfig, load_state_with_runtime};
    use crate::dashboard_handlers::{
        list_announcements_admin, mark_announcements_read, update_announcement,
    };
    use crate::users::{
        CreateAnnouncementInput, MarkAnnouncementsReadInput, UpdateAnnouncementInput, UserRole,
    };
    use axum::Json;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, HeaderValue};

    async fn make_state() -> AppState {
        load_state_with_runtime(RuntimeConfig {
            listen: "127.0.0.1:0".to_string(),
            metrics_path: "/metrics".to_string(),
            database_dsn: "sqlite::memory:".to_string(),
            request_log_spool_dir: None,
            node: crate::node_config::NodeSettings::primary_default(),
        })
        .await
        .expect("state loads")
    }

    async fn session_headers(state: &AppState, username: &str, role: UserRole) -> HeaderMap {
        let user = state
            .user_store
            .create_user(username, "password123", role, None)
            .await
            .expect("user created");
        let session = state
            .user_store
            .create_session(&user.id, 7)
            .await
            .expect("session created");
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {}", session.token)).expect("header value"),
        );
        headers
    }

    #[tokio::test]
    async fn announcements_require_admin_to_create() {
        let state = make_state().await;
        let user_headers = session_headers(&state, "ann_plain", UserRole::User).await;
        let result = create_announcement(
            State(state.clone()),
            user_headers,
            Json(CreateAnnouncementInput {
                title: "denied".into(),
                content: "no".into(),
                announcement_type: "info".into(),
                pinned: false,
                enabled: true,
            }),
        )
        .await;
        assert!(result.is_err(), "AN-7: a non-admin must not publish");
    }

    #[tokio::test]
    async fn announcement_lifecycle_read_and_unread() {
        let state = make_state().await;
        let admin_headers = session_headers(&state, "ann_admin", UserRole::Admin).await;
        let reader_headers = session_headers(&state, "ann_reader", UserRole::User).await;

        let (status, created) = create_announcement(
            State(state.clone()),
            admin_headers.clone(),
            Json(CreateAnnouncementInput {
                title: "Maintenance window".into(),
                content: "The gateway reboots at 03:00 Beijing time.".into(),
                announcement_type: "warning".into(),
                pinned: true,
                enabled: true,
            }),
        )
        .await
        .expect("AN-7: admin creates");
        assert_eq!(status, axum::http::StatusCode::CREATED);
        assert_eq!(created.0.title, "Maintenance window");
        assert!(created.0.pinned);
        let announcement_id = created.0.id;

        // AN-4: reader sees the announcement unread.
        let listed = list_announcements(
            State(state.clone()),
            reader_headers.clone(),
            axum::extract::Query(super::AnnouncementsQuery { limit: None }),
        )
        .await
        .expect("AN-4: list")
        .0;
        assert_eq!(listed.unread_count, 1);
        assert_eq!(listed.announcements.len(), 1);
        assert_eq!(listed.announcements[0].id, announcement_id);
        assert_eq!(listed.announcements[0].is_read, Some(false));

        // AN-8: update pins off and disables.
        let updated = update_announcement(
            State(state.clone()),
            admin_headers.clone(),
            Path(announcement_id.clone()),
            Json(UpdateAnnouncementInput {
                title: None,
                content: None,
                announcement_type: None,
                pinned: Some(false),
                enabled: Some(false),
            }),
        )
        .await
        .expect("AN-8: update");
        assert!(!updated.pinned);
        assert!(!updated.enabled);

        // Disabled announcements vanish from the user list and unread count.
        let listed = list_announcements(
            State(state.clone()),
            reader_headers.clone(),
            axum::extract::Query(super::AnnouncementsQuery { limit: None }),
        )
        .await
        .expect("AN-4: list")
        .0;
        assert_eq!(listed.unread_count, 0);
        assert!(listed.announcements.is_empty());

        // Re-enable, then AN-5: mark read with an explicit id list.
        let _ = update_announcement(
            State(state.clone()),
            admin_headers.clone(),
            Path(announcement_id.clone()),
            Json(UpdateAnnouncementInput {
                title: None,
                content: None,
                announcement_type: None,
                pinned: None,
                enabled: Some(true),
            }),
        )
        .await
        .expect("AN-8: re-enable");
        let marked = mark_announcements_read(
            State(state.clone()),
            reader_headers.clone(),
            Json(MarkAnnouncementsReadInput {
                ids: vec![announcement_id.clone()],
            }),
        )
        .await
        .expect("AN-5: mark read")
        .0;
        assert_eq!(marked.unread_count, 0);

        // AN-5 idempotence: re-marking keeps the count at zero.
        let marked = mark_announcements_read(
            State(state.clone()),
            reader_headers.clone(),
            Json(MarkAnnouncementsReadInput { ids: vec![] }),
        )
        .await
        .expect("AN-5: mark all")
        .0;
        assert_eq!(marked.unread_count, 0);

        // AN-6: admin list includes disabled detail; AN-9: delete cleans up.
        let admin_list = list_announcements_admin(State(state.clone()), admin_headers.clone())
            .await
            .expect("AN-6: admin list")
            .0;
        assert_eq!(admin_list.announcements.len(), 1);

        delete_announcement(
            State(state.clone()),
            admin_headers.clone(),
            Path(announcement_id.clone()),
        )
        .await
        .expect("AN-9: delete");
        let missing = delete_announcement(
            State(state.clone()),
            admin_headers.clone(),
            Path(announcement_id.clone()),
        )
        .await;
        assert!(missing.is_err(), "AN-9: deleting twice must 404");
    }

    #[tokio::test]
    async fn announcement_validation_rejects_bad_input() {
        let state = make_state().await;
        let admin_headers = session_headers(&state, "ann_valid", UserRole::SuperAdmin).await;

        let too_long_title = "x".repeat(201);
        let result = create_announcement(
            State(state.clone()),
            admin_headers.clone(),
            Json(CreateAnnouncementInput {
                title: too_long_title,
                content: "ok".into(),
                announcement_type: "info".into(),
                pinned: false,
                enabled: true,
            }),
        )
        .await;
        assert!(result.is_err(), "AN-7: title over 200 chars must 400");

        let result = create_announcement(
            State(state.clone()),
            admin_headers,
            Json(CreateAnnouncementInput {
                title: "ok".into(),
                content: "   ".into(),
                announcement_type: "bogus".into(),
                pinned: false,
                enabled: true,
            }),
        )
        .await;
        assert!(result.is_err(), "AN-7: blank content / bad type must 400");
    }
}
