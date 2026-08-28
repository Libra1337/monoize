use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, ORIGIN};
use axum::http::{Method, Request, StatusCode};
use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use http_body_util::BodyExt;
use monoize::app::{RuntimeConfig, build_app, load_state_with_runtime};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::node_config::NodeSettings;
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentOrderInput, PaymentOrderError, PaymentOrderStore,
};
use monoize::store_billing::reauth::ReauthStore;
use monoize::store_billing::retention::{
    CreateStoreLegalHoldInput, CreateStoreRetentionContainmentInput, RetentionRunActor,
    StoreRetention, StoreRetentionDataClass, StoreRetentionError, StoreRetentionRunState,
};
use sea_orm::{ConnectionTrait, QueryResult};
use sea_orm_migration::MigratorTrait;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

fn instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 28, 12, 0, 0).unwrap()
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

async fn insert_user(db: &DbPool, id: &str) {
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO users
                (id, username, password_hash, role, created_at, updated_at, enabled,
                 balance_nano_usd, balance_unlimited, group_id)
             SELECT $1, $1, 'test', 'admin', $2, $2, 1, '0', 0, id
             FROM monoize_groups WHERE is_default = 1 LIMIT 1",
            vec![id.into(), timestamp(instant()).into()],
        ))
        .await
        .expect("user");
}

async fn insert_policy(db: &DbPool, now: DateTime<Utc>) {
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('privacy-1', '2026-08', 'test', '[\"test\"]', $1, 'test',
                     'reviewer', $2, $3, $4, 1)",
            vec![
                serde_json::json!({
                    "raw_callback_days": 30,
                    "network_metadata_days": 90,
                    "financial_records_days": 2555,
                    "redemption_audit_days": 730,
                    "expired_reauth_grant_hours": 24
                })
                .to_string()
                .into(),
                "a".repeat(64).into(),
                timestamp(now - Duration::days(1)).into(),
                timestamp(now + Duration::days(365)).into(),
            ],
        ))
        .await
        .expect("privacy record");
}

async fn scalar(db: &DbPool, sql: &str) -> i64 {
    db.read()
        .query_one(db.stmt(sql, vec![]))
        .await
        .expect("query")
        .expect("row")
        .try_get("", "value")
        .expect("value")
}

async fn row(db: &DbPool, sql: &str) -> QueryResult {
    db.read()
        .query_one(db.stmt(sql, vec![]))
        .await
        .expect("query")
        .expect("row")
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

fn hold_input(
    data_class: StoreRetentionDataClass,
    identifiers: Vec<String>,
    expires_at: DateTime<Utc>,
    extends_hold_id: Option<String>,
) -> CreateStoreLegalHoldInput {
    CreateStoreLegalHoldInput {
        data_class,
        identifiers,
        reason: "preserve evidence".to_string(),
        requesting_authority: "Privacy Office".to_string(),
        requester_id: "requester".to_string(),
        approver_role: "privacy".to_string(),
        expires_at,
        extends_hold_id,
    }
}

#[tokio::test]
async fn raw_callback_deletion_is_bounded_and_legal_hold_expiry_is_automatic() {
    let db = database().await;
    let now = instant();
    insert_policy(&db, now).await;
    for index in 0..=500 {
        db.write()
            .await
            .execute(db.stmt(
                "INSERT INTO store_provider_events
                    (id, credential_version_id, provider_event_id, event_kind, body_digest,
                     parsed_json, verification_result, raw_format_version, raw_key_id,
                     raw_nonce_base64, raw_ciphertext_base64, source_ip, user_agent,
                     projection_state, state_revision, received_at)
                 VALUES ($1, 'credential', $1, 'payment', $2, '{}', 'valid', 1, 'key',
                         'nonce', 'ciphertext', NULL, NULL, 'applied', 0, $3)",
                vec![
                    format!("event-{index:04}").into(),
                    "b".repeat(64).into(),
                    timestamp(now - Duration::days(31)).into(),
                ],
            ))
            .await
            .expect("provider event");
    }
    let retention = StoreRetention::new(db.clone(), "primary-a");
    let held = retention
        .create_legal_hold(
            hold_input(
                StoreRetentionDataClass::RawCallbackBodies,
                vec!["event-0000".to_string()],
                now + Duration::days(1),
                None,
            ),
            "approver",
            now,
        )
        .await
        .expect("legal hold");

    let first = retention
        .run_at(now, RetentionRunActor::scheduled())
        .await
        .expect("first run");

    assert_eq!(first.state, StoreRetentionRunState::Succeeded);
    assert_eq!(first.policy_version, "2026-08");
    assert_eq!(first.counts.raw_callback_bodies, 500);
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE raw_ciphertext_base64 IS NOT NULL"
        )
        .await,
        1
    );
    let held_event = row(
        &db,
        "SELECT raw_ciphertext_base64, body_digest, parsed_json, projection_state
         FROM store_provider_events WHERE id = 'event-0000'",
    )
    .await;
    assert_eq!(
        held_event
            .try_get::<String>("", "raw_ciphertext_base64")
            .unwrap(),
        "ciphertext"
    );
    assert_eq!(
        held_event
            .try_get::<String>("", "projection_state")
            .unwrap(),
        "applied"
    );

    let second = retention
        .run_at(now + Duration::days(2), RetentionRunActor::scheduled())
        .await
        .expect("second run");
    assert_eq!(second.counts.raw_callback_bodies, 1);
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE raw_ciphertext_base64 IS NOT NULL"
        )
        .await,
        0
    );
    assert!(
        !retention
            .list_legal_holds(now + Duration::days(2), 100)
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.id == held.id)
            .unwrap()
            .active
    );
}

