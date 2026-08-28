use std::sync::Arc;

use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration, Utc};
use ed25519_dalek::SigningKey;
use http_body_util::BodyExt;
use monoize::app::{AppState, RuntimeConfig, build_app, load_state_with_runtime};
use monoize::node_config::{NodeRole, NodeSettings};
use monoize::replica::admission_http::{
    ADMISSION_CONFIRM_PATH, ADMISSION_ISSUE_PATH, ADMISSION_KEYSET_PATH,
};
use monoize::replica::metering::METERING_INGEST_PATH;
use monoize::store_billing::admission_runtime::AdmissionService;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::models::{PlanQuota, WindowKind};
use monoize::store_billing::quota::{EntitlementGenerationInput, QuotaStore};
use monoize::store_billing::quota_gate::{GateSlot, QuotaGateStore, QuotaManifest};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

const REPLICA_TOKEN: &str = "replica-cluster-token-with-at-least-32-bytes";
const REPLICA_ID: &str = "6f8b7f54-1833-4dc8-9e93-24482e870c22";

async fn primary_state(token: Option<&str>) -> (TempDir, AppState) {
    let temp = TempDir::new().expect("temp dir");
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
    (temp, state)
}

async fn request(
    app: &axum::Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    replica_id: Option<&str>,
    content_type: Option<&str>,
    body: impl Into<Body>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(replica_id) = replica_id {
        builder = builder.header("X-Monoize-Replica-ID", replica_id);
    }
    if let Some(content_type) = content_type {
        builder = builder.header(CONTENT_TYPE, content_type);
    }
    app.clone()
        .oneshot(builder.body(body.into()).expect("request"))
        .await
        .expect("response")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("JSON response")
}

async fn assert_error(response: axum::response::Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["message"].is_string());
    assert_eq!(body["error"].as_object().expect("error object").len(), 2);
    assert_eq!(body.as_object().expect("envelope").len(), 1);
}

fn balance_issue(audience: &str) -> Value {
    json!({
        "audience": audience,
        "user_id": "user-without-plan",
        "request_id": "request-1",
        "effective_groups": ["default"],
        "maximum_nano_usd": "1000000",
        "pricing_revision": "pricing-1"
    })
}

async fn enable_plan_admission(state: &mut AppState) {
    let db = &state.db_pool;
    let now = Utc::now();
    let group: String = db
        .read()
        .query_one(db.stmt("SELECT id FROM monoize_groups WHERE is_default = 1", vec![]))
        .await
        .expect("default group query")
        .expect("default group")
        .try_get("", "id")
        .expect("default group id");
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO users
             (id, username, password_hash, role, created_at, updated_at, enabled,
              balance_nano_usd, balance_unlimited, group_id)
             VALUES ('http-plan-user', 'http-plan-user', 'test', 'user', $1, $1, 1,
                     '0', 0, $2)",
            vec![now.to_rfc3339().into(), group.into()],
        ))
        .await
        .expect("plan user");
    db.write()
        .await
        .execute_unprepared(
            "INSERT INTO store_products
             (id, kind, name, description, price_currency, price_minor,
              duration_seconds, group_ids, sort_order, enabled, created_at, updated_at, revision)
             VALUES ('http-plan', 'plan', 'HTTP plan', '', 'CNY', '100',
                     86400, '[]', 0, 0, '2026-08-28T00:00:00Z',
                     '2026-08-28T00:00:00Z', 1)",
        )
        .await
        .expect("plan product");

    let gate = QuotaGateStore::new(db.clone());
    let environment = gate.live_environment().await.expect("gate environment");
    gate.import_manifest(
        GateSlot::Current,
        QuotaManifest::passed(environment, "http-test", "drill", now, "admin")
            .expect("gate manifest"),
    )
    .await
    .expect("quota gate");
    QuotaStore::new(db.clone())
        .replace_entitlement(EntitlementGenerationInput {
            expected_generation: None,
            user_id: "http-plan-user".to_string(),
            product_id: "http-plan".to_string(),
            product_name: "HTTP plan".to_string(),
            starts_at: now - Duration::minutes(1),
            ends_at: now + Duration::days(1),
            rate_numerator: "6".to_string(),
            rate_denominator: "1".to_string(),
            group_ids: vec![],
            quotas: vec![PlanQuota {
                id: "http-plan-day".to_string(),
                window_kind: WindowKind::Day,
                window_seconds: 86_400,
                quota_fen_cny: "1000".to_string(),
                sort_order: 0,
            }],
            source_kind: "order".to_string(),
            source_id: "http-plan-source".to_string(),
        })
        .await
        .expect("plan entitlement");

    let wrap = Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new("http-wrap-key", [41_u8; 32]).expect("wrap key"),
            vec![],
        )
        .expect("wrap key ring"),
    );
    let key_id = "http-admission-key";
    let seed = [19_u8; 32];
    let activated_at = now - Duration::seconds(1);
    let encrypted = wrap
        .encrypt(&format!("store-admission-key:{key_id}:seed:v1"), &seed)
        .expect("encrypt admission seed");
    let public = URL_SAFE_NO_PAD.encode(SigningKey::from_bytes(&seed).verifying_key().as_bytes());
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_admission_keys
             (key_id, public_key_base64, encrypted_private_key_json, state,
              published_at, activated_at, retired_at, last_issued_expires_at,
              verify_until, config_epoch)
             VALUES ($1, $2, $3, 'active', $4, $4, NULL, NULL, NULL, 1)",
            vec![
                key_id.into(),
                public.into(),
                serde_json::to_string(&encrypted)
                    .expect("encrypted JSON")
                    .into(),
                activated_at.to_rfc3339().into(),
            ],
        ))
        .await
        .expect("admission key");
    state.admission_service = Some(Arc::new(
        AdmissionService::new(db.clone(), wrap, "lynshen-primary").expect("admission service"),
    ));
}

