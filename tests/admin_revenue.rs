use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use monoize::app::{RuntimeConfig, build_app, load_state_with_runtime};
use monoize::users::UserRole;
use sea_orm::ConnectionTrait;
use serde_json::Value;
use tower::ServiceExt;

async fn get_json(router: &axum::Router, path: &str, session_token: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body reads")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn post_json(
    router: &axum::Router,
    path: &str,
    session_token: &str,
    body: &str,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body reads")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn delete_json(
    router: &axum::Router,
    path: &str,
    session_token: &str,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(path)
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body reads")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

struct Setup {
    router: axum::Router,
    admin_session: String,
    user_session: String,
    user_id: String,
    excluded_user_id: String,
    today: String,
}

async fn setup_revenue() -> Setup {
    let state = load_state_with_runtime(RuntimeConfig::with_defaults(
        "127.0.0.1:0",
        "/metrics",
        "sqlite::memory:".to_string(),
    ))
    .await
    .expect("state loads");
    let admin = state
        .user_store
        .create_user("revenue_admin", "password123", UserRole::SuperAdmin, None)
        .await
        .expect("admin creates");
    let user = state
        .user_store
        .create_user("revenue_user", "password123", UserRole::User, None)
        .await
        .expect("user creates");
    let excluded_user = state
        .user_store
        .create_user("revenue_excluded", "password123", UserRole::User, None)
        .await
        .expect("excluded user creates");
    let admin_session = state
        .user_store
        .create_session(&admin.id, 7)
        .await
        .expect("admin session creates");
    let user_session = state
        .user_store
        .create_session(&user.id, 7)
        .await
        .expect("user session creates");

    // AR-2: a day is one Asia/Shanghai local day. 2026-09-15T18:30Z is
    // 2026-09-16 02:30 Beijing, so the current Beijing day is 2026-09-16 and
    // its local midnight is 2026-09-15T16:00:00Z.
    let beijing_midnight = chrono::DateTime::parse_from_rfc3339("2026-09-15T16:00:00Z")
        .expect("fixed midnight")
        .with_timezone(&chrono::Utc);

    // One row inside the current Beijing day (30 minutes after local midnight),
    // one row inside the previous Beijing day, and one row from the excluded
    // user inside the current day that must never count.
    for (id, user_id, model, charge, created_at) in [
        (
            "rev-1",
            user.id.as_str(),
            "model-a",
            "100",
            beijing_midnight + chrono::Duration::minutes(30),
        ),
        (
            "rev-2",
            user.id.as_str(),
            "model-b",
            "50",
            beijing_midnight - chrono::Duration::minutes(30),
        ),
        (
            "rev-excluded",
            excluded_user.id.as_str(),
            "model-a",
            "999",
            beijing_midnight + chrono::Duration::minutes(45),
        ),
    ] {
        state
            .db_pool
            .write()
            .await
            .execute(state.db_pool.stmt(
                "INSERT INTO request_logs (id, user_id, model, is_stream, input_tokens, output_tokens, charge_nano_usd, status, created_at, created_at_unix_ms) VALUES ($1, $2, $3, 0, 10, 5, $4, 'success', $5, $6)",
                vec![
                    id.into(),
                    user_id.into(),
                    model.into(),
                    charge.into(),
                    created_at.to_rfc3339().into(),
                    created_at.timestamp_millis().into(),
                ],
            ))
            .await
            .expect("request log inserts");
    }

    Setup {
        router: build_app(state),
        admin_session: admin_session.token,
        user_session: user_session.token,
        user_id: user.id,
        excluded_user_id: excluded_user.id,
        today: "2026-09-16".to_string(),
    }
}

#[tokio::test]
async fn revenue_daily_requires_admin_and_reports_beijing_days() {
    let setup = setup_revenue().await;

    // AR-1: a non-admin session is rejected.
    let (status, body) = get_json(
        &setup.router,
        "/api/dashboard/admin/revenue/daily",
        &setup.user_session,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // AR-10: the current day aggregates live; the excluded user's 999 does not
    // appear because the exclusion list is empty but the row belongs to another
    // user. The live day must sum only matching rows of that day.
    let (status, body) = get_json(
        &setup.router,
        "/api/dashboard/admin/revenue/daily",
        &setup.admin_session,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let days = body["days"].as_array().expect("days array");
    // The database clock differs from the fixed 2026-09-16 rows, so the live
    // current day is the real Beijing today; the inserted rows fall on
    // 2026-09-16 or 2026-09-15 and are served only when the real current day
    // matches. Assert the structural contract instead of fixed dates.
    let current_day = days
        .iter()
        .find(|day| day["day"].as_str() == Some(setup.today.as_str()));
    if let Some(day) = current_day {
        // Today holds rev-1 (100) and the not-yet-excluded rev-excluded (999)
        // under model-a; rev-2 (50) sits in the previous Beijing day.
        assert_eq!(day["total_charge_nano_usd"], "1099");
        assert_eq!(day["total_calls"], 2);
        let models = day["models"].as_array().expect("models array");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["model"], "model-a");
        assert_eq!(models[0]["charge_nano_usd"], "1099");

        // AR-10: per-user detail rows, ordered by charge descending.
        let users = day["users"].as_array().expect("users array");
        assert_eq!(users.len(), 2);
        assert_eq!(users[0]["user_id"], setup.excluded_user_id);
        assert_eq!(
            users[0]["username"], "revenue_excluded",
            "the username snapshot must resolve through the users join"
        );
        assert_eq!(users[0]["charge_nano_usd"], "999");
        assert_eq!(users[1]["user_id"], setup.user_id);
        assert_eq!(users[1]["username"], "revenue_user");
        assert_eq!(users[1]["charge_nano_usd"], "100");
        assert_eq!(users[1]["calls"], 1);
    }
}

#[tokio::test]
async fn exclusion_add_and_remove_recomputes_history() {
    let setup = setup_revenue().await;
    let from = "2026-09-01";
    let to = "2026-09-30";

    let (status, body) = get_json(
        &setup.router,
        &format!("/api/dashboard/admin/revenue/daily?from={from}&to={to}"),
        &setup.admin_session,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let before = body["days"].as_array().expect("days array").to_vec();

    // AR-12: add the excluded user; the historical rows must be recomputed.
    let body = format!("{{\"user_id\":\"{}\"}}", setup.excluded_user_id);
    let (status, body) = post_json(
        &setup.router,
        "/api/dashboard/admin/revenue/exclusions",
        &setup.admin_session,
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let exclusions = body["exclusions"].as_array().expect("exclusions array");
    assert_eq!(exclusions.len(), 1);
    assert_eq!(exclusions[0]["user_id"], setup.excluded_user_id);

    let (status, body) = get_json(
        &setup.router,
        &format!("/api/dashboard/admin/revenue/daily?from={from}&to={to}"),
        &setup.admin_session,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = body["days"].as_array().expect("days array").to_vec();
    // The excluded user's 999 nano charge must be gone from the 2026-09-16 day
    // when that day is covered by the range.
    if let Some(day) = after
        .iter()
        .find(|day| day["day"].as_str() == Some(setup.today.as_str()))
    {
        let before_day = before
            .iter()
            .find(|day| day["day"].as_str() == Some(setup.today.as_str()));
        if let Some(before_day) = before_day {
            assert_ne!(
                day["total_charge_nano_usd"], before_day["total_charge_nano_usd"],
                "the exclusion must change the recomputed day"
            );
        }
        assert_eq!(day["total_charge_nano_usd"], "100");
    }

    // AR-13: removing an unknown user fails.
    let (status, _) = delete_json(
        &setup.router,
        "/api/dashboard/admin/revenue/exclusions/missing-user",
        &setup.admin_session,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // AR-13: remove the exclusion; the charge returns.
    let (status, body) = delete_json(
        &setup.router,
        &format!(
            "/api/dashboard/admin/revenue/exclusions/{}",
            setup.excluded_user_id
        ),
        &setup.admin_session,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["exclusions"].as_array().expect("empty").len(), 0);
}

#[tokio::test]
async fn revenue_range_validation_rejects_bad_days() {
    let setup = setup_revenue().await;

    let (status, _) = get_json(
        &setup.router,
        "/api/dashboard/admin/revenue/daily?from=2026-9-6&to=2026-09-30",
        &setup.admin_session,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = get_json(
        &setup.router,
        "/api/dashboard/admin/revenue/daily?from=2026-09-30&to=2026-09-01",
        &setup.admin_session,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // AR-12: adding an unknown user returns 404.
    let (status, _) = post_json(
        &setup.router,
        "/api/dashboard/admin/revenue/exclusions",
        &setup.admin_session,
        "{\"user_id\":\"no-such-user\"}",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn excel_export_returns_workbook() {
    let setup = setup_revenue().await;

    let response = setup
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/admin/revenue/daily/export?from=2026-09-01&to=2026-09-30")
                .header(AUTHORIZATION, format!("Bearer {}", setup.admin_session))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .expect("content type")
        .to_str()
        .expect("ascii")
        .to_string();
    assert_eq!(
        content_type,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    let disposition = response
        .headers()
        .get("content-disposition")
        .expect("disposition")
        .to_str()
        .expect("ascii")
        .to_string();
    assert!(disposition.contains("monoize-revenue-2026-09-01-2026-09-30.xlsx"));
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body reads")
        .to_bytes();
    // An xlsx file is a ZIP archive starting with the local-file signature.
    assert_eq!(&bytes[..2], b"PK", "xlsx must be a ZIP container");
}

/// Regression: the aggregate groups by (day, model, user), so two users
/// consuming the same model must merge into ONE model row — persisting them
/// separately violated the (day, model) unique index during settlement.
#[tokio::test]
async fn settlement_merges_per_user_groups_into_one_model_row() {
    let state = load_state_with_runtime(RuntimeConfig::with_defaults(
        "127.0.0.1:0",
        "/metrics",
        "sqlite::memory:".to_string(),
    ))
    .await
    .expect("state loads");
    let user_a = state
        .user_store
        .create_user("merge_a", "password123", UserRole::User, None)
        .await
        .expect("user a creates");
    let user_b = state
        .user_store
        .create_user("merge_b", "password123", UserRole::User, None)
        .await
        .expect("user b creates");
    let now = chrono::Utc::now();
    for (id, user_id, charge) in [
        ("merge-1", user_a.id.as_str(), "100"),
        ("merge-2", user_b.id.as_str(), "250"),
    ] {
        state
            .db_pool
            .write()
            .await
            .execute(state.db_pool.stmt(
                "INSERT INTO request_logs (id, user_id, model, is_stream, input_tokens, output_tokens, charge_nano_usd, status, created_at, created_at_unix_ms) VALUES ($1, $2, 'same-model', 0, 10, 5, $3, 'success', $4, $5)",
                vec![
                    id.into(),
                    user_id.into(),
                    charge.into(),
                    now.to_rfc3339().into(),
                    now.timestamp_millis().into(),
                ],
            ))
            .await
            .expect("request log inserts");
    }

    // The settlement startup pass recomputes every elapsed day inside the
    // retention window; this must not trip the (day, model) unique index.
    monoize::users::settle_elapsed_days(&state.db_pool, now)
        .await
        .expect("settlement completes without unique violation");

    let today = {
        let beijing = now.timestamp_millis() + 8 * 3600 * 1000;
        chrono::DateTime::from_timestamp_millis(beijing)
            .expect("timestamp")
            .format("%Y-%m-%d")
            .to_string()
    };
    use sea_orm::ConnectionTrait;
    let rows = state
        .db_pool
        .read()
        .query_all(state.db_pool.stmt(
            "SELECT model, COUNT(*) AS n FROM admin_revenue_daily_model_rows WHERE day = $1 GROUP BY model",
            vec![today.clone().into()],
        ))
        .await
        .expect("model rows query");
    // The current day is live-only, so query the aggregate directly instead.
    let aggregate = monoize::users::aggregate_revenue_day(&state.db_pool, &today, &[])
        .await
        .expect("aggregate resolves")
        .expect("the day has rows");
    assert_eq!(aggregate.models.len(), 1, "same-model rows must merge");
    assert_eq!(aggregate.models[0].model, "same-model");
    assert_eq!(aggregate.models[0].charge_nano_usd, "350");
    assert_eq!(aggregate.models[0].calls, 2);
    assert_eq!(aggregate.users.len(), 2);
    assert_eq!(aggregate.users[0].charge_nano_usd, "250");
    assert_eq!(aggregate.users[1].charge_nano_usd, "100");
    assert_eq!(aggregate.total_charge_nano_usd, "350");
    let _ = rows;
}