#[tokio::test]
async fn legal_hold_extension_creates_new_immutable_approval_and_audit() {
    let db = database().await;
    let now = instant();
    let retention = StoreRetention::new(db.clone(), "primary-a");
    let original = retention
        .create_legal_hold(
            hold_input(
                StoreRetentionDataClass::FinancialRecords,
                vec!["order-b".to_string(), "order-a".to_string()],
                now + Duration::days(1),
                None,
            ),
            "approver-a",
            now,
        )
        .await
        .expect("initial hold");
    assert_eq!(original.identifiers, vec!["order-a", "order-b"]);

    let extension = retention
        .create_legal_hold(
            hold_input(
                StoreRetentionDataClass::FinancialRecords,
                vec!["order-a".to_string(), "order-b".to_string()],
                now + Duration::days(2),
                Some(original.id.clone()),
            ),
            "approver-b",
            now + Duration::hours(1),
        )
        .await
        .expect("extension");

    assert_ne!(extension.id, original.id);
    assert_eq!(
        extension.extends_hold_id.as_deref(),
        Some(original.id.as_str())
    );
    assert_eq!(
        scalar(&db, "SELECT COUNT(*) AS value FROM store_legal_holds").await,
        2
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) AS value FROM store_access_audits
             WHERE action = 'legal_hold_create' AND result = 'succeeded'"
        )
        .await,
        2
    );
    assert_eq!(
        retention
            .create_legal_hold(
                hold_input(
                    StoreRetentionDataClass::FinancialRecords,
                    vec!["order-a".to_string(), "order-b".to_string()],
                    extension.expires_at,
                    Some(extension.id.clone()),
                ),
                "approver-c",
                now + Duration::hours(2),
            )
            .await,
        Err(StoreRetentionError::InvalidInput)
    );
    let listed = retention
        .list_legal_holds(now + Duration::hours(36), 100)
        .await
        .unwrap();
    assert!(
        !listed
            .iter()
            .find(|hold| hold.id == original.id)
            .unwrap()
            .active
    );
    assert!(
        listed
            .iter()
            .find(|hold| hold.id == extension.id)
            .unwrap()
            .active
    );
}

#[tokio::test]
async fn three_failures_pause_checkout_and_only_containment_clears_the_pause() {
    let db = database().await;
    insert_user(&db, "admin").await;
    let now = instant();
    let retention = StoreRetention::new(db.clone(), "primary-a");

    for offset in 0..3 {
        let failed = retention
            .run_at(
                now + Duration::minutes(offset),
                RetentionRunActor::scheduled(),
            )
            .await
            .expect("failed run is recorded");
        assert_eq!(failed.state, StoreRetentionRunState::Failed);
        assert_eq!(
            failed.error_category.as_deref(),
            Some("privacy_policy_unavailable")
        );
    }
    let paused = retention.status().await.unwrap();
    assert_eq!(paused.consecutive_failures, 3);
    assert!(paused.checkout_paused);
    let first_alert_id = paused.active_alert.unwrap().id;

    let snapshot = ExchangeRateSnapshot {
        base: "USD".to_string(),
        quote: "CNY".to_string(),
        cny_per_usd: "7".to_string(),
        source_updated_at: now,
        refreshed_at: now,
    };
    assert_eq!(
        PaymentOrderStore::new(db.clone())
            .create_order(
                "admin",
                CreatePaymentOrderInput {
                    idempotency_key: "new-order".to_string(),
                    product_id: "missing".to_string(),
                    payment_channel_id: "missing".to_string(),
                    payment_currency: Currency::CNY,
                    custom_recharge_minor: None,
                },
                &snapshot,
            )
            .await,
        Err(PaymentOrderError::RetentionPaused)
    );

    retention
        .contain(
            CreateStoreRetentionContainmentInput {
                reason: "deletion service recovered".to_string(),
                evidence_digest: "c".repeat(64),
            },
            "admin",
            now + Duration::minutes(3),
        )
        .await
        .expect("containment");
    let contained = retention.status().await.unwrap();
    assert_eq!(contained.consecutive_failures, 3);
    assert!(!contained.checkout_paused);
    assert!(contained.active_alert.is_none());

    retention
        .run_at(now + Duration::minutes(4), RetentionRunActor::scheduled())
        .await
        .expect("fourth failed run");
    let repaused = retention.status().await.unwrap();
    assert_eq!(repaused.consecutive_failures, 4);
    assert!(repaused.checkout_paused);
    assert_ne!(repaused.active_alert.unwrap().id, first_alert_id);

    insert_policy(&db, now + Duration::minutes(5)).await;
    let succeeded = retention
        .run_at(now + Duration::minutes(5), RetentionRunActor::scheduled())
        .await
        .expect("successful run");
    assert_eq!(succeeded.state, StoreRetentionRunState::Succeeded);
    let success_does_not_contain = retention.status().await.unwrap();
    assert_eq!(success_does_not_contain.consecutive_failures, 0);
    assert!(success_does_not_contain.checkout_paused);

    retention
        .contain(
            CreateStoreRetentionContainmentInput {
                reason: "verified successful deletion".to_string(),
                evidence_digest: "d".repeat(64),
            },
            "admin",
            now + Duration::minutes(6),
        )
        .await
        .expect("second containment");
    assert!(!retention.status().await.unwrap().checkout_paused);
}