fn plan_issue() -> Value {
    json!({
        "audience": REPLICA_ID,
        "user_id": "http-plan-user",
        "request_id": "http-plan-request",
        "effective_groups": ["default"],
        "maximum_nano_usd": "1000000",
        "pricing_revision": "pricing-1"
    })
}

#[tokio::test]
async fn primary_and_replica_states_have_role_correct_admission_ownership() {
    let (_temp, primary) = primary_state(None).await;
    assert!(primary.admission_service.is_some());

    let replica = primary.with_node_role(NodeRole::Replica);
    assert!(replica.admission_service.is_none());
    assert!(replica.metering_token_digest.is_none());
}

#[tokio::test]
async fn internal_routes_mount_only_on_a_primary_with_a_replica_token() {
    let (_enabled_temp, enabled) = primary_state(Some(REPLICA_TOKEN)).await;
    let enabled = build_app(enabled);
    let keyset = request(
        &enabled,
        Method::GET,
        ADMISSION_KEYSET_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(keyset.status(), StatusCode::OK);

    let (_disabled_temp, disabled) = primary_state(None).await;
    let disabled = build_app(disabled);
    let missing = request(
        &disabled,
        Method::POST,
        ADMISSION_ISSUE_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        Some("application/json"),
        balance_issue(REPLICA_ID).to_string(),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let (_replica_temp, primary) = primary_state(Some(REPLICA_TOKEN)).await;
    let replica = build_app(primary.with_node_role(NodeRole::Replica));
    let missing = request(
        &replica,
        Method::POST,
        ADMISSION_ISSUE_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        Some("application/json"),
        balance_issue(REPLICA_ID).to_string(),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn authentication_precedes_body_parsing_and_requires_a_canonical_uuid_v4() {
    let (_temp, state) = primary_state(Some(REPLICA_TOKEN)).await;
    let app = build_app(state);

    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_ISSUE_PATH,
            Some("wrong-token"),
            Some(REPLICA_ID),
            Some("application/json"),
            "{invalid",
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "replica_auth_failed",
    )
    .await;

    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_ISSUE_PATH,
            Some(REPLICA_TOKEN),
            Some(&REPLICA_ID.to_ascii_uppercase()),
            Some("application/json"),
            "{invalid",
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "replica_auth_failed",
    )
    .await;

    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_ISSUE_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            "{invalid",
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "admission_input_invalid",
    )
    .await;

    assert_error(
        request(
            &app,
            Method::POST,
            METERING_INGEST_PATH,
            Some(REPLICA_TOKEN),
            None,
            Some("application/json"),
            "{invalid",
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "replica_auth_failed",
    )
    .await;
}

#[tokio::test]
async fn issue_enforces_transport_contract_and_returns_balance_without_token_fields() {
    let (_temp, state) = primary_state(Some(REPLICA_TOKEN)).await;
    let app = build_app(state);

    let response = request(
        &app,
        Method::POST,
        ADMISSION_ISSUE_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        Some("application/json; charset=utf-8"),
        balance_issue(REPLICA_ID).to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await, json!({ "funding": "balance" }));

    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_ISSUE_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("text/plain"),
            balance_issue(REPLICA_ID).to_string(),
        )
        .await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "admission_content_type_invalid",
    )
    .await;

    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_ISSUE_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            balance_issue("b8c79fe7-9f4a-49f8-a210-3bc1467ec50e").to_string(),
        )
        .await,
        StatusCode::FORBIDDEN,
        "replica_audience_mismatch",
    )
    .await;

    let mut unknown = balance_issue(REPLICA_ID);
    unknown["issued_at"] = json!("2026-08-28T00:00:00Z");
    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_ISSUE_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            unknown.to_string(),
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "admission_input_invalid",
    )
    .await;

    let mut noncanonical = balance_issue(REPLICA_ID);
    noncanonical["maximum_nano_usd"] = json!("01");
    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_ISSUE_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            noncanonical.to_string(),
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "admission_input_invalid",
    )
    .await;

    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_ISSUE_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            vec![b' '; 65_537],
        )
        .await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "admission_request_too_large",
    )
    .await;
}

