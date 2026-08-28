use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use http_body_util::BodyExt;
use monoize::app::{AppState, RuntimeConfig, build_app, load_state_with_runtime};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::node_config::{NodeRole, NodeSettings};
use monoize::replica::admission_http::ADMISSION_ISSUE_PATH;
use monoize::store_billing::availability::{StorePrimaryLease, StorePrimaryLeaseError};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

const REPLICA_TOKEN: &str = "store-availability-token-with-at-least-32-bytes";
const REPLICA_ID: &str = "6f8b7f54-1833-4dc8-9e93-24482e870c22";

fn instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap()
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

async fn database() -> DbPool {
    let db = DbPool::connect("sqlite::memory:").await.expect("database");
    Migrator::up(&*db.write().await, None)
        .await
        .expect("migrations");
    db
}

async fn stored_lease(db: &DbPool) -> (String, i64, String) {
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT owner_id, epoch, expires_at FROM store_primary_leases
             WHERE name = 'store_primary'",
            vec![],
        ))
        .await
        .expect("lease query")
        .expect("lease row");
    (
        row.try_get("", "owner_id").expect("owner ID"),
        row.try_get("", "epoch").expect("epoch"),
        row.try_get("", "expires_at").expect("expiry"),
    )
}

#[tokio::test]
async fn first_acquisition_creates_epoch_one_for_exactly_fifteen_seconds() {
    let db = database().await;
    let now = instant();

    let lease = StorePrimaryLease::acquire_at(db.clone(), "owner-a", now)
        .await
        .expect("first acquisition");

    assert_eq!(lease.owner_id(), "owner-a");
    assert_eq!(lease.epoch(), 1);
    assert_eq!(
        stored_lease(&db).await,
        (
            "owner-a".to_string(),
            1,
            timestamp(now + Duration::seconds(15))
        )
    );
}

#[tokio::test]
async fn same_owner_renewal_preserves_epoch() {
    let db = database().await;
    let now = instant();
    let lease = StorePrimaryLease::acquire_at(db.clone(), "owner-a", now)
        .await
        .unwrap();

    lease
        .renew_at(now + Duration::seconds(5))
        .await
        .expect("renewal");

    assert_eq!(lease.epoch(), 1);
    assert_eq!(
        stored_lease(&db).await,
        (
            "owner-a".to_string(),
            1,
            timestamp(now + Duration::seconds(20))
        )
    );
}

#[tokio::test]
async fn competing_owner_is_blocked_before_expiry() {
    let db = database().await;
    let now = instant();
    StorePrimaryLease::acquire_at(db.clone(), "owner-a", now)
        .await
        .unwrap();

    let error = StorePrimaryLease::acquire_at(db.clone(), "owner-b", now + Duration::seconds(14))
        .await
        .unwrap_err();

    assert_eq!(error, StorePrimaryLeaseError::Unavailable);
    assert_eq!(stored_lease(&db).await.0, "owner-a");
}

#[tokio::test]
async fn takeover_after_expiry_increments_epoch() {
    let db = database().await;
    let now = instant();
    StorePrimaryLease::acquire_at(db.clone(), "owner-a", now)
        .await
        .unwrap();

    let replacement =
        StorePrimaryLease::acquire_at(db.clone(), "owner-b", now + Duration::seconds(15))
            .await
            .expect("takeover");

    assert_eq!(replacement.epoch(), 2);
    assert_eq!(stored_lease(&db).await.0, "owner-b");
}

#[tokio::test]
async fn stale_epoch_is_rejected_after_takeover() {
    let db = database().await;
    let now = instant();
    let stale = StorePrimaryLease::acquire_at(db.clone(), "owner-a", now)
        .await
        .unwrap();
    StorePrimaryLease::acquire_at(db, "owner-b", now + Duration::seconds(15))
        .await
        .unwrap();

    assert_eq!(
        stale
            .validate_at(now + Duration::seconds(15))
            .await
            .unwrap_err(),
        StorePrimaryLeaseError::EpochMismatch
    );
}

#[tokio::test]
async fn validation_rejects_wrong_owner() {
    let db = database().await;
    let now = instant();
    let lease = StorePrimaryLease::acquire_at(db.clone(), "owner-a", now)
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_primary_leases SET owner_id = 'owner-b'
             WHERE name = 'store_primary'",
            vec![],
        ))
        .await
        .unwrap();

    assert_eq!(
        lease.validate_at(now).await.unwrap_err(),
        StorePrimaryLeaseError::OwnerMismatch
    );
}

#[tokio::test]
async fn validation_rejects_expiry() {
    let db = database().await;
    let now = instant();
    let lease = StorePrimaryLease::acquire_at(db, "owner-a", now)
        .await
        .unwrap();

    assert_eq!(
        lease
            .validate_at(now + Duration::seconds(15))
            .await
            .unwrap_err(),
        StorePrimaryLeaseError::Expired
    );
}

