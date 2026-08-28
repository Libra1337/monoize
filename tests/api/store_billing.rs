use async_trait::async_trait;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN};
use axum::http::{Method, Request, StatusCode};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use monoize::store_billing::adapters::wechat::WechatCredential;
use monoize::store_billing::credentials::CredentialStore;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::{
    ExchangeRateFetcher, ExchangeRateService, ExchangeRateSnapshot, ExchangeRateStore,
};
use monoize::store_billing::governance::PaymentGovernanceStore;
use monoize::store_billing::{
    MerchantCapabilityKind, MerchantCapabilityState, PutStoreMerchantCapabilityInput,
};
use monoize::users::UserRole;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Barrier;
use tower::ServiceExt;

use super::setup;

#[derive(Clone)]
struct OfflineRateFetcher;

#[async_trait]
impl ExchangeRateFetcher for OfflineRateFetcher {
    async fn fetch_latest_usd(&self) -> Result<String, String> {
        Err("offline test fetcher".to_string())
    }
}

async fn configure_rate(ctx: &mut super::TestContext) {
    let snapshot = ExchangeRateSnapshot {
        base: "USD".to_string(),
        quote: "CNY".to_string(),
        cny_per_usd: "6.7370".to_string(),
        source_updated_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 0, 0).unwrap(),
        refreshed_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
    };
    let store = ExchangeRateStore::new(ctx.state.db_pool.clone());
    store.persist(&snapshot).await.unwrap();
    ctx.state.exchange_rate_service = ExchangeRateService::with_fetcher(store, OfflineRateFetcher)
        .await
        .unwrap();
    ctx.router = monoize::app::build_app(ctx.state.clone());
}

async fn configure_offline_rate(ctx: &mut super::TestContext) {
    ctx.state
        .db_pool
        .write()
        .await
        .execute(
            ctx.state
                .db_pool
                .stmt("DELETE FROM store_exchange_rates", vec![]),
        )
        .await
        .unwrap();
    ctx.state.exchange_rate_service = ExchangeRateService::with_fetcher(
        ExchangeRateStore::new(ctx.state.db_pool.clone()),
        OfflineRateFetcher,
    )
    .await
    .unwrap();
    ctx.router = monoize::app::build_app(ctx.state.clone());
}