#[tokio::test]
async fn keyset_is_verifier_only_and_never_cacheable() {
    let (_temp, state) = primary_state(Some(REPLICA_TOKEN)).await;
    let response = request(
        &build_app(state),
        Method::GET,
        ADMISSION_KEYSET_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    assert_eq!(json_body(response).await, json!({ "keys": [] }));
}

#[tokio::test]
async fn primary_admission_storage_error_does_not_expose_database_detail() {
    let (temp, state) = primary_state(Some(REPLICA_TOKEN)).await;
    state
        .db_pool
        .write()
        .await
        .execute_unprepared("DROP TABLE store_admission_keys")
        .await
        .unwrap();
    let response = request(
        &build_app(state),
        Method::GET,
        ADMISSION_KEYSET_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "admission_storage_error");
    assert_eq!(body["error"]["message"], "plan admission is unavailable");
    let encoded = body.to_string();
    assert!(!encoded.contains("store_admission_keys"), "{encoded}");
    assert!(
        !encoded.contains(&temp.path().display().to_string()),
        "{encoded}"
    );
}

#[tokio::test]
async fn confirm_maps_audience_input_and_missing_token_failures() {
    let (_temp, state) = primary_state(Some(REPLICA_TOKEN)).await;
    let app = build_app(state);
    let body = json!({
        "audience": REPLICA_ID,
        "token_id": "missing-token",
        "reservation_id": "missing-reservation",
        "request_id": "missing-request"
    });

    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_CONFIRM_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            body.to_string(),
        )
        .await,
        StatusCode::NOT_FOUND,
        "admission_token_not_found",
    )
    .await;

    let mut mismatch = body.clone();
    mismatch["audience"] = json!("b8c79fe7-9f4a-49f8-a210-3bc1467ec50e");
    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_CONFIRM_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            mismatch.to_string(),
        )
        .await,
        StatusCode::FORBIDDEN,
        "replica_audience_mismatch",
    )
    .await;

    let mut unknown = body;
    unknown["unknown"] = json!(true);
    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_CONFIRM_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            unknown.to_string(),
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "admission_input_invalid",
    )
    .await;
}

#[tokio::test]
async fn plan_issue_keyset_and_confirmation_preserve_persisted_bindings() {
    let (_temp, mut state) = primary_state(Some(REPLICA_TOKEN)).await;
    enable_plan_admission(&mut state).await;
    let app = build_app(state);

    let issue = request(
        &app,
        Method::POST,
        ADMISSION_ISSUE_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        Some("application/json"),
        plan_issue().to_string(),
    )
    .await;
    let issue_status = issue.status();
    let issued = json_body(issue).await;
    assert_eq!(issue_status, StatusCode::OK, "{issued}");
    assert_eq!(issued["funding"], "plan");
    assert_eq!(issued["duplicate"], false);
    assert!(
        issued["compact_jws"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert!(issued["issued_at"].as_str().is_some());
    assert!(issued["expires_at"].as_str().is_some());
    assert_eq!(issued.as_object().expect("plan response").len(), 7);

    let duplicate = request(
        &app,
        Method::POST,
        ADMISSION_ISSUE_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        Some("application/json"),
        plan_issue().to_string(),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::OK);
    let duplicate = json_body(duplicate).await;
    assert_eq!(duplicate["duplicate"], true);
    for field in [
        "token_id",
        "reservation_id",
        "compact_jws",
        "issued_at",
        "expires_at",
    ] {
        assert_eq!(duplicate[field], issued[field], "persisted {field}");
    }

    let keyset = request(
        &app,
        Method::GET,
        ADMISSION_KEYSET_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(keyset.status(), StatusCode::OK);
    let keyset = json_body(keyset).await;
    assert_eq!(keyset["keys"].as_array().expect("keyset").len(), 1);
    let key = &keyset["keys"][0];
    assert_eq!(key["state"], "active");
    assert_eq!(key["verify_until"], Value::Null);
    assert_eq!(key.as_object().expect("public key").len(), 5);
    assert!(key.get("encrypted_private_key_json").is_none());

    let confirmation = json!({
        "audience": REPLICA_ID,
        "token_id": issued["token_id"],
        "reservation_id": issued["reservation_id"],
        "request_id": "http-plan-request"
    });
    let first = request(
        &app,
        Method::POST,
        ADMISSION_CONFIRM_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        Some("application/json"),
        confirmation.to_string(),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(
        json_body(first).await,
        json!({ "confirmed": true, "duplicate": false })
    );

    let second = request(
        &app,
        Method::POST,
        ADMISSION_CONFIRM_PATH,
        Some(REPLICA_TOKEN),
        Some(REPLICA_ID),
        Some("application/json"),
        confirmation.to_string(),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        json_body(second).await,
        json!({ "confirmed": true, "duplicate": true })
    );

    let mut conflict = confirmation;
    conflict["request_id"] = json!("another-request");
    assert_error(
        request(
            &app,
            Method::POST,
            ADMISSION_CONFIRM_PATH,
            Some(REPLICA_TOKEN),
            Some(REPLICA_ID),
            Some("application/json"),
            conflict.to_string(),
        )
        .await,
        StatusCode::CONFLICT,
        "admission_binding_mismatch",
    )
    .await;
}