#[tokio::test]
async fn takeover_rejects_signed_epoch_overflow_without_changing_the_row() {
    let db = database().await;
    let now = instant();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_primary_leases
                (name, owner_id, epoch, expires_at, updated_at)
             VALUES ('store_primary', 'owner-a', $1, $2, $2)",
            vec![
                i64::MAX.into(),
                timestamp(now - Duration::seconds(1)).into(),
            ],
        ))
        .await
        .unwrap();

    let error = StorePrimaryLease::acquire_at(db.clone(), "owner-b", now)
        .await
        .unwrap_err();

    assert_eq!(error, StorePrimaryLeaseError::EpochOverflow);
    assert_eq!(stored_lease(&db).await.1, i64::MAX);
    assert_eq!(stored_lease(&db).await.0, "owner-a");
}

async fn primary_state(token: Option<&str>) -> (TempDir, AppState) {
    let temp = TempDir::new().expect("temporary directory");
    let mut node = NodeSettings::primary_default();
    node.replica_token = token.map(str::to_string);
    let state = load_state_with_runtime(RuntimeConfig {
        listen: "127.0.0.1:0".to_string(),
        metrics_path: "/metrics".to_string(),
        database_dsn: format!("sqlite://{}", temp.path().join("monoize.db").display()),
        request_log_spool_dir: None,
        node,
    })
    .await
    .expect("primary state");
    assert!(state.store_primary_lease.is_some());
    (temp, state)
}

#[tokio::test]
async fn app_builder_can_explicitly_acquire_a_real_primary_lease() {
    let (_temp, state) = primary_state(None).await;
    let mut state = state
        .with_node_role(NodeRole::Replica)
        .with_node_role(NodeRole::Primary);
    state
        .db_pool
        .write()
        .await
        .execute_unprepared("DELETE FROM store_primary_leases WHERE name = 'store_primary'")
        .await
        .unwrap();

    state
        .acquire_store_primary_lease("fixture-owner")
        .await
        .expect("explicit fixture lease");

    assert_eq!(
        state
            .store_primary_lease
            .as_ref()
            .expect("lease handle")
            .owner_id(),
        "fixture-owner"
    );
    state.validate_store_primary_lease().await.unwrap();
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("JSON response")
}

async fn assert_error(response: axum::response::Response, code: &str) {
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response_json(response).await["error"]["code"], code);
}

#[tokio::test]
async fn store_mutation_middleware_fails_closed_when_lease_is_absent() {
    let (_temp, state) = primary_state(None).await;
    state
        .db_pool
        .write()
        .await
        .execute_unprepared("DELETE FROM store_primary_leases WHERE name = 'store_primary'")
        .await
        .unwrap();
    let app = build_app(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/dashboard/store/orders")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_error(response, "store_primary_unavailable").await;
}

#[tokio::test]
async fn payment_callback_fails_closed_when_lease_is_expired() {
    let (_temp, state) = primary_state(None).await;
    state
        .db_pool
        .write()
        .await
        .execute(state.db_pool.stmt(
            "UPDATE store_primary_leases SET expires_at = $1
             WHERE name = 'store_primary'",
            vec![timestamp(Utc::now() - Duration::seconds(1)).into()],
        ))
        .await
        .unwrap();
    let app = build_app(state);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/store/callbacks/missing")
                .body(Body::from("invalid callback"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_error(response, "store_primary_unavailable").await;
}

#[tokio::test]
async fn internal_plan_admission_fails_closed_after_lease_takeover() {
    let (_temp, state) = primary_state(Some(REPLICA_TOKEN)).await;
    let current_epoch = state.store_primary_lease.as_ref().expect("lease").epoch();
    state
        .db_pool
        .write()
        .await
        .execute(state.db_pool.stmt(
            "UPDATE store_primary_leases
             SET owner_id = 'replacement-owner', epoch = $1, expires_at = $2, updated_at = $3
             WHERE name = 'store_primary'",
            vec![
                current_epoch.checked_add(1).unwrap().into(),
                timestamp(Utc::now() + Duration::seconds(15)).into(),
                timestamp(Utc::now()).into(),
            ],
        ))
        .await
        .unwrap();
    let app = build_app(state);
    let body = json!({
        "audience": REPLICA_ID,
        "user_id": "user-without-plan",
        "request_id": "request-1",
        "effective_groups": ["default"],
        "maximum_nano_usd": "1000000",
        "pricing_revision": "pricing-1"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(ADMISSION_ISSUE_PATH)
                .header(AUTHORIZATION, format!("Bearer {REPLICA_TOKEN}"))
                .header("X-Monoize-Replica-ID", REPLICA_ID)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_error(response, "store_primary_unavailable").await;
}

#[tokio::test]
async fn replica_store_mutation_keeps_store_write_rejected_code() {
    let (_temp, state) = primary_state(None).await;
    let app = build_app(state.with_node_role(NodeRole::Replica));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/dashboard/store/orders")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_error(response, "store_write_rejected").await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/dashboard/store/orders")
                .header("cookie", "monoize_session=replica-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_error(response, "store_write_rejected").await;
}