async fn seed_governed_stripe(ctx: &super::TestContext) {
    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_payment_channels SET enabled = 1
             WHERE id = 'store-channel-stripe';
             INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ('api-governance-credential', 'store-channel-stripe', 'stripe', 1,
                     'key', 'nonce', 'ciphertext',
                     '2222222222222222222222222222222222222222222222222222222222222222',
                     'active',
                     '2026-08-28T00:00:00Z');
             INSERT INTO store_payment_compliance
                (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
             VALUES ('api-governance-compliance', 'store-channel-stripe', '2026-08-28',
                     'admin', '127.0.0.1', '2026-08-28T00:00:00Z');
             INSERT INTO store_merchant_capabilities
                (id, channel_id, capability, state, environment, merchant_account_digest,
                 provider_product, evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES
                ('api-cap-payment-query', 'store-channel-stripe', 'payment_query', 'supported',
                 'sandbox', '2222222222222222222222222222222222222222222222222222222222222222', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin',
                 '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('api-cap-refund', 'store-channel-stripe', 'refund', 'supported',
                 'sandbox', '2222222222222222222222222222222222222222222222222222222222222222', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin',
                 '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('api-cap-refund-query', 'store-channel-stripe', 'refund_query', 'supported',
                 'sandbox', '2222222222222222222222222222222222222222222222222222222222222222', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin',
                 '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('api-cap-settlement', 'store-channel-stripe', 'settlement_report', 'supported',
                 'sandbox', '2222222222222222222222222222222222222222222222222222222222222222', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin',
                 '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z');
             INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('api-governance-privacy', 'v1', 'CN', '[]', '{}', 'contract', 'admin',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z', 1);
             INSERT INTO store_channel_readiness_profiles
                (channel_id, active_credential_digest, privacy_record_id,
                 callback_verification_passed, supported_currencies_json, amount_limits_json,
                 checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                 availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES ('store-channel-stripe',
                     '2222222222222222222222222222222222222222222222222222222222222222',
                     'api-governance-privacy', 1, '[\"CNY\",\"USD\"]',
                     '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                     '[\"redirect\"]',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z')",
        )
        .await
        .unwrap();
}

async fn dashboard_session(ctx: &super::TestContext, username: &str, role: UserRole) -> String {
    let user = ctx
        .state
        .user_store
        .create_user(username, "test-password", role, None)
        .await
        .unwrap();
    let session = ctx
        .state
        .user_store
        .create_session(&user.id, 7)
        .await
        .unwrap();
    format!("Bearer {}", session.token)
}

async fn json_request(
    ctx: &super::TestContext,
    method: Method,
    path: &str,
    authorization: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(authorization) = authorization {
        builder = builder.header(AUTHORIZATION, authorization);
    }
    let body = if let Some(body) = body {
        builder = builder.header(CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = ctx
        .router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    (status, response_json(response).await)
}

async fn json_request_with_reauth(
    ctx: &super::TestContext,
    method: Method,
    path: &str,
    authorization: &str,
    reauth_token: Option<&str>,
    body: Value,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(AUTHORIZATION, authorization)
        .header(CONTENT_TYPE, "application/json");
    if let Some(token) = reauth_token {
        builder = builder.header("X-Store-Reauth-Token", token);
    }
    let response = ctx
        .router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, value, headers)
}

async fn cookie_json_request(
    ctx: &super::TestContext,
    method: Method,
    path: &str,
    session_token: &str,
    origin: Option<&str>,
    reauth_token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(COOKIE, format!("monoize_session={session_token}"))
        .header(CONTENT_TYPE, "application/json");
    if let Some(origin) = origin {
        builder = builder.header(ORIGIN, origin);
    }
    if let Some(token) = reauth_token {
        builder = builder.header("X-Store-Reauth-Token", token);
    }
    let response = ctx
        .router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    (status, response_json(response).await)
}

async fn raw_request_with_reauth(
    ctx: &super::TestContext,
    method: Method,
    path: &str,
    authorization: &str,
    reauth_token: Option<&str>,
    body: Value,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(AUTHORIZATION, authorization)
        .header(CONTENT_TYPE, "application/json");
    if let Some(token) = reauth_token {
        builder = builder.header("X-Store-Reauth-Token", token);
    }
    let response = ctx
        .router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

#[tokio::test]
async fn wechat_credential_replacement_persists_the_merchant_side_identity_digest() {
    let mut ctx = setup().await;
    let admin = dashboard_session(&ctx, "wechat_credential_admin", UserRole::Admin).await;
    ctx.state.payment_keys = Some(Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new("wechat-credential-key", [37_u8; 32]).unwrap(),
            vec![],
        )
        .unwrap(),
    ));
    ctx.router = monoize::app::build_app(ctx.state.clone());
    let credential = json!({
        "merchant_id":"1900000109",
        "app_id":"wx1234567890",
        "api_v3_key":"0123456789abcdef0123456789abcdef",
        "merchant_certificate_serial":"merchant-certificate-1",
        "merchant_private_key_pem":"merchant-private-key-1",
        "platform_certificate_serial":"platform-certificate-1",
        "platform_public_key_pem":"platform-public-key-1"
    });
    let expected_digest = WechatCredential::from_json(credential.to_string().as_bytes())
        .unwrap()
        .account_identity_digest();
    let (status, grant, _) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        &admin,
        None,
        json!({"current_password":"test-password","scope":"credential_update"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    let (status, saved, _) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-wechat/credential",
        &admin,
        grant["token"].as_str(),
        credential,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["account_identity_digest"], expected_digest);
    assert!(!saved.to_string().contains("0123456789abcdef"));
    assert!(!saved.to_string().contains("merchant-private-key-1"));
    let persisted = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT account_identity_digest FROM store_channel_credentials
             WHERE channel_id = 'store-channel-wechat' AND status = 'active'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "account_identity_digest")
        .unwrap();
    assert_eq!(persisted, expected_digest);
}

#[tokio::test]
async fn credential_replacement_requires_scoped_reauth_and_never_returns_secrets() {
    let mut ctx = setup().await;
    let admin = dashboard_session(&ctx, "credential_admin", UserRole::Admin).await;
    ctx.state.payment_keys = Some(Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new("credential-key", [31_u8; 32]).unwrap(),
            vec![],
        )
        .unwrap(),
    ));
    ctx.router = monoize::app::build_app(ctx.state.clone());
    let credential = json!({
        "secret_key":"sk_test_reauth",
        "publishable_key":"pk_test_reauth",
        "webhook_signing_secret":"whsec_reauth",
        "api_version":"2026-08-01",
        "account_id":"acct_reauth",
        "live_mode":false
    });

    let (status, error, _) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/credential",
        &admin,
        None,
        credential.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");

    let (status, grant, headers) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        &admin,
        None,
        json!({"current_password":"test-password","scope":"credential_update"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");
    let token = grant["token"].as_str().unwrap();

    let (status, saved, headers) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/credential",
        &admin,
        Some(token),
        credential,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");
    let serialized = saved.to_string();
    for forbidden in [
        "sk_test_reauth",
        "whsec_reauth",
        "ciphertext",
        "nonce_base64",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "response exposed {forbidden}"
        );
    }

    let row = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT c.ciphertext_base64, c.status, p.enabled
             FROM store_channel_credentials c
             JOIN store_payment_channels p ON p.id = c.channel_id
             WHERE c.channel_id = 'store-channel-stripe' AND c.status = 'active'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        row.try_get::<String>("", "ciphertext_base64").unwrap(),
        "sk_test_reauth"
    );
    assert_eq!(row.try_get::<i32>("", "enabled").unwrap(), 0);

    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "INSERT INTO store_merchant_capabilities
             (id, channel_id, capability, state, environment, merchant_account_digest,
              provider_product, evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES
             ('capability-before-replace', 'store-channel-stripe', 'refund', 'supported',
              'sandbox', 'old-account', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'credential_admin',
              '2026-08-27T00:00:00Z', '2026-11-25T00:00:00Z')",
        )
        .await
        .unwrap();
    let (status, _, _) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/credential",
        &admin,
        Some(token),
        json!({
            "secret_key":"sk_test_reauth_2",
            "publishable_key":"pk_test_reauth_2",
            "webhook_signing_secret":"whsec_reauth_2",
            "api_version":"2026-08-01",
            "account_id":"acct_reauth_2",
            "live_mode":false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let counts = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT
                SUM(CASE WHEN status = 'active' THEN 1 ELSE 0 END) AS active_count,
                SUM(CASE WHEN status = 'retired' THEN 1 ELSE 0 END) AS retired_count,
                (SELECT COUNT(*) FROM store_merchant_capabilities
                 WHERE channel_id = 'store-channel-stripe') AS capability_count
             FROM store_channel_credentials
             WHERE channel_id = 'store-channel-stripe'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counts.try_get::<i64>("", "active_count").unwrap(), 1);
    assert_eq!(counts.try_get::<i64>("", "retired_count").unwrap(), 1);
    assert_eq!(counts.try_get::<i64>("", "capability_count").unwrap(), 0);
}

#[tokio::test]
async fn cookie_store_secret_mutations_require_the_configured_origin() {
    let mut ctx = setup().await;
    let admin = dashboard_session(&ctx, "origin_admin", UserRole::Admin).await;
    let session_token = admin.strip_prefix("Bearer ").unwrap();
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.state.payment_keys = Some(Arc::new(
        PaymentKeyRing::new(PaymentKey::new("origin-key", [47_u8; 32]).unwrap(), vec![]).unwrap(),
    ));
    ctx.router = monoize::app::build_app(ctx.state.clone());
    let reauth_body = json!({
        "current_password":"test-password",
        "scope":"credential_update"
    });

    for origin in [None, Some("https://attacker.example")] {
        let (status, body) = cookie_json_request(
            &ctx,
            Method::POST,
            "/api/dashboard/store/admin/reauth",
            session_token,
            origin,
            None,
            reauth_body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"]["code"], "store_origin_invalid");
    }

    let (status, grant) = cookie_json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        session_token,
        Some("https://lynshen.org"),
        None,
        reauth_body,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");

    let (status, body) = cookie_json_request(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/credential",
        session_token,
        None,
        grant["token"].as_str(),
        json!({
            "secret_key":"sk_test_origin",
            "publishable_key":"pk_test_origin",
            "webhook_signing_secret":"whsec_origin",
            "api_version":"2026-08-01",
            "account_id":"acct_origin",
            "live_mode":false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "store_origin_invalid");

    let response = raw_store_mutation(
        &ctx.router,
        Method::POST,
        "/api/dashboard/store/admin/icons",
        Some(session_token),
        None,
        Some("https://attacker.example"),
        Some("text/plain"),
        Body::from("not-multipart"),
    )
    .await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "store_origin_invalid");
}

fn store_json_mutations() -> Vec<(Method, &'static str)> {
    vec![
        (Method::POST, "/api/dashboard/store/orders"),
        (Method::POST, "/api/dashboard/store/orders/missing/attempts"),
        (Method::POST, "/api/dashboard/store/redeem"),
        (Method::POST, "/api/dashboard/store/admin/products"),
        (Method::PUT, "/api/dashboard/store/admin/products/missing"),
        (
            Method::DELETE,
            "/api/dashboard/store/admin/products/missing",
        ),
        (Method::POST, "/api/dashboard/store/admin/payment-channels"),
        (
            Method::PUT,
            "/api/dashboard/store/admin/payment-channels/missing",
        ),
        (
            Method::DELETE,
            "/api/dashboard/store/admin/payment-channels/missing",
        ),
        (Method::POST, "/api/dashboard/store/admin/reauth"),
        (
            Method::PUT,
            "/api/dashboard/store/admin/payment-channels/missing/credential",
        ),
        (
            Method::PUT,
            "/api/dashboard/store/admin/payment-channels/missing/compliance",
        ),
        (
            Method::PUT,
            "/api/dashboard/store/admin/payment-channels/missing/capabilities/refund",
        ),
        (Method::POST, "/api/dashboard/store/admin/redemption-codes"),
        (
            Method::POST,
            "/api/dashboard/store/admin/redemption-codes/reveal",
        ),
        (
            Method::POST,
            "/api/dashboard/store/admin/redemption-codes/export",
        ),
        (
            Method::POST,
            "/api/dashboard/store/admin/redemption-codes/missing/revoke",
        ),
        (Method::PUT, "/api/dashboard/store/admin/settings"),
    ]
}

async fn store_icon_mutation(
    router: &axum::Router,
    cookie: Option<&str>,
    bearer: Option<&str>,
    origin: Option<&str>,
) -> axum::response::Response {
    let boundary = "store-origin-icon";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"icon.png\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(b"\x89PNG\r\n\x1a\norigin");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/dashboard/store/admin/icons")
        .header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        );
    if let Some(cookie) = cookie {
        request = request.header(COOKIE, format!("monoize_session={cookie}"));
    }
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, bearer);
    }
    if let Some(origin) = origin {
        request = request.header(ORIGIN, origin);
    }
    router
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

async fn raw_store_mutation(
    router: &axum::Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    bearer: Option<&str>,
    origin: Option<&str>,
    content_type: Option<&str>,
    body: impl Into<Body>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        request = request.header(COOKIE, format!("monoize_session={cookie}"));
    }
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, bearer);
    }
    if let Some(origin) = origin {
        request = request.header(ORIGIN, origin);
    }
    if let Some(content_type) = content_type {
        request = request.header(CONTENT_TYPE, content_type);
    }
    router
        .clone()
        .oneshot(request.body(body.into()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn store_mutation_guard_precedes_body_extraction_and_delete_bodies_are_strict_json() {
    let mut ctx = setup().await;
    let admin = dashboard_session(&ctx, "store_prebody_guard_admin", UserRole::Admin).await;
    let session_token = admin.strip_prefix("Bearer ").unwrap();
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.router = monoize::app::build_app(ctx.state.clone());

    let response = raw_store_mutation(
        &ctx.router,
        Method::POST,
        "/api/dashboard/store/admin/icons",
        Some(session_token),
        None,
        Some("https://attacker.example"),
        Some("multipart/form-data; boundary=broken"),
        Body::from("not-a-multipart-body"),
    )
    .await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "store_origin_invalid");

    let strict_json_routes = [
        (
            Method::DELETE,
            "/api/dashboard/store/admin/products/missing",
        ),
        (
            Method::DELETE,
            "/api/dashboard/store/admin/payment-channels/missing",
        ),
        (
            Method::POST,
            "/api/dashboard/store/admin/redemption-codes/missing/revoke",
        ),
    ];
    for (method, path) in strict_json_routes {
        for (content_type, raw_body) in [
            (None, ""),
            (Some("text/plain"), "{}"),
            (Some("application/json"), "{\"unexpected\":true}"),
        ] {
            let response = raw_store_mutation(
                &ctx.router,
                method.clone(),
                path,
                Some(session_token),
                None,
                Some("https://lynshen.org"),
                content_type,
                Body::from(raw_body),
            )
            .await;
            let status = response.status();
            let body = response_json(response).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {path}: {body}");
            assert_eq!(body["error"]["code"], "invalid_request", "{body}");
        }

        let response = raw_store_mutation(
            &ctx.router,
            method.clone(),
            path,
            Some(session_token),
            None,
            Some("https://lynshen.org"),
            Some("application/json"),
            Body::from("{}"),
        )
        .await;
        let status = response.status();
        let body = response_json(response).await;
        assert_ne!(status, StatusCode::BAD_REQUEST, "{method} {path}: {body}");
        assert_ne!(body["error"]["code"], "invalid_request", "{body}");
    }

    let response = raw_store_mutation(
        &ctx.router,
        Method::POST,
        "/api/dashboard/store/orders",
        Some(session_token),
        Some(&admin),
        None,
        Some("application/json"),
        Body::from("{}"),
    )
    .await;
    let body = response_json(response).await;
    assert_ne!(body["error"]["code"], "store_origin_invalid", "{body}");
}

#[tokio::test]
async fn every_cookie_store_mutation_requires_exact_origin_and_bearer_bypasses_origin() {
    let mut ctx = setup().await;
    let admin = dashboard_session(&ctx, "store_origin_matrix_admin", UserRole::Admin).await;
    let session_token = admin.strip_prefix("Bearer ").unwrap();
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.router = monoize::app::build_app(ctx.state.clone());

    for (method, path) in store_json_mutations() {
        for origin in [None, Some("https://attacker.example")] {
            let (status, body) = cookie_json_request(
                &ctx,
                method.clone(),
                path,
                session_token,
                origin,
                None,
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {body}");
            assert_eq!(
                body["error"]["code"], "store_origin_invalid",
                "{method} {path}: {body}"
            );
        }

        let (_, legal_origin) = cookie_json_request(
            &ctx,
            method.clone(),
            path,
            session_token,
            Some("https://lynshen.org"),
            None,
            json!({}),
        )
        .await;
        assert_ne!(
            legal_origin["error"]["code"], "store_origin_invalid",
            "legal Origin did not reach handler for {method} {path}"
        );

        let (_, bearer) =
            json_request(&ctx, method.clone(), path, Some(&admin), Some(json!({}))).await;
        assert_ne!(
            bearer["error"]["code"], "store_origin_invalid",
            "Bearer unexpectedly required Origin for {method} {path}"
        );
    }

    for origin in [None, Some("https://attacker.example")] {
        let response = store_icon_mutation(&ctx.router, Some(session_token), None, origin).await;
        let status = response.status();
        let body = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "icon: {body}");
        assert_eq!(body["error"]["code"], "store_origin_invalid", "{body}");
    }
    assert_eq!(
        store_icon_mutation(
            &ctx.router,
            Some(session_token),
            None,
            Some("https://lynshen.org"),
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        store_icon_mutation(&ctx.router, None, Some(&admin), None)
            .await
            .status(),
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn manual_complete_is_absent_and_replica_store_mutations_use_repository_rejection() {
    let ctx = setup().await;
    let admin = dashboard_session(&ctx, "store_replica_matrix_admin", UserRole::Admin).await;
    let before_icons: i64 = ctx
        .state
        .db_pool
        .read()
        .query_one(
            ctx.state
                .db_pool
                .stmt("SELECT COUNT(*) AS value FROM store_payment_icons", vec![]),
        )
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();

    let (status, _) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/orders/missing/complete",
        Some(&admin),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let replica = monoize::app::build_app(
        ctx.state
            .clone()
            .with_node_role(monoize::node_config::NodeRole::Replica),
    );
    for (method, path) in store_json_mutations() {
        let response = replica
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(path)
                    .header(AUTHORIZATION, &admin)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response_json(response).await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{method} {path}: {body}"
        );
        assert_eq!(body["error"]["code"], "store_write_rejected", "{body}");
    }
    let response = store_icon_mutation(&replica, None, Some(&admin), None).await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"]["code"], "store_write_rejected");

    let after_icons: i64 = ctx
        .state
        .db_pool
        .read()
        .query_one(
            ctx.state
                .db_pool
                .stmt("SELECT COUNT(*) AS value FROM store_payment_icons", vec![]),
        )
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    assert_eq!(after_icons, before_icons);
}

#[tokio::test]
async fn redemption_reveal_export_and_revocation_use_scoped_reauth_and_no_store_headers() {
    let mut ctx = setup().await;
    let admin = dashboard_session(&ctx, "redemption_admin", UserRole::Admin).await;
    ctx.state.payment_keys = Some(Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new("api-redemption-key", [79_u8; 32]).unwrap(),
            vec![],
        )
        .unwrap(),
    ));
    ctx.router = monoize::app::build_app(ctx.state.clone());

    let (status, generated, generated_headers) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/redemption-codes",
        &admin,
        None,
        json!({
            "reward":{"kind":"balance","currency":"USD","amount_minor":"100"},
            "count":2,
            "validity_days":30
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{generated}");
    assert_sensitive_headers(&generated_headers);
    let first_id = generated[0]["record"]["id"].as_str().unwrap();
    let second_id = generated[1]["record"]["id"].as_str().unwrap();
    let first_code = generated[0]["code"].as_str().unwrap();
    let second_code = generated[1]["code"].as_str().unwrap();
    assert_eq!(first_code.len(), 19);
    assert_eq!(second_code.len(), 19);

    let (status, error, _) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/redemption-codes/reveal",
        &admin,
        None,
        json!({"code_ids":[first_id],"action":"reveal"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");

    let (status, grant, grant_headers) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        &admin,
        None,
        json!({"current_password":"test-password","scope":"redemption_access"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    assert_sensitive_headers(&grant_headers);
    let token = grant["token"].as_str().unwrap();

    let (status, revealed, reveal_headers) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/redemption-codes/reveal",
        &admin,
        Some(token),
        json!({"code_ids":[first_id,second_id],"action":"copy"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{revealed}");
    assert_sensitive_headers(&reveal_headers);
    assert_eq!(revealed[0]["code"], first_code);
    assert_eq!(revealed[1]["code"], second_code);

    let (status, export_headers, bytes) = raw_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/redemption-codes/export",
        &admin,
        Some(token),
        json!({"code_ids":[first_id,second_id]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_sensitive_headers(&export_headers);
    assert_eq!(
        export_headers.get("content-type").unwrap(),
        "text/csv; charset=utf-8"
    );
    let csv = String::from_utf8(bytes).unwrap();
    assert!(csv.contains(first_code));
    assert!(csv.contains(second_code));

    let (status, revoked, _) = json_request_with_reauth(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/admin/redemption-codes/{second_id}/revoke"),
        &admin,
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{revoked}");
    assert_eq!(revoked["status"], "revoked");
    let row = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT encrypted_ciphertext_base64 FROM store_redemption_codes WHERE id = $1",
            vec![second_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<Option<String>>("", "encrypted_ciphertext_base64")
            .unwrap(),
        None
    );
}

fn assert_sensitive_headers(headers: &axum::http::HeaderMap) {
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");
    assert_eq!(headers.get("pragma").unwrap(), "no-cache");
    assert_eq!(headers.get("referrer-policy").unwrap(), "no-referrer");
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
}

async fn idempotent_json_request(
    ctx: &super::TestContext,
    path: &str,
    authorization: &str,
    idempotency_key: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = ctx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header(AUTHORIZATION, authorization)
                .header(CONTENT_TYPE, "application/json")
                .header("Idempotency-Key", idempotency_key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, response_json(response).await)
}

async fn stripe_availability(ctx: &super::TestContext, admin: &str) -> Value {
    let (status, availability, _) = json_request_with_reauth(
        ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/availability",
        admin,
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{availability}");
    availability
}

async fn assert_governed_order_rejected(
    ctx: &super::TestContext,
    user: &str,
    idempotency_key: &str,
    product_id: &Value,
    currency: &str,
) {
    let (status, body) = idempotent_json_request(
        ctx,
        "/api/dashboard/store/orders",
        user,
        idempotency_key,
        json!({
            "product_id": product_id,
            "payment_channel_id":"store-channel-stripe",
            "payment_currency":currency,
            "custom_recharge_minor":null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "payment_channel_unavailable");
}

async fn assert_corrupted_capability_blocks_channel(
    ctx: &super::TestContext,
    admin: &str,
    user: &str,
    product_id: &Value,
    idempotency_key: &str,
) {
    let availability = stripe_availability(ctx, admin).await;
    assert_eq!(availability["effective_available"], false);
    assert!(
        availability["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "capability_payment_query_invalid")
    );
    let (status, catalog) = json_request(
        ctx,
        Method::GET,
        "/api/dashboard/store/catalog",
        Some(user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    assert!(catalog["payment_channels"].as_array().unwrap().is_empty());
    assert_governed_order_rejected(ctx, user, idempotency_key, product_id, "CNY").await;
}

fn balance_product(name: &str) -> Value {
    json!({
        "kind": "balance",
        "name": name,
        "description": "API test recharge",
        "price_currency": "CNY",
        "price_minor": "1000",
        "duration_seconds": null,
        "group_ids": [],
        "sort_order": 10,
        "enabled": true,
        "balance": { "recharge_minor": "1000", "bonus_minor": "200" },
        "quotas": []
    })
}

fn payment_channel(name: &str) -> Value {
    json!({
        "adapter_kind": "http",
        "name": name,
        "icon_kind": "builtin",
        "icon_value": "custom",
        "sort_order": 10,
        "enabled": false
    })
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
}

#[tokio::test]
async fn store_catalog_requires_a_dashboard_session() {
    let ctx = setup().await;
    let (status, body) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/catalog",
        None,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], json!("unauthorized"));
}

#[tokio::test]
async fn entitlement_endpoint_returns_null_without_an_active_store_plan() {
    let ctx = setup().await;
    let user = dashboard_session(&ctx, "store_entitlement_user", UserRole::User).await;

    let (status, body) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/entitlement",
        Some(&user),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, Value::Null);
}

#[tokio::test]
async fn store_admin_guards_and_product_channel_management() {
    let mut ctx = setup().await;
    configure_rate(&mut ctx).await;
    let admin = dashboard_session(&ctx, "store_api_admin", UserRole::Admin).await;
    let user = dashboard_session(&ctx, "store_api_user", UserRole::User).await;

    let (status, body) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/products",
        Some(&user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], json!("forbidden"));

    let (status, product) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/products",
        Some(&admin),
        Some(balance_product("Recharge 10")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{product}");
    assert!(product["id"].is_string());

    let (status, channel) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/payment-channels",
        Some(&admin),
        Some(payment_channel("Custom provider")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");
    assert_eq!(channel["adapter_kind"], "http");
    assert!(channel.get("kind").is_none());
    assert!(channel.get("mode").is_none());
    assert!(channel.get("endpoint").is_none());
    assert!(channel.get("config_secret").is_none());

    let (status, catalog) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/catalog",
        Some(&user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    assert_eq!(catalog["products"].as_array().unwrap().len(), 1);
    assert!(catalog["payment_channels"].as_array().unwrap().is_empty());

    let (status, rate) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/exchange-rate",
        Some(&user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rate}");
    assert_eq!(rate["cny_per_usd"], json!("6.7370"));

    let (status, _) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/orders/legacy-order/complete",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn enabled_only_channel_is_not_available_for_catalog_or_orders() {
    let mut ctx = setup().await;
    configure_rate(&mut ctx).await;
    let admin = dashboard_session(&ctx, "governance_enabled_admin", UserRole::Admin).await;
    let user = dashboard_session(&ctx, "governance_enabled_user", UserRole::User).await;
    let (status, product) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/products",
        Some(&admin),
        Some(balance_product("Governed recharge")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{product}");
    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_payment_channels SET enabled = 1
             WHERE id = 'store-channel-stripe'",
        )
        .await
        .unwrap();

    let (status, catalog) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/catalog",
        Some(&user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    assert!(catalog["payment_channels"].as_array().unwrap().is_empty());

    let (status, body) = idempotent_json_request(
        &ctx,
        "/api/dashboard/store/orders",
        &user,
        "governance-enabled-only",
        json!({
            "product_id": product["id"],
            "payment_channel_id": "store-channel-stripe",
            "payment_currency": "CNY",
            "custom_recharge_minor": null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "payment_channel_unavailable");
}

#[tokio::test]
async fn payment_governance_endpoints_require_admin_reauth_and_no_store() {
    let ctx = setup().await;
    let admin = dashboard_session(&ctx, "governance_api_admin", UserRole::Admin).await;
    let user = dashboard_session(&ctx, "governance_api_user", UserRole::User).await;
    let compliance_path =
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/compliance";

    let (status, _) = json_request(&ctx, Method::GET, compliance_path, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = json_request(&ctx, Method::GET, compliance_path, Some(&user), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body, headers) =
        json_request_with_reauth(&ctx, Method::GET, compliance_path, &admin, None, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_sensitive_headers(&headers);
    assert_eq!(body["current_terms_version"], "2026-08-28");
    assert!(body["compliance"].is_null());

    let (status, error, _) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        compliance_path,
        &admin,
        None,
        json!({"confirmed":true,"terms_version":"2026-08-28"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");

    let (status, grant, _) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        &admin,
        None,
        json!({"current_password":"test-password","scope":"compliance_confirm"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    let (status, compliance, headers) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        compliance_path,
        &admin,
        grant["token"].as_str(),
        json!({"confirmed":true,"terms_version":"2026-08-28"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{compliance}");
    assert_sensitive_headers(&headers);
    assert!(compliance["admin_user_id"].is_string());
    assert!(compliance["confirmed_at"].is_string());
    assert!(compliance.get("credential").is_none());

    for suffix in ["capabilities", "availability"] {
        let (status, body, headers) = json_request_with_reauth(
            &ctx,
            Method::GET,
            &format!("/api/dashboard/store/admin/payment-channels/store-channel-stripe/{suffix}"),
            &admin,
            None,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_sensitive_headers(&headers);
    }
    let (status, body, _) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/capabilities/unknown",
        &admin,
        None,
        json!({
            "state":"supported",
            "environment":"sandbox",
            "provider_product":"checkout",
            "evidence_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "controlled_transaction_id":null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn governed_channel_becomes_available_and_rotation_fails_closed() {
    let mut ctx = setup().await;
    ctx.state.payment_keys = Some(Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new("governance-key", [59_u8; 32]).unwrap(),
            vec![],
        )
        .unwrap(),
    ));
    configure_rate(&mut ctx).await;
    let admin = dashboard_session(&ctx, "governance_flow_admin", UserRole::Admin).await;
    let user = dashboard_session(&ctx, "governance_flow_user", UserRole::User).await;
    let (_, product) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/products",
        Some(&admin),
        Some(balance_product("Governance flow recharge")),
    )
    .await;
    let stripe_credential = |suffix: &str| {
        json!({
            "secret_key":format!("sk_test_{suffix}"),
            "publishable_key":format!("pk_test_{suffix}"),
            "webhook_signing_secret":format!("whsec_{suffix}"),
            "api_version":"2026-08-01",
            "account_id":format!("acct_{suffix}"),
            "live_mode":false
        })
    };
    let (status, credential_grant, _) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        &admin,
        None,
        json!({"current_password":"test-password","scope":"credential_update"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{credential_grant}");
    let (status, saved, _) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/credential",
        &admin,
        credential_grant["token"].as_str(),
        stripe_credential("governed"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let merchant_digest = saved["account_identity_digest"].as_str().unwrap();

    let (_, channels) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels",
        Some(&admin),
        None,
    )
    .await;
    let stripe = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["id"] == "store-channel-stripe")
        .unwrap();
    assert_eq!(stripe["effective_available"], false);
    let (status, _) = json_request(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe",
        Some(&admin),
        Some(json!({"expected_revision":stripe["revision"],"enabled":true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, compliance_grant, _) = json_request_with_reauth(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        &admin,
        None,
        json!({"current_password":"test-password","scope":"compliance_confirm"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{compliance_grant}");
    let (status, _, _) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/compliance",
        &admin,
        compliance_grant["token"].as_str(),
        json!({"confirmed":true,"terms_version":"2026-08-28"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    for (capability, state) in [
        ("payment_query", "supported"),
        ("refund", "supported"),
        ("refund_query", "supported"),
        ("dispute_event", "manual"),
        ("dispute_query", "manual"),
        ("bill_download", "supported"),
        ("settlement_report", "supported"),
    ] {
        let (status, saved_capability, headers) = json_request_with_reauth(
            &ctx,
            Method::PUT,
            &format!(
                "/api/dashboard/store/admin/payment-channels/store-channel-stripe/capabilities/{capability}"
            ),
            &admin,
            None,
            json!({
                "state":state,
                "environment":"sandbox",
                "provider_product":"checkout",
                "evidence_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "controlled_transaction_id":format!("controlled-{capability}")
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved_capability}");
        assert_sensitive_headers(&headers);
        assert_eq!(saved_capability["merchant_account_digest"], merchant_digest);
    }
    let (_, all_capabilities, _) = json_request_with_reauth(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/capabilities",
        &admin,
        None,
        json!({}),
    )
    .await;
    assert_eq!(
        all_capabilities["capabilities"].as_array().unwrap().len(),
        7
    );

    let (status, pending, headers) = json_request_with_reauth(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/availability",
        &admin,
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pending}");
    assert_sensitive_headers(&headers);
    assert_eq!(pending["effective_available"], false);
    assert!(
        pending["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "readiness_profile_missing")
    );
    let (_, catalog) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/catalog",
        Some(&user),
        None,
    )
    .await;
    assert!(catalog["payment_channels"].as_array().unwrap().is_empty());
    let (status, unavailable_order) = idempotent_json_request(
        &ctx,
        "/api/dashboard/store/orders",
        &user,
        "governed-order",
        json!({
            "product_id":product["id"],
            "payment_channel_id":"store-channel-stripe",
            "payment_currency":"CNY",
            "custom_recharge_minor":null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{unavailable_order}");
    assert_eq!(
        unavailable_order["error"]["code"],
        "payment_channel_unavailable"
    );

    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('privacy-current', 'v1', 'CN', '[]', '{}', 'contract', 'privacy-admin',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z', 1)",
        )
        .await
        .unwrap();
    ctx.state
        .db_pool
        .write()
        .await
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_channel_readiness_profiles
                (channel_id, active_credential_digest, privacy_record_id,
                 callback_verification_passed, supported_currencies_json, amount_limits_json,
                 checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                 availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES ($1, $2, 'privacy-current', 1, '[\"CNY\",\"USD\"]',
                     '{\"CNY\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"},\"USD\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"}}',
                     '[\"redirect\"]',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'readiness-admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z')",
            vec!["store-channel-stripe".into(), merchant_digest.into()],
        ))
        .await
        .unwrap();
    let (status, availability, _) = json_request_with_reauth(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/availability",
        &admin,
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{availability}");
    assert_eq!(availability["effective_available"], true);
    assert_eq!(availability["supported_currencies"], json!(["CNY", "USD"]));
    assert_eq!(availability["checkout_action_kinds"], json!(["redirect"]));
    let (_, catalog) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/catalog",
        Some(&user),
        None,
    )
    .await;
    assert_eq!(catalog["payment_channels"].as_array().unwrap().len(), 1);
    let (status, order) = idempotent_json_request(
        &ctx,
        "/api/dashboard/store/orders",
        &user,
        "governed-order-ready",
        json!({
            "product_id":product["id"],
            "payment_channel_id":"store-channel-stripe",
            "payment_currency":"CNY",
            "custom_recharge_minor":null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{order}");

    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_readiness_profiles
             SET amount_limits_json = '{\"CNY\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"},\"CNY\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"},\"USD\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"}}'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();
    let duplicate_amount_currency = stripe_availability(&ctx, &admin).await;
    assert_eq!(duplicate_amount_currency["effective_available"], false);
    assert!(
        duplicate_amount_currency["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "readiness_metadata_invalid")
    );
    let (_, catalog) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/catalog",
        Some(&user),
        None,
    )
    .await;
    assert!(catalog["payment_channels"].as_array().unwrap().is_empty());
    assert_governed_order_rejected(
        &ctx,
        &user,
        "governed-duplicate-amount-currency",
        &product["id"],
        "CNY",
    )
    .await;
    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_readiness_profiles
             SET amount_limits_json = '{\"CNY\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"},\"USD\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"}}'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();

    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_readiness_profiles
             SET expires_at = '2026-01-01T00:00:00Z'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();
    let expired_profile = stripe_availability(&ctx, &admin).await;
    assert!(
        expired_profile["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "readiness_profile_expired")
    );
    ctx.state.db_pool.write().await.execute_unprepared(
        "UPDATE store_channel_readiness_profiles SET expires_at = '2099-01-01T00:00:00Z',
         active_credential_digest = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
         WHERE channel_id = 'store-channel-stripe'"
    ).await.unwrap();
    let mismatched_profile = stripe_availability(&ctx, &admin).await;
    assert!(
        mismatched_profile["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "readiness_profile_credential_mismatch")
    );
    ctx.state
        .db_pool
        .write()
        .await
        .execute(ctx.state.db_pool.stmt(
            "UPDATE store_channel_readiness_profiles SET active_credential_digest = $2
             WHERE channel_id = $1",
            vec!["store-channel-stripe".into(), merchant_digest.into()],
        ))
        .await
        .unwrap();
    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_privacy_records SET accepted = 0 WHERE id = 'privacy-current'",
        )
        .await
        .unwrap();
    let rejected_privacy = stripe_availability(&ctx, &admin).await;
    assert!(
        rejected_privacy["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "privacy_gate_pending")
    );
    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_privacy_records SET accepted = 1 WHERE id = 'privacy-current'",
        )
        .await
        .unwrap();

    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_readiness_profiles
         SET supported_currencies_json = '[\"USD\"]',
             amount_limits_json = '{\"USD\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"}}'
         WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();
    assert_governed_order_rejected(
        &ctx,
        &user,
        "governed-currency-rejected",
        &product["id"],
        "CNY",
    )
    .await;
    ctx.state.db_pool.write().await.execute_unprepared(
        "UPDATE store_channel_readiness_profiles
         SET supported_currencies_json = '[\"CNY\",\"USD\"]',
             amount_limits_json = '{\"CNY\":{\"min_minor\":\"2000\",\"max_minor\":\"100000\"},\"USD\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"}}'
         WHERE channel_id = 'store-channel-stripe'"
    ).await.unwrap();
    assert_governed_order_rejected(
        &ctx,
        &user,
        "governed-amount-rejected",
        &product["id"],
        "CNY",
    )
    .await;
    ctx.state.db_pool.write().await.execute_unprepared(
        "UPDATE store_channel_readiness_profiles
         SET amount_limits_json = '{\"CNY\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"},\"USD\":{\"min_minor\":\"100\",\"max_minor\":\"100000\"}}',
             checkout_action_kinds_json = '[\"qr\"]'
         WHERE channel_id = 'store-channel-stripe'"
    ).await.unwrap();
    let incompatible_action = stripe_availability(&ctx, &admin).await;
    assert!(
        incompatible_action["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "checkout_action_incompatible")
    );
    assert!(
        incompatible_action["checkout_action_kinds"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_readiness_profiles SET checkout_action_kinds_json = '[\"redirect\"]'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();

    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_merchant_capabilities
             SET verified_at = '2025-01-01T00:00:00Z',
                 expires_at = '2026-01-01T00:00:00Z'
         WHERE channel_id = 'store-channel-stripe' AND capability = 'settlement_report'",
        )
        .await
        .unwrap();
    let (_, expired, _) = json_request_with_reauth(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/availability",
        &admin,
        None,
        json!({}),
    )
    .await;
    assert_eq!(expired["effective_available"], false);
    assert!(
        expired["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "capability_settlement_report_expired")
    );
    ctx.state.db_pool.write().await.execute_unprepared(
        "UPDATE store_merchant_capabilities SET expires_at = '2099-01-01T00:00:00Z', merchant_account_digest = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
         WHERE channel_id = 'store-channel-stripe' AND capability = 'settlement_report'"
    ).await.unwrap();
    let (_, mismatched, _) = json_request_with_reauth(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/availability",
        &admin,
        None,
        json!({}),
    )
    .await;
    assert_eq!(mismatched["effective_available"], false);
    assert!(
        mismatched["unavailable_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "capability_settlement_report_credential_mismatch")
    );

    let (status, _, _) = json_request_with_reauth(
        &ctx,
        Method::PUT,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/credential",
        &admin,
        credential_grant["token"].as_str(),
        stripe_credential("rotated"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, capabilities, _) = json_request_with_reauth(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/capabilities",
        &admin,
        None,
        json!({}),
    )
    .await;
    assert!(capabilities["capabilities"].as_array().unwrap().is_empty());
    let (_, rotated, _) = json_request_with_reauth(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels/store-channel-stripe/availability",
        &admin,
        None,
        json!({}),
    )
    .await;
    assert_eq!(rotated["effective_available"], false);
}

#[tokio::test]
async fn malformed_capability_rows_fail_closed_without_breaking_catalog() {
    let mut ctx = setup().await;
    configure_rate(&mut ctx).await;
    seed_governed_stripe(&ctx).await;
    let admin = dashboard_session(&ctx, "capability_shape_admin", UserRole::Admin).await;
    let user = dashboard_session(&ctx, "capability_shape_user", UserRole::User).await;
    let (_, product) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/products",
        Some(&admin),
        Some(balance_product("Capability shape recharge")),
    )
    .await;
    assert_eq!(
        stripe_availability(&ctx, &admin).await["effective_available"],
        true
    );

    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_merchant_capabilities SET evidence_digest = 'broken'
             WHERE channel_id = 'store-channel-stripe' AND capability = 'payment_query'",
        )
        .await
        .unwrap();
    assert_corrupted_capability_blocks_channel(
        &ctx,
        &admin,
        &user,
        &product["id"],
        "capability-invalid-digest",
    )
    .await;
    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_merchant_capabilities
             SET evidence_digest = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 verified_at = 'not-a-timestamp'
             WHERE channel_id = 'store-channel-stripe' AND capability = 'payment_query'",
        )
        .await
        .unwrap();
    assert_corrupted_capability_blocks_channel(
        &ctx,
        &admin,
        &user,
        &product["id"],
        "capability-invalid-timestamp",
    )
    .await;

    let write = ctx.state.db_pool.write().await;
    write
        .execute_unprepared("PRAGMA ignore_check_constraints = ON")
        .await
        .unwrap();
    write
        .execute_unprepared(
            "UPDATE store_merchant_capabilities
             SET verified_at = '2026-08-28T00:00:00Z', state = 'unknown'
             WHERE channel_id = 'store-channel-stripe' AND capability = 'payment_query'",
        )
        .await
        .unwrap();
    write
        .execute_unprepared("PRAGMA ignore_check_constraints = OFF")
        .await
        .unwrap();
    drop(write);
    assert_corrupted_capability_blocks_channel(
        &ctx,
        &admin,
        &user,
        &product["id"],
        "capability-invalid-state",
    )
    .await;
    ctx.state
        .db_pool
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_merchant_capabilities
             SET state = 'supported', capability = 'PAYMENT_QUERY'
             WHERE channel_id = 'store-channel-stripe' AND capability = 'payment_query'",
        )
        .await
        .unwrap();
    assert_corrupted_capability_blocks_channel(
        &ctx,
        &admin,
        &user,
        &product["id"],
        "capability-invalid-name",
    )
    .await;
}

#[tokio::test]
async fn credential_rotation_and_capability_put_never_leave_an_old_merchant_digest() {
    let ctx = setup().await;
    let ring = Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new("governance-race-key", [61_u8; 32]).unwrap(),
            vec![],
        )
        .unwrap(),
    );
    let credentials = CredentialStore::new(ctx.state.db_pool.clone(), ring);
    let old = credentials
        .replace(
            "store-channel-stripe",
            json!({
                "secret_key":"sk_test_race_old",
                "publishable_key":"pk_test_race_old",
                "webhook_signing_secret":"whsec_race_old",
                "api_version":"2026-08-01",
                "account_id":"acct_race_old",
                "live_mode":false
            }),
        )
        .await
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let rotate_barrier = barrier.clone();
    let rotate_store = credentials.clone();
    let rotate = tokio::spawn(async move {
        rotate_barrier.wait().await;
        rotate_store
            .replace(
                "store-channel-stripe",
                json!({
                    "secret_key":"sk_test_race_new",
                    "publishable_key":"pk_test_race_new",
                    "webhook_signing_secret":"whsec_race_new",
                    "api_version":"2026-08-01",
                    "account_id":"acct_race_new",
                    "live_mode":false
                }),
            )
            .await
    });
    let capability_barrier = barrier.clone();
    let governance = PaymentGovernanceStore::new(ctx.state.db_pool.clone());
    let capability = tokio::spawn(async move {
        capability_barrier.wait().await;
        governance
            .put_capability(
                "store-channel-stripe",
                MerchantCapabilityKind::Refund,
                PutStoreMerchantCapabilityInput {
                    state: MerchantCapabilityState::Supported,
                    environment: "sandbox".to_string(),
                    provider_product: "checkout".to_string(),
                    evidence_digest:
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_string(),
                    controlled_transaction_id: Some("race-refund".to_string()),
                },
                "race-admin",
            )
            .await
    });
    barrier.wait().await;
    let rotated = rotate.await.unwrap().unwrap();
    capability.await.unwrap().unwrap();

    let persisted = PaymentGovernanceStore::new(ctx.state.db_pool.clone())
        .capabilities("store-channel-stripe")
        .await
        .unwrap();
    assert!(
        persisted.capabilities.is_empty()
            || persisted
                .capabilities
                .iter()
                .all(|record| record.merchant_account_digest == rotated.account_identity_digest)
    );
    assert!(
        persisted
            .capabilities
            .iter()
            .all(|record| record.merchant_account_digest != old.account_identity_digest)
    );
}

#[test]
fn postgres_governance_writers_lock_the_same_channel_row() {
    let credential_source = include_str!("../../src/store_billing/credentials.rs");
    let governance_source = include_str!("../../src/store_billing/governance.rs");
    assert!(credential_source.contains("lock_channel(&self.db, &*tx, channel_id)"));
    assert!(
        governance_source
            .contains("SELECT id FROM store_payment_channels WHERE id = $1 FOR UPDATE")
    );
}

#[test]
fn catalog_and_admin_channel_lists_use_the_fixed_query_batch_evaluator() {
    let store_source = include_str!("../../src/store_billing/store.rs").replace("\r\n", "\n");
    for (start, end) in [
        ("pub async fn catalog(", "pub async fn list_products_admin("),
        (
            "pub async fn list_payment_channels_admin(",
            "pub async fn delete_product(",
        ),
    ] {
        let start = store_source.find(start).expect("list function must exist");
        let end = store_source[start..]
            .find(end)
            .map(|offset| start + offset)
            .expect("list function must have a bounded body");
        let body = &store_source[start..end];
        assert!(body.contains("evaluate_channels("));
        assert!(!body.contains("evaluate_channel("));
    }

    let governance = include_str!("../../src/store_billing/governance.rs").replace("\r\n", "\n");
    let start = governance
        .find("async fn load_governance_snapshot")
        .expect("batch loader must exist");
    let end = governance[start..]
        .find("fn evaluate_snapshot_channel(")
        .map(|offset| start + offset)
        .expect("batch loader must have a bounded body");
    assert_eq!(governance[start..end].matches(".query_all(").count(), 6);

    let evaluate_start = governance
        .find("pub async fn evaluate_channel<")
        .expect("single Channel evaluator must exist");
    let evaluate_end = governance[evaluate_start..]
        .find("pub async fn evaluate_channels<")
        .map(|offset| evaluate_start + offset)
        .expect("single Channel evaluator must have a bounded body");
    assert!(
        governance[evaluate_start..evaluate_end]
            .contains("load_scoped_governance_snapshot")
    );
    let scoped_start = governance
        .find("async fn load_scoped_governance_snapshot")
        .expect("scoped loader must exist");
    let scoped_end = governance[scoped_start..]
        .find("async fn load_governance_snapshot")
        .map(|offset| scoped_start + offset)
        .expect("scoped loader must have a bounded body");
    let scoped = &governance[scoped_start..scoped_end];
    assert_eq!(scoped.matches(".query_all(").count(), 6);
    assert_eq!(scoped.matches("vec![channel_id.into()]").count(), 6);
    assert!(scoped.contains(
        "SELECT privacy_record_id FROM store_channel_readiness_profiles WHERE channel_id = $1"
    ));
}

#[tokio::test]
async fn batch_and_single_channel_availability_match_for_two_channels() {
    let ctx = setup().await;
    seed_governed_stripe(&ctx).await;
    let admin = dashboard_session(&ctx, "batch_availability_admin", UserRole::Admin).await;
    let (status, channels) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/payment-channels",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{channels}");
    for channel_id in ["store-channel-stripe", "store-channel-alipay"] {
        let listed = channels
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["id"] == channel_id)
            .unwrap();
        let (status, single, _) = json_request_with_reauth(
            &ctx,
            Method::GET,
            &format!(
                "/api/dashboard/store/admin/payment-channels/{channel_id}/availability"
            ),
            &admin,
            None,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{single}");
        for field in [
            "effective_available",
            "unavailable_reasons",
            "supported_currencies",
            "amount_limits",
            "checkout_action_kinds",
        ] {
            assert_eq!(listed[field], single[field], "{channel_id}.{field}");
        }
    }
}

#[tokio::test]
async fn redemption_does_not_require_a_payment_channel() {
    let mut ctx = setup().await;
    configure_offline_rate(&mut ctx).await;
    ctx.state.payment_keys = Some(Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new("offline-redemption-key", [83_u8; 32]).unwrap(),
            vec![],
        )
        .unwrap(),
    ));
    ctx.router = monoize::app::build_app(ctx.state.clone());
    let admin = dashboard_session(&ctx, "store_code_admin", UserRole::Admin).await;
    let user = dashboard_session(&ctx, "store_code_user", UserRole::User).await;

    let (status, generated) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/redemption-codes",
        Some(&admin),
        Some(json!({
            "reward": { "kind": "balance", "currency": "USD", "amount_minor": "100" },
            "count": 1,
            "validity_days": 30
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{generated}");
    let code = generated[0]["code"].as_str().unwrap();

    let (status, redeemed) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/redeem",
        Some(&user),
        Some(json!({ "code": code })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{redeemed}");
    assert_eq!(redeemed["status"], json!("used"));

    let (status, records) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/redemption-codes?limit=1000",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{records}");
    assert!(!serde_json::to_string(&records).unwrap().contains(code));
}

#[tokio::test]
async fn invalid_redemption_and_query_errors_use_the_store_envelope() {
    let mut ctx = setup().await;
    configure_offline_rate(&mut ctx).await;
    let user = dashboard_session(&ctx, "store_query_user", UserRole::User).await;

    let (status, body) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/redeem",
        Some(&user),
        Some(json!({ "code": "not-a-code" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], json!("invalid_redemption_code"));

    let (status, body) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/orders?limit=abc",
        Some(&user),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("invalid_request"));
}

#[tokio::test]
async fn store_json_errors_distinguish_currency_and_amount_overflow() {
    let mut ctx = setup().await;
    configure_rate(&mut ctx).await;
    let admin = dashboard_session(&ctx, "store_error_admin", UserRole::Admin).await;
    let user = dashboard_session(&ctx, "store_error_user", UserRole::User).await;

    let (_, product) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/products",
        Some(&admin),
        Some(balance_product("Error mapping recharge")),
    )
    .await;
    seed_governed_stripe(&ctx).await;
    let request = |currency: &str, custom_recharge_minor: Option<String>| {
        json!({
            "product_id": product["id"],
            "payment_channel_id": "store-channel-stripe",
            "payment_currency": currency,
            "custom_recharge_minor": custom_recharge_minor
        })
    };

    let (status, body) = idempotent_json_request(
        &ctx,
        "/api/dashboard/store/orders",
        &user,
        "invalid-currency",
        request("EUR", None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("invalid_currency"));

    let (status, body) = idempotent_json_request(
        &ctx,
        "/api/dashboard/store/orders",
        &user,
        "overflow-amount",
        request("CNY", Some("9".repeat(100))),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], json!("amount_overflow"));
}
