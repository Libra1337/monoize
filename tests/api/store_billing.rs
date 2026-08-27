use async_trait::async_trait;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN};
use axum::http::{Method, Request, StatusCode};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use monoize::store_billing::adapters::wechat::WechatCredential;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::{
    ExchangeRateFetcher, ExchangeRateService, ExchangeRateSnapshot, ExchangeRateStore,
};
use monoize::users::UserRole;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use std::sync::Arc;
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
              'sandbox', 'old-account', 'checkout', 'evidence', 'credential_admin',
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
        assert_eq!(body["error"]["code"], "invalid_store_origin");
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
    assert_eq!(body["error"]["code"], "invalid_store_origin");
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