#[tokio::test]
async fn retention_reauthentication_scopes_are_distinct() {
    let db = database().await;
    insert_user(&db, "admin").await;
    let reauth = ReauthStore::new(db);
    let retention = reauth
        .issue("admin", "session", "retention_operation")
        .await
        .expect("retention grant");
    let legal = reauth
        .issue("admin", "session", "legal_hold")
        .await
        .expect("legal hold grant");

    reauth
        .verify("admin", "session", &retention.token, "retention_operation")
        .await
        .expect("matching grant");
    assert!(
        reauth
            .verify("admin", "session", &retention.token, "legal_hold")
            .await
            .is_err()
    );
    reauth
        .verify("admin", "session", &legal.token, "legal_hold")
        .await
        .expect("matching legal grant");
}

#[tokio::test]
async fn admin_retention_routes_enforce_session_origin_reauthentication_and_exact_json() {
    let temp = TempDir::new().expect("temporary directory");
    let mut state = load_state_with_runtime(RuntimeConfig {
        listen: "127.0.0.1:0".to_string(),
        metrics_path: "/metrics".to_string(),
        database_dsn: format!("sqlite://{}", temp.path().join("monoize.db").display()),
        request_log_spool_dir: None,
        node: NodeSettings::primary_default(),
    })
    .await
    .expect("primary state");
    state.payment_public_origin = Some(url::Url::parse("https://store.example").unwrap());
    let admin = state
        .user_store
        .create_user(
            "retention-admin",
            "password",
            monoize::users::UserRole::Admin,
            None,
        )
        .await
        .expect("admin");
    let session = state
        .user_store
        .create_session(&admin.id, 7)
        .await
        .expect("session");
    let legal_grant = ReauthStore::new(state.db_pool.clone())
        .issue(&admin.id, &session.token, "legal_hold")
        .await
        .expect("legal hold grant");
    let router = build_app(state.clone());

    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/store/admin/retention")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let hold_body = json!({
        "data_class": "financial_records",
        "identifiers": ["order-1"],
        "reason": "preserve evidence",
        "requesting_authority": "Privacy Office",
        "requester_id": "requester",
        "approver_role": "privacy",
        "expires_at": timestamp(Utc::now() + Duration::days(1)),
        "extends_hold_id": null
    });
    let wrong_origin = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/dashboard/store/admin/retention/legal-holds")
                .header(COOKIE, format!("monoize_session={}", session.token))
                .header(CONTENT_TYPE, "application/json")
                .header(ORIGIN, "https://attacker.example")
                .header("X-Store-Reauth-Token", &legal_grant.token)
                .body(Body::from(hold_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);

    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/dashboard/store/admin/retention/legal-holds")
                .header(COOKIE, format!("monoize_session={}", session.token))
                .header(CONTENT_TYPE, "application/json")
                .header(ORIGIN, "https://store.example")
                .header("X-Store-Reauth-Token", &legal_grant.token)
                .body(Body::from(hold_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(response_json(created).await["active"], true);

    let invalid_body = {
        let mut body = hold_body;
        body["unknown"] = json!(true);
        body
    };
    let invalid = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/dashboard/store/admin/retention/legal-holds")
                .header(COOKIE, format!("monoize_session={}", session.token))
                .header(CONTENT_TYPE, "application/json")
                .header(ORIGIN, "https://store.example")
                .header("X-Store-Reauth-Token", &legal_grant.token)
                .body(Body::from(invalid_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let overview = router
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/store/admin/retention")
                .header(COOKIE, format!("monoize_session={}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(overview.status(), StatusCode::OK);
    assert_eq!(overview.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    let overview = response_json(overview).await;
    assert_eq!(overview["holds"].as_array().unwrap().len(), 1);
}
