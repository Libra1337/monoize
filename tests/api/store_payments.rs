use aes_gcm::aead::{Aead, KeyInit as AesKeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use async_trait::async_trait;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{TimeZone, Utc};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use monoize::store_billing::adapters::alipay::{
    AlipayCheckoutResult, AlipayCredential, AlipayProduct, canonical_alipay_parameters,
};
use monoize::store_billing::adapters::stripe::{StripeCheckoutResult, StripeCredential};
use monoize::store_billing::adapters::wechat::{
    WechatCheckoutResult, WechatCredential, WechatProduct, wechat_callback_signature_message,
};
use monoize::store_billing::checkout::CheckoutProvider;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::{
    ExchangeRateFetcher, ExchangeRateService, ExchangeRateSnapshot, ExchangeRateStore,
};
use monoize::store_billing::payment::{AdapterError, CheckoutAction, CheckoutRequest};
use monoize::users::UserRole;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::rand_core::OsRng;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

use super::setup;

#[derive(Clone)]
struct OfflineRateFetcher;

#[derive(Clone, Default)]
struct ApiCheckoutProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl CheckoutProvider for ApiCheckoutProvider {
    async fn create_stripe_checkout(
        &self,
        _credential: &StripeCredential,
        _request: &CheckoutRequest,
    ) -> Result<StripeCheckoutResult, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(StripeCheckoutResult {
            provider_object_id: "cs_api_checkout".to_string(),
            action: CheckoutAction::Redirect {
                url: "https://checkout.stripe.com/c/pay_api".to_string(),
                expires_at: "2026-08-27T18:00:00Z".to_string(),
            },
        })
    }

    async fn create_alipay_checkout(
        &self,
        _credential: &AlipayCredential,
        request: &CheckoutRequest,
        _product: AlipayProduct,
        _notify_url: url::Url,
    ) -> Result<AlipayCheckoutResult, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AlipayCheckoutResult {
            provider_object_id: request.order_number.clone(),
            action: CheckoutAction::Form {
                action: "https://openapi.alipay.com/gateway.do".to_string(),
                fields: vec![("out_trade_no".to_string(), request.order_number.clone())],
                expires_at: "2026-08-27T18:00:00Z".to_string(),
            },
        })
    }

    async fn create_wechat_checkout(
        &self,
        _credential: &WechatCredential,
        request: &CheckoutRequest,
        _product: WechatProduct,
        _notify_url: url::Url,
        _client_ip: Option<std::net::IpAddr>,
    ) -> Result<WechatCheckoutResult, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(WechatCheckoutResult {
            provider_object_id: request.order_number.clone(),
            action: CheckoutAction::Qr {
                payload: "weixin://wxpay/bizpayurl?pr=api-test".to_string(),
                expires_at: "2026-08-27T18:00:00Z".to_string(),
            },
        })
    }
}

#[async_trait]
impl ExchangeRateFetcher for OfflineRateFetcher {
    async fn fetch_latest_usd(&self) -> Result<String, String> {
        Err("offline test fetcher".to_string())
    }
}

async fn configure_payment_fixture(ctx: &mut super::TestContext) {
    let snapshot = ExchangeRateSnapshot {
        base: "USD".to_string(),
        quote: "CNY".to_string(),
        cny_per_usd: "6.7370".to_string(),
        source_updated_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 0, 0).unwrap(),
        refreshed_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
    };
    let rate_store = ExchangeRateStore::new(ctx.state.db_pool.clone());
    rate_store.persist(&snapshot).await.unwrap();
    ctx.state.exchange_rate_service =
        ExchangeRateService::with_fetcher(rate_store, OfflineRateFetcher)
            .await
            .unwrap();
    let write = ctx.state.db_pool.write().await;
    write
        .execute_unprepared(
            "INSERT INTO store_products
                (id, kind, name, description, price_currency, price_minor,
                 duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
             VALUES
                ('api-payment-product', 'balance', 'Recharge', '', 'CNY', '1000',
                 NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
        )
        .await
        .unwrap();
    write
        .execute_unprepared(
            "INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
             VALUES ('api-payment-product', '1000', '0')",
        )
        .await
        .unwrap();
    write
        .execute_unprepared(
            "UPDATE store_payment_channels SET enabled = 1
             WHERE id = 'store-channel-stripe'",
        )
        .await
        .unwrap();
    write
        .execute_unprepared(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES
                ('api-payment-credential', 'store-channel-stripe', 'stripe', 1, 'key-1',
                 'bm9uY2U=', 'Y2lwaGVydGV4dA==', 'acct-digest', 'active',
                 '2026-08-27T00:00:00Z')",
        )
        .await
        .unwrap();
    drop(write);
    ctx.router = monoize::app::build_app(ctx.state.clone());
}

async fn configure_checkout_runtime(ctx: &mut super::TestContext, provider: ApiCheckoutProvider) {
    let ring = PaymentKeyRing::new(
        PaymentKey::new("api-checkout-key", [23_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let encrypted = ring
        .encrypt(
            "store_channel_credentials:api-payment-credential:secret",
            br#"{
                "secret_key":"sk_test_api",
                "publishable_key":"pk_test_api",
                "webhook_signing_secret":"whsec_api",
                "api_version":"2026-08-01",
                "account_id":"acct_api",
                "live_mode":false
            }"#,
        )
        .unwrap();
    let account_digest = Sha256::digest(b"acct_api")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ctx.state
        .db_pool
        .write()
        .await
        .execute(ctx.state.db_pool.stmt(
            "UPDATE store_channel_credentials
             SET format_version = $2, key_id = $3, nonce_base64 = $4,
                 ciphertext_base64 = $5, account_identity_digest = $6
             WHERE id = $1",
            vec![
                "api-payment-credential".into(),
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                account_digest.into(),
            ],
        ))
        .await
        .unwrap();
    ctx.state.payment_keys = Some(Arc::new(ring));
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.state.checkout_provider = Arc::new(provider);
    ctx.router = monoize::app::build_app(ctx.state.clone());
}

async fn configure_alipay_runtime(
    ctx: &mut super::TestContext,
    provider: ApiCheckoutProvider,
) -> String {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    let private_pem = private.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
    let public_pem = public.to_public_key_pem(LineEnding::LF).unwrap();
    let ring = PaymentKeyRing::new(
        PaymentKey::new("api-alipay-key", [29_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let encrypted = ring
        .encrypt(
            "store_channel_credentials:api-alipay-credential:secret",
            serde_json::json!({
                "app_id":"2026000000000001",
                "seller_id":"2088000000000001",
                "merchant_private_key_pem":private_pem,
                "alipay_public_key_pem":public_pem,
                "environment":"sandbox"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let account_digest = Sha256::digest(b"2088000000000001")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let write = ctx.state.db_pool.write().await;
    write
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ($1, 'store-channel-alipay', 'alipay', $2, $3, $4, $5, $6,
                     'active', '2026-08-27T00:00:00Z')",
            vec![
                "api-alipay-credential".into(),
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                account_digest.into(),
            ],
        ))
        .await
        .unwrap();
    write
        .execute_unprepared(
            "UPDATE store_payment_channels SET enabled = 1
             WHERE id = 'store-channel-alipay'",
        )
        .await
        .unwrap();
    drop(write);
    ctx.state.payment_keys = Some(Arc::new(ring));
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.state.checkout_provider = Arc::new(provider);
    ctx.router = monoize::app::build_app(ctx.state.clone());
    private_pem
}

async fn configure_wechat_runtime(
    ctx: &mut super::TestContext,
    provider: ApiCheckoutProvider,
) -> String {
    let platform_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let platform_public = RsaPublicKey::from(&platform_private);
    let platform_private_pem = platform_private
        .to_pkcs8_pem(LineEnding::LF)
        .unwrap()
        .to_string();
    let platform_public_pem = platform_public.to_public_key_pem(LineEnding::LF).unwrap();
    let ring = PaymentKeyRing::new(
        PaymentKey::new("api-wechat-key", [33_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let encrypted = ring
        .encrypt(
            "store_channel_credentials:api-wechat-credential:secret",
            serde_json::json!({
                "merchant_id":"1900000109",
                "app_id":"wx1234567890",
                "api_v3_key":"0123456789abcdef0123456789abcdef",
                "merchant_certificate_serial":"7777777777777777777777777777777777777777",
                "merchant_private_key_pem":platform_private_pem.as_str(),
                "platform_certificate_serial":"PLATFORM-CERTIFICATE-1",
                "platform_public_key_pem":platform_public_pem
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let account_digest = Sha256::digest(b"1900000109")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let write = ctx.state.db_pool.write().await;
    write
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ($1, 'store-channel-wechat', 'wechat', $2, $3, $4, $5, $6,
                     'active', '2026-08-27T00:00:00Z')",
            vec![
                "api-wechat-credential".into(),
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                account_digest.into(),
            ],
        ))
        .await
        .unwrap();
    write
        .execute_unprepared(
            "UPDATE store_payment_channels SET enabled = 1
             WHERE id = 'store-channel-wechat'",
        )
        .await
        .unwrap();
    drop(write);
    ctx.state.payment_keys = Some(Arc::new(ring));
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.state.checkout_provider = Arc::new(provider);
    ctx.router = monoize::app::build_app(ctx.state.clone());
    platform_private_pem
}

async fn rotate_wechat_platform_credential(ctx: &super::TestContext) -> String {
    let platform_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let platform_public = RsaPublicKey::from(&platform_private);
    let platform_private_pem = platform_private
        .to_pkcs8_pem(LineEnding::LF)
        .unwrap()
        .to_string();
    let platform_public_pem = platform_public.to_public_key_pem(LineEnding::LF).unwrap();
    let ring = PaymentKeyRing::new(
        PaymentKey::new("api-wechat-key", [33_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let encrypted = ring
        .encrypt(
            "store_channel_credentials:api-wechat-credential-rotated:secret",
            serde_json::json!({
                "merchant_id":"1900000109",
                "app_id":"wx1234567890",
                "api_v3_key":"0123456789abcdef0123456789abcdef",
                "merchant_certificate_serial":"7777777777777777777777777777777777777777",
                "merchant_private_key_pem":platform_private_pem.as_str(),
                "platform_certificate_serial":"PLATFORM-CERTIFICATE-2",
                "platform_public_key_pem":platform_public_pem
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let account_digest = Sha256::digest(b"1900000109")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let write = ctx.state.db_pool.write().await;
    write
        .execute_unprepared(
            "UPDATE store_channel_credentials SET status = 'retired'
             WHERE id = 'api-wechat-credential'",
        )
        .await
        .unwrap();
    write
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ('api-wechat-credential-rotated', 'store-channel-wechat', 'wechat',
                     $1, $2, $3, $4, $5, 'active', '2026-08-27T00:01:00Z')",
            vec![
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                account_digest.into(),
            ],
        ))
        .await
        .unwrap();
    platform_private_pem
}

async fn session(ctx: &super::TestContext, username: &str) -> String {
    let user = ctx
        .state
        .user_store
        .create_user(username, "test-password", UserRole::User, None)
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
    authorization: &str,
    idempotency_key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(AUTHORIZATION, authorization);
    if let Some(key) = idempotency_key {
        builder = builder.header("Idempotency-Key", key);
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, value)
}

fn stripe_signature(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut signed = timestamp.to_string().into_bytes();
    signed.push(b'.');
    signed.extend_from_slice(body);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    mac.update(&signed);
    let signature = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("t={timestamp},v1={signature}")
}

async fn stripe_callback_request(
    ctx: &super::TestContext,
    body: &[u8],
    signature: &str,
) -> (StatusCode, Value) {
    let response = ctx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/store/callbacks/store-channel-stripe")
                .header("Stripe-Signature", signature)
                .header(
                    "User-Agent",
                    "Stripe/1.0 (+https://stripe.com/docs/webhooks)",
                )
                .body(Body::from(body.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, value)
}

async fn alipay_callback_request(ctx: &super::TestContext, body: &str) -> (StatusCode, String) {
    let response = ctx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/store/callbacks/store-channel-alipay")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn wechat_callback_request(
    ctx: &super::TestContext,
    body: &[u8],
    timestamp: &str,
    nonce: &str,
    certificate_serial: &str,
    signature: &str,
) -> (StatusCode, Value) {
    let response = ctx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/store/callbacks/store-channel-wechat")
                .header(CONTENT_TYPE, "application/json")
                .header("wechatpay-timestamp", timestamp)
                .header("wechatpay-nonce", nonce)
                .header("wechatpay-serial", certificate_serial)
                .header("wechatpay-signature", signature)
                .body(Body::from(body.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn payment_order_api_requires_idempotency_and_persists_attempt_first() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    let user = session(&ctx, "payment-api-user").await;
    let request = json!({
        "product_id": "api-payment-product",
        "payment_channel_id": "store-channel-stripe",
        "payment_currency": "CNY",
        "custom_recharge_minor": null
    });

    let (status, error) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        None,
        Some(request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "missing_idempotency_key");

    let (status, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("checkout-api-1"),
        Some(request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{order}");
    assert_eq!(order["payment_state"], "unpaid");
    let order_id = order["id"].as_str().unwrap();

    let (status, replay) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("checkout-api-1"),
        Some(request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["id"], order["id"]);

    let (status, error) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/orders/{order_id}/attempts"),
        &user,
        Some("attempt-api-1"),
        Some(json!({"expected_payment_method":"card"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{error}");
    assert_eq!(error["error"]["code"], "payment_configuration_unavailable");
    let persisted = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT order_id, state FROM store_payment_attempts WHERE idempotency_key = $1",
            vec!["attempt-api-1".into()],
        ))
        .await
        .unwrap()
        .expect("attempt is persisted before runtime configuration validation");
    assert_eq!(
        persisted.try_get::<String>("", "order_id").unwrap(),
        order_id
    );
    assert_eq!(persisted.try_get::<String>("", "state").unwrap(), "failed");
}

#[tokio::test]
async fn payment_order_api_is_user_scoped_and_has_no_manual_complete_route() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    let owner = session(&ctx, "payment-owner").await;
    let other = session(&ctx, "payment-other").await;
    let (_, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &owner,
        Some("checkout-owner-1"),
        Some(json!({
            "product_id": "api-payment-product",
            "payment_channel_id": "store-channel-stripe",
            "payment_currency": "CNY"
        })),
    )
    .await;
    let order_id = order["id"].as_str().unwrap();

    let (status, _) = json_request(
        &ctx,
        Method::GET,
        &format!("/api/dashboard/store/orders/{order_id}"),
        &other,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/admin/orders/{order_id}/complete"),
        &owner,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stripe_callback_is_public_verified_encrypted_and_idempotent() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    configure_checkout_runtime(&mut ctx, ApiCheckoutProvider::default()).await;
    let user = session(&ctx, "stripe-callback-user").await;
    let (_, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("stripe-callback-order"),
        Some(json!({
            "product_id": "api-payment-product",
            "payment_channel_id": "store-channel-stripe",
            "payment_currency": "CNY"
        })),
    )
    .await;
    let order_id = order["id"].as_str().unwrap();
    let order_number = order["order_number"].as_str().unwrap();
    let (status, checkout) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/orders/{order_id}/attempts"),
        &user,
        Some("stripe-callback-attempt"),
        Some(json!({"expected_payment_method":"card"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{checkout}");
    let attempt_id = checkout["attempt"]["id"].as_str().unwrap();
    let event = json!({
        "id": "evt_store_paid_1",
        "object": "event",
        "api_version": "2026-08-01",
        "type": "checkout.session.completed",
        "account": "acct_api",
        "data": {"object": {
            "id": "cs_api_checkout",
            "object": "checkout.session",
            "amount_total": 1000,
            "currency": "cny",
            "client_reference_id": order_number,
            "metadata": {"store_attempt_id": attempt_id},
            "payment_intent": "pi_store_paid_1",
            "payment_status": "paid",
            "status": "complete"
        }}
    });
    let body = serde_json::to_vec(&event).unwrap();
    let timestamp = Utc::now().timestamp();
    let signature = stripe_signature(b"whsec_api", timestamp, &body);

    let (status, error) =
        stripe_callback_request(&ctx, &body, &format!("t={timestamp},v1={}", "0".repeat(64))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "invalid_payment_callback");

    let mut mismatched_event = event.clone();
    mismatched_event["id"] = json!("evt_store_amount_mismatch");
    mismatched_event["data"]["object"]["amount_total"] = json!(999);
    let mismatched_body = serde_json::to_vec(&mismatched_event).unwrap();
    let mismatched_signature = stripe_signature(b"whsec_api", timestamp, &mismatched_body);
    let (status, response) =
        stripe_callback_request(&ctx, &mismatched_body, &mismatched_signature).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response, json!({"received": true}));

    for _ in 0..2 {
        let (status, response) = stripe_callback_request(&ctx, &body, &signature).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response, json!({"received": true}));
    }

    let row = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT o.payment_state, o.fulfillment_state,
                    e.raw_key_id, e.raw_nonce_base64, e.raw_ciphertext_base64
             FROM store_orders o
             JOIN store_provider_events e ON e.provider_event_id = $2
             WHERE o.id = $1",
            vec![order_id.into(), "evt_store_paid_1".into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "payment_state").unwrap(), "paid");
    assert_eq!(
        row.try_get::<String>("", "fulfillment_state").unwrap(),
        "fulfilled"
    );
    assert!(!row.try_get::<String>("", "raw_key_id").unwrap().is_empty());
    assert!(
        !row.try_get::<String>("", "raw_nonce_base64")
            .unwrap()
            .is_empty()
    );
    assert!(
        !row.try_get::<String>("", "raw_ciphertext_base64")
            .unwrap()
            .is_empty()
    );
    let ledger_count: i64 = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger
             WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{order_id}").into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    assert_eq!(ledger_count, 1);
}

#[tokio::test]
async fn alipay_callback_returns_success_after_verified_idempotent_fulfillment() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    let private_pem = configure_alipay_runtime(&mut ctx, ApiCheckoutProvider::default()).await;
    let user = session(&ctx, "alipay-callback-user").await;
    let (_, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("alipay-callback-order"),
        Some(json!({
            "product_id": "api-payment-product",
            "payment_channel_id": "store-channel-alipay",
            "payment_currency": "CNY"
        })),
    )
    .await;
    let order_id = order["id"].as_str().unwrap();
    let order_number = order["order_number"].as_str().unwrap();
    let (status, checkout) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/orders/{order_id}/attempts"),
        &user,
        Some("alipay-callback-attempt"),
        Some(json!({"expected_payment_method":"computer_web"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{checkout}");
    let attempt_id = checkout["attempt"]["id"].as_str().unwrap();
    ctx.state
        .db_pool
        .write()
        .await
        .execute(ctx.state.db_pool.stmt(
            "UPDATE store_payment_attempts
             SET state = 'created', provider_object_id = NULL
             WHERE id = $1",
            vec![attempt_id.into()],
        ))
        .await
        .unwrap();

    let mut fields = BTreeMap::from([
        ("notify_id".to_string(), "notify-api-alipay-1".to_string()),
        ("app_id".to_string(), "2026000000000001".to_string()),
        ("seller_id".to_string(), "2088000000000001".to_string()),
        ("out_trade_no".to_string(), order_number.to_string()),
        ("trade_no".to_string(), "2026082722001002".to_string()),
        ("trade_status".to_string(), "TRADE_SUCCESS".to_string()),
        ("total_amount".to_string(), "10.00".to_string()),
        ("charset".to_string(), "utf-8".to_string()),
        ("sign_type".to_string(), "RSA2".to_string()),
    ]);
    let canonical = canonical_alipay_parameters(&fields);
    fields.insert(
        "sign".to_string(),
        monoize::store_billing::crypto::sign_rsa_sha256_base64(&private_pem, canonical.as_bytes())
            .unwrap(),
    );
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter())
        .finish();

    for _ in 0..2 {
        let (status, response) = alipay_callback_request(&ctx, &body).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response, "success");
    }
    fields.insert(
        "notify_id".to_string(),
        "notify-api-alipay-mismatch".to_string(),
    );
    fields.insert("total_amount".to_string(), "10.01".to_string());
    let mismatched_canonical = canonical_alipay_parameters(&fields);
    fields.insert(
        "sign".to_string(),
        monoize::store_billing::crypto::sign_rsa_sha256_base64(
            &private_pem,
            mismatched_canonical.as_bytes(),
        )
        .unwrap(),
    );
    let mismatched_body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter())
        .finish();
    for _ in 0..2 {
        let (status, _) = alipay_callback_request(&ctx, &mismatched_body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let row = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT o.payment_state, o.fulfillment_state,
                    a.state AS attempt_state, a.provider_object_id
             FROM store_orders o
             JOIN store_payment_attempts a ON a.order_id = o.id
             WHERE o.id = $1",
            vec![order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "payment_state").unwrap(), "paid");
    assert_eq!(
        row.try_get::<String>("", "fulfillment_state").unwrap(),
        "fulfilled"
    );
    assert_eq!(row.try_get::<String>("", "attempt_state").unwrap(), "paid");
    assert_eq!(
        row.try_get::<String>("", "provider_object_id").unwrap(),
        order_number
    );
    let ledger_count: i64 = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger
             WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{order_id}").into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    assert_eq!(ledger_count, 1);
}

#[tokio::test]
async fn wechat_callback_returns_official_success_after_verified_idempotent_fulfillment() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    configure_wechat_runtime(&mut ctx, ApiCheckoutProvider::default()).await;
    let user = session(&ctx, "wechat-callback-user").await;
    let (_, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("wechat-callback-order"),
        Some(json!({
            "product_id": "api-payment-product",
            "payment_channel_id": "store-channel-wechat",
            "payment_currency": "CNY"
        })),
    )
    .await;
    let order_id = order["id"].as_str().unwrap();
    let order_number = order["order_number"].as_str().unwrap();
    let (status, checkout) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/orders/{order_id}/attempts"),
        &user,
        Some("wechat-callback-attempt"),
        Some(json!({"expected_payment_method":"native"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{checkout}");
    let attempt_id = checkout["attempt"]["id"].as_str().unwrap();
    ctx.state
        .db_pool
        .write()
        .await
        .execute(ctx.state.db_pool.stmt(
            "UPDATE store_payment_attempts
             SET state = 'created', provider_object_id = NULL
             WHERE id = $1",
            vec![attempt_id.into()],
        ))
        .await
        .unwrap();
    let platform_private_pem = rotate_wechat_platform_credential(&ctx).await;

    let resource_nonce = *b"0123456789ab";
    let resource = serde_json::to_vec(&json!({
        "appid":"wx1234567890",
        "mchid":"1900000109",
        "out_trade_no":order_number,
        "transaction_id":"4200000001202608270002",
        "trade_state":"SUCCESS",
        "amount":{"total":1000,"currency":"CNY"}
    }))
    .unwrap();
    let encrypted = Aes256Gcm::new_from_slice(b"0123456789abcdef0123456789abcdef")
        .unwrap()
        .encrypt(
            &Nonce::try_from(resource_nonce.as_slice()).unwrap(),
            Payload {
                msg: &resource,
                aad: b"transaction",
            },
        )
        .unwrap();
    let body = serde_json::to_vec(&json!({
        "id":"event-api-wechat-1",
        "event_type":"TRANSACTION.SUCCESS",
        "resource":{
            "original_type":"transaction",
            "algorithm":"AEAD_AES_256_GCM",
            "ciphertext":STANDARD.encode(encrypted),
            "associated_data":"transaction",
            "nonce":"0123456789ab"
        }
    }))
    .unwrap();
    let timestamp = Utc::now().timestamp().to_string();
    let nonce = "callback-nonce-api-1";
    let signature = monoize::store_billing::crypto::sign_rsa_sha256_base64(
        &platform_private_pem,
        &wechat_callback_signature_message(&timestamp, nonce, &body),
    )
    .unwrap();

    for _ in 0..2 {
        let (status, response) = wechat_callback_request(
            &ctx,
            &body,
            &timestamp,
            nonce,
            "PLATFORM-CERTIFICATE-2",
            &signature,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response, json!({"code":"SUCCESS","message":"成功"}));
    }
    let mismatched_resource = serde_json::to_vec(&json!({
        "appid":"wx1234567890",
        "mchid":"1900000109",
        "out_trade_no":order_number,
        "transaction_id":"4200000001202608270003",
        "trade_state":"SUCCESS",
        "amount":{"total":1001,"currency":"CNY"}
    }))
    .unwrap();
    let mismatched_resource_nonce = *b"0123456789ac";
    let mismatched_encrypted = Aes256Gcm::new_from_slice(b"0123456789abcdef0123456789abcdef")
        .unwrap()
        .encrypt(
            &Nonce::try_from(mismatched_resource_nonce.as_slice()).unwrap(),
            Payload {
                msg: &mismatched_resource,
                aad: b"transaction",
            },
        )
        .unwrap();
    let mismatched_body = serde_json::to_vec(&json!({
        "id":"event-api-wechat-mismatch",
        "event_type":"TRANSACTION.SUCCESS",
        "resource":{
            "original_type":"transaction",
            "algorithm":"AEAD_AES_256_GCM",
            "ciphertext":STANDARD.encode(mismatched_encrypted),
            "associated_data":"transaction",
            "nonce":"0123456789ac"
        }
    }))
    .unwrap();
    let mismatched_nonce = "callback-nonce-api-2";
    let mismatched_signature = monoize::store_billing::crypto::sign_rsa_sha256_base64(
        &platform_private_pem,
        &wechat_callback_signature_message(&timestamp, mismatched_nonce, &mismatched_body),
    )
    .unwrap();
    for _ in 0..2 {
        let (status, _) = wechat_callback_request(
            &ctx,
            &mismatched_body,
            &timestamp,
            mismatched_nonce,
            "PLATFORM-CERTIFICATE-2",
            &mismatched_signature,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let manual_review = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT projection_state FROM store_provider_events
             WHERE credential_version_id = 'api-wechat-credential'
               AND provider_event_id = 'event-api-wechat-mismatch'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        manual_review
            .try_get::<String>("", "projection_state")
            .unwrap(),
        "manual_review"
    );
    let row = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT o.payment_state, o.fulfillment_state,
                    a.state AS attempt_state, a.provider_object_id
             FROM store_orders o
             JOIN store_payment_attempts a ON a.order_id = o.id
             WHERE o.id = $1",
            vec![order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "payment_state").unwrap(), "paid");
    assert_eq!(
        row.try_get::<String>("", "fulfillment_state").unwrap(),
        "fulfilled"
    );
    assert_eq!(row.try_get::<String>("", "attempt_state").unwrap(), "paid");
    assert_eq!(
        row.try_get::<String>("", "provider_object_id").unwrap(),
        order_number
    );
    let ledger_count: i64 = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger
             WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{order_id}").into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    assert_eq!(ledger_count, 1);
}

#[tokio::test]
async fn stripe_callback_rejects_a_body_larger_than_128_kib() {
    let ctx = setup().await;
    let body = vec![b'x'; 131_073];
    let timestamp = Utc::now().timestamp();
    let signature = stripe_signature(b"irrelevant", timestamp, &body);

    let (status, error) = stripe_callback_request(&ctx, &body, &signature).await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{error}");
    assert_eq!(error["error"]["code"], "callback_body_too_large");
}

#[tokio::test]
async fn payment_attempt_api_returns_and_replays_the_persisted_checkout_action() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    let provider = ApiCheckoutProvider::default();
    configure_checkout_runtime(&mut ctx, provider.clone()).await;
    let user = session(&ctx, "payment-checkout-api-user").await;
    let (_, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("checkout-action-order"),
        Some(json!({
            "product_id": "api-payment-product",
            "payment_channel_id": "store-channel-stripe",
            "payment_currency": "CNY"
        })),
    )
    .await;
    let order_id = order["id"].as_str().unwrap();

    let (status, created) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/orders/{order_id}/attempts"),
        &user,
        Some("checkout-action-attempt"),
        Some(json!({"expected_payment_method":"card"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["attempt"]["state"], "presented");
    assert_eq!(created["action"]["kind"], "redirect");
    assert_eq!(
        created["action"]["url"],
        "https://checkout.stripe.com/c/pay_api"
    );

    let (status, replay) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/orders/{order_id}/attempts"),
        &user,
        Some("checkout-action-attempt"),
        Some(json!({"expected_payment_method":"card"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay, created);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn payment_order_polling_is_limited_to_thirty_requests_per_user_per_minute() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    let user = session(&ctx, "payment-poll-user").await;
    let (_, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("checkout-poll-order"),
        Some(json!({
            "product_id": "api-payment-product",
            "payment_channel_id": "store-channel-stripe",
            "payment_currency": "CNY"
        })),
    )
    .await;
    let path = format!(
        "/api/dashboard/store/orders/{}",
        order["id"].as_str().unwrap()
    );

    for _ in 0..30 {
        let (status, _) = json_request(&ctx, Method::GET, &path, &user, None, None).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, error) = json_request(&ctx, Method::GET, &path, &user, None, None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{error}");
    assert_eq!(error["error"]["code"], "order_poll_rate_limited");
}
