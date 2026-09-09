use async_trait::async_trait;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use chrono::{TimeZone, Utc};
use hmac::{Hmac, KeyInit, Mac};
use http_body_util::BodyExt;
use monoize::store_billing::adapters::epay::{
    EpayCheckoutResult, EpayCredential, EpayDevice, EpayMethod, sign_epay_parameters,
};
use monoize::store_billing::adapters::stripe::{StripeCheckoutResult, StripeCredential};
use monoize::store_billing::checkout::CheckoutProvider;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::{
    ExchangeRateFetcher, ExchangeRateService, ExchangeRateSnapshot, ExchangeRateStore,
};
use monoize::store_billing::operations::PaymentQueryProvider;
use monoize::store_billing::payment::{
    AdapterError, CheckoutAction, CheckoutRequest, PaymentQuery, ProviderPaymentState,
    ProviderRefundState,
};
use monoize::store_billing::refund_operations::{
    RefundProvider, RefundProviderContract, RefundProviderOutcome,
};
use monoize::users::UserRole;
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

#[derive(Clone)]
struct ApiPaymentQueryProvider {
    calls: Arc<AtomicUsize>,
    outcome: Result<ProviderPaymentState, AdapterError>,
}

#[derive(Clone)]
struct ApiRefundProvider {
    create_calls: Arc<AtomicUsize>,
    query_calls: Arc<AtomicUsize>,
    create_state: ProviderRefundState,
    query_state: ProviderRefundState,
}

impl ApiRefundProvider {
    fn new(create_state: ProviderRefundState, query_state: ProviderRefundState) -> Self {
        Self {
            create_calls: Arc::new(AtomicUsize::new(0)),
            query_calls: Arc::new(AtomicUsize::new(0)),
            create_state,
            query_state,
        }
    }

    fn outcome(state: ProviderRefundState) -> RefundProviderOutcome {
        RefundProviderOutcome {
            provider_refund_id: Some("re_api_refund".to_string()),
            not_found_is_definitive: false,
            state,
        }
    }
}

#[async_trait]
impl RefundProvider for ApiRefundProvider {
    async fn create_refund(
        &self,
        _contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Self::outcome(self.create_state.clone()))
    }

    async fn query_refund(
        &self,
        _contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        self.query_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Self::outcome(self.query_state.clone()))
    }
}

impl ApiPaymentQueryProvider {
    fn returning(state: ProviderPaymentState) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            outcome: Ok(state),
        }
    }

    fn failing(error: AdapterError) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            outcome: Err(error),
        }
    }
}

#[async_trait]
impl PaymentQueryProvider for ApiPaymentQueryProvider {
    async fn query_stripe_payment(
        &self,
        _credential: &StripeCredential,
        _query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.outcome.clone()
    }

    async fn query_epay_payment(
        &self,
        _credential: &EpayCredential,
        _query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.outcome.clone()
    }
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

    async fn create_epay_checkout(
        &self,
        _credential: &EpayCredential,
        request: &CheckoutRequest,
        method: EpayMethod,
        _notify_url: url::Url,
        _device: EpayDevice,
        _client_ip: Option<std::net::IpAddr>,
    ) -> Result<EpayCheckoutResult, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(EpayCheckoutResult {
            provider_object_id: request.order_number.clone(),
            action: CheckoutAction::Qr {
                payload: format!("epay://{}/{}", method.as_str(), request.order_number),
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
                 'bm9uY2U=', 'Y2lwaGVydGV4dA==',
                 '3333333333333333333333333333333333333333333333333333333333333333',
                 'active',
                 '2026-08-27T00:00:00Z')",
        )
        .await
        .unwrap();
    drop(write);
    seed_payment_governance(
        ctx,
        "store-channel-stripe",
        "3333333333333333333333333333333333333333333333333333333333333333",
    )
    .await;
    ctx.router = monoize::app::build_app(ctx.state.clone());
}

async fn seed_payment_governance(
    ctx: &super::TestContext,
    channel_id: &str,
    merchant_account_digest: &str,
) {
    let write = ctx.state.db_pool.write().await;
    write
        .execute(ctx.state.db_pool.stmt(
            "DELETE FROM store_payment_compliance WHERE channel_id = $1",
            vec![channel_id.into()],
        ))
        .await
        .unwrap();
    write
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_payment_compliance
                (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
             VALUES ($1, $2, '2026-08-28', 'payment-test-admin', '127.0.0.1',
                     '2026-08-28T00:00:00Z')",
            vec![format!("{channel_id}-compliance").into(), channel_id.into()],
        ))
        .await
        .unwrap();
    for capability in [
        "payment_query",
        "refund",
        "refund_query",
        "settlement_report",
    ] {
        write
            .execute(ctx.state.db_pool.stmt(
                "INSERT INTO store_merchant_capabilities
                    (id, channel_id, capability, state, environment, merchant_account_digest,
                     provider_product, evidence_digest, verifier_admin_id, verified_at, expires_at)
                 VALUES ($1, $2, $3, 'supported', 'sandbox', $4, 'checkout',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                         'payment-test-admin', '2026-08-28T00:00:00Z',
                         '2099-01-01T00:00:00Z')
                 ON CONFLICT (channel_id, capability) DO UPDATE SET
                    merchant_account_digest = excluded.merchant_account_digest,
                    expires_at = excluded.expires_at",
                vec![
                    format!("{channel_id}-{capability}").into(),
                    channel_id.into(),
                    capability.into(),
                    merchant_account_digest.into(),
                ],
            ))
            .await
            .unwrap();
    }
    let (currencies, limits, actions) = match channel_id {
        "store-channel-epay" => (
            "[\"CNY\"]",
            "{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}",
            "[\"qr\",\"redirect\"]",
        ),
        _ => (
            "[\"CNY\",\"USD\"]",
            "{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}",
            "[\"redirect\"]",
        ),
    };
    let privacy_id = format!("{channel_id}-privacy");
    write
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ($1, 'v1', 'CN', '[]', '{}', 'contract', 'payment-test-admin',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z', 1)
             ON CONFLICT (id) DO UPDATE SET accepted = 1,
                 next_review_at = excluded.next_review_at",
            vec![privacy_id.clone().into()],
        ))
        .await
        .unwrap();
    write
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_channel_readiness_profiles
                (channel_id, active_credential_digest, privacy_record_id,
                 callback_verification_passed, supported_currencies_json, amount_limits_json,
                 checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                 availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES ($1, $2, $3, 1, $4, $5, $6,
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'payment-test-admin', '2026-08-28T00:00:00Z',
                     '2099-01-01T00:00:00Z')
             ON CONFLICT (channel_id) DO UPDATE SET
                 active_credential_digest = excluded.active_credential_digest,
                 privacy_record_id = excluded.privacy_record_id,
                 callback_verification_passed = excluded.callback_verification_passed,
                 supported_currencies_json = excluded.supported_currencies_json,
                 amount_limits_json = excluded.amount_limits_json,
                 checkout_action_kinds_json = excluded.checkout_action_kinds_json,
                 expires_at = excluded.expires_at",
            vec![
                channel_id.into(),
                merchant_account_digest.into(),
                privacy_id.into(),
                currencies.into(),
                limits.into(),
                actions.into(),
            ],
        ))
        .await
        .unwrap();
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
                account_digest.clone().into(),
            ],
        ))
        .await
        .unwrap();
    seed_payment_governance(ctx, "store-channel-stripe", &account_digest).await;
    ctx.state.payment_keys = Some(Arc::new(ring));
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.state.checkout_provider = Arc::new(provider);
    ctx.router = monoize::app::build_app(ctx.state.clone());
}

const API_EPAY_MERCHANT_KEY: &str = "89unJUB8HZ54Hj7x4nUj56HN4nUzUJ8i";
const API_EPAY_MERCHANT_ID: &str = "1001";

async fn configure_epay_runtime(ctx: &mut super::TestContext, provider: ApiCheckoutProvider) {
    let ring = PaymentKeyRing::new(
        PaymentKey::new("api-epay-key", [29_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let credential_json = serde_json::json!({
        "gateway_base_url": "https://pay.example.com/",
        "merchant_id": API_EPAY_MERCHANT_ID,
        "merchant_key": API_EPAY_MERCHANT_KEY,
        "alipay_enabled": true,
        "wxpay_enabled": true,
    })
    .to_string();
    let encrypted = ring
        .encrypt(
            "store_channel_credentials:api-epay-credential:secret",
            credential_json.as_bytes(),
        )
        .unwrap();
    let account_digest = EpayCredential::from_json(credential_json.as_bytes())
        .unwrap()
        .account_identity_digest();
    let write = ctx.state.db_pool.write().await;
    write
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ($1, 'store-channel-epay', 'epay', $2, $3, $4, $5, $6,
                     'active', '2026-08-27T00:00:00Z')",
            vec![
                "api-epay-credential".into(),
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                account_digest.clone().into(),
            ],
        ))
        .await
        .unwrap();
    write
        .execute_unprepared(
            "UPDATE store_payment_channels SET enabled = 1
             WHERE id = 'store-channel-epay'",
        )
        .await
        .unwrap();
    write
        .execute_unprepared(
            "UPDATE store_epay_methods SET enabled = 1
             WHERE channel_id = 'store-channel-epay'",
        )
        .await
        .unwrap();
    drop(write);
    seed_payment_governance(ctx, "store-channel-epay", &account_digest).await;
    ctx.state.payment_keys = Some(Arc::new(ring));
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.state.checkout_provider = Arc::new(provider);
    ctx.router = monoize::app::build_app(ctx.state.clone());
}

/// Builds one signed EPay notification query string. An empty override value removes the field.
fn signed_epay_callback(overrides: &[(&str, &str)]) -> String {
    let mut parameters = BTreeMap::from([
        ("pid".to_string(), API_EPAY_MERCHANT_ID.to_string()),
        ("type".to_string(), "alipay".to_string()),
        ("name".to_string(), "Recharge".to_string()),
        ("money".to_string(), "10.00".to_string()),
        ("trade_status".to_string(), "TRADE_SUCCESS".to_string()),
    ]);
    for (key, value) in overrides {
        if value.is_empty() {
            parameters.remove(*key);
        } else {
            parameters.insert((*key).to_string(), (*value).to_string());
        }
    }
    let signature = sign_epay_parameters(&parameters, API_EPAY_MERCHANT_KEY);
    parameters.insert("sign".to_string(), signature);
    parameters.insert("sign_type".to_string(), "MD5".to_string());
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(parameters.iter())
        .finish()
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

async fn admin_session(ctx: &super::TestContext, username: &str) -> String {
    let user = ctx
        .state
        .user_store
        .create_user(username, "test-password", UserRole::Admin, None)
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

async fn raw_json_request(
    ctx: &super::TestContext,
    method: Method,
    path: &str,
    authorization: Option<&str>,
    body: Option<&str>,
) -> axum::response::Response {
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
    ctx.router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

async fn refund_request(
    router: &axum::Router,
    method: Method,
    path: &str,
    authorization: Option<&str>,
    reauth_token: Option<&str>,
    idempotency_key: Option<&str>,
    origin: Option<&str>,
    cookie: Option<&str>,
    body: &str,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(CONTENT_TYPE, "application/json");
    if let Some(authorization) = authorization {
        builder = builder.header(AUTHORIZATION, authorization);
    }
    if let Some(reauth_token) = reauth_token {
        builder = builder.header("X-Store-Reauth-Token", reauth_token);
    }
    if let Some(idempotency_key) = idempotency_key {
        builder = builder.header("Idempotency-Key", idempotency_key);
    }
    if let Some(origin) = origin {
        builder = builder.header("Origin", origin);
    }
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, value)
}

async fn issue_refund_reauth(ctx: &super::TestContext, admin: &str) -> String {
    let (status, grant) = json_request(
        ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        admin,
        None,
        Some(json!({
            "current_password": "test-password",
            "scope": "refund"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    grant["token"].as_str().unwrap().to_string()
}

async fn issue_reprocess_reauth(ctx: &super::TestContext, admin: &str) -> String {
    let (status, grant) = json_request(
        ctx,
        Method::POST,
        "/api/dashboard/store/admin/reauth",
        admin,
        None,
        Some(json!({
            "current_password": "test-password",
            "scope": "reprocess"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    grant["token"].as_str().unwrap().to_string()
}

fn assert_no_store(response: &axum::response::Response) {
    assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
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

async fn epay_callback_request(ctx: &super::TestContext, query: &str) -> (StatusCode, String) {
    let response = ctx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/store/callbacks/store-channel-epay?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
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
async fn epay_callback_rejects_a_method_mismatch_without_changing_financial_state() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    configure_epay_runtime(&mut ctx, ApiCheckoutProvider::default()).await;
    let user = session(&ctx, "epay-method-mismatch-user").await;
    let (_, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("epay-method-mismatch-order"),
        Some(json!({
            "product_id": "api-payment-product",
            "payment_channel_id": "store-channel-epay",
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
        Some("epay-method-mismatch-attempt"),
        Some(json!({"expected_payment_method":"alipay"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{checkout}");

    // The notification claims WeChat while the attempt expects Alipay.
    let query = signed_epay_callback(&[
        ("out_trade_no", order_number),
        ("trade_no", "2026082722001003"),
        ("type", "wxpay"),
    ]);
    for _ in 0..2 {
        let (status, _) = epay_callback_request(&ctx, &query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let event = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value, MIN(credential_version_id) AS credential_version_id,
                    MIN(projection_state) AS projection_state,
                    MIN(raw_key_id) AS raw_key_id, MIN(parsed_json) AS parsed_json
             FROM store_provider_events
             WHERE provider_event_id = '2026082722001003'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.try_get::<i64>("", "value").unwrap(), 1);
    assert_eq!(
        event
            .try_get::<String>("", "credential_version_id")
            .unwrap(),
        "api-epay-credential"
    );
    assert_eq!(
        event.try_get::<String>("", "projection_state").unwrap(),
        "manual_review"
    );
    assert!(
        !event
            .try_get::<String>("", "raw_key_id")
            .unwrap()
            .is_empty()
    );
    let parsed: Value =
        serde_json::from_str(&event.try_get::<String>("", "parsed_json").unwrap()).unwrap();
    assert_eq!(parsed["order_number"], order_number);
    assert_eq!(parsed["method"], "wxpay");
    assert!(parsed.get("sign").is_none());
    let application_count = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value FROM store_order_event_applications",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap();
    assert_eq!(application_count, 0);
    let row = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT payment_state, fulfillment_state FROM store_orders WHERE id = $1",
            vec![order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "payment_state").unwrap(),
        "unpaid"
    );
    assert_eq!(
        row.try_get::<String>("", "fulfillment_state").unwrap(),
        "pending"
    );
}

#[tokio::test]
async fn epay_callback_returns_success_after_verified_idempotent_fulfillment() {
    let mut ctx = setup().await;
    configure_payment_fixture(&mut ctx).await;
    configure_epay_runtime(&mut ctx, ApiCheckoutProvider::default()).await;
    let user = session(&ctx, "epay-callback-user").await;
    let (_, order) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some("epay-callback-order"),
        Some(json!({
            "product_id": "api-payment-product",
            "payment_channel_id": "store-channel-epay",
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
        Some("epay-callback-attempt"),
        Some(json!({"expected_payment_method":"alipay"})),
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
    ctx.state
        .db_pool
        .write()
        .await
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_payment_attempts
                (id, order_id, channel_id, adapter_kind, credential_version_id,
                 merchant_account_identity, expected_payment_method,
                 payment_contract_version, state, idempotency_key, created_at, updated_at)
             SELECT 'stale-epay-attempt', order_id, channel_id, adapter_kind,
                    credential_version_id, merchant_account_identity,
                    expected_payment_method, payment_contract_version, 'expired',
                    'stale-epay-attempt-key', '2026-08-26T23:59:00Z',
                    '2026-08-26T23:59:00Z'
             FROM store_payment_attempts WHERE id = $1",
            vec![attempt_id.into()],
        ))
        .await
        .unwrap();

    // SB-EP-3A: the gateway reports a whole-yuan order as `10`, not `10.00`. The amount check
    // must accept that form, or the callback is rejected for every whole-yuan recharge while
    // an amount that happens to carry fen still settles.
    let query = signed_epay_callback(&[
        ("out_trade_no", order_number),
        ("trade_no", "2026082722001002"),
        ("money", "10"),
    ]);
    for _ in 0..2 {
        let (status, response) = epay_callback_request(&ctx, &query).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response, "success");
    }

    // A second candidate attempt for the same order makes the binding ambiguous.
    ctx.state
        .db_pool
        .write()
        .await
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_payment_attempts
                (id, order_id, channel_id, adapter_kind, credential_version_id,
                 merchant_account_identity, expected_payment_method,
                 payment_contract_version, state, failure_kind, idempotency_key,
                 created_at, updated_at)
             SELECT 'ambiguous-epay-attempt', order_id, channel_id, adapter_kind,
                    credential_version_id, merchant_account_identity,
                    expected_payment_method, payment_contract_version, 'failed',
                    'provider_rejected', 'ambiguous-epay-attempt-key',
                    '2026-08-27T00:00:02Z', '2026-08-27T00:00:02Z'
             FROM store_payment_attempts WHERE id = $1",
            vec![attempt_id.into()],
        ))
        .await
        .unwrap();
    let ambiguous_query = signed_epay_callback(&[
        ("out_trade_no", order_number),
        ("trade_no", "2026082722001004"),
    ]);
    for _ in 0..2 {
        let (status, _) = epay_callback_request(&ctx, &ambiguous_query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let ambiguous_event = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value, MIN(projection_state) AS projection_state
             FROM store_provider_events
             WHERE credential_version_id = 'api-epay-credential'
               AND provider_event_id = '2026082722001004'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ambiguous_event.try_get::<i64>("", "value").unwrap(), 1);
    assert_eq!(
        ambiguous_event
            .try_get::<String>("", "projection_state")
            .unwrap(),
        "manual_review"
    );

    // A verified notification whose amount differs from the frozen quote must not apply.
    let mismatched_query = signed_epay_callback(&[
        ("out_trade_no", order_number),
        ("trade_no", "2026082722001005"),
        ("money", "10.01"),
    ]);
    for _ in 0..2 {
        let (status, _) = epay_callback_request(&ctx, &mismatched_query).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // A tampered signature must fail authentication and record nothing.
    let tampered_query = signed_epay_callback(&[
        ("out_trade_no", order_number),
        ("trade_no", "2026082722001006"),
    ])
    .replace("money=10.00", "money=99.00");
    let (status, _) = epay_callback_request(&ctx, &tampered_query).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let tampered_count: i64 = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE provider_event_id = '2026082722001006'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    assert_eq!(tampered_count, 0);

    let row = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT o.payment_state, o.fulfillment_state,
                    a.state AS attempt_state, a.provider_object_id
             FROM store_orders o
             JOIN store_payment_attempts a ON a.order_id = o.id
             WHERE o.id = $1 AND a.id = $2",
            vec![order_id.into(), attempt_id.into()],
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

#[tokio::test]
async fn admin_order_operation_routes_enforce_auth_origin_primary_and_no_manual_complete() {
    let mut ctx = setup().await;
    let admin = admin_session(&ctx, "admin_order_route_admin").await;
    let user = session(&ctx, "admin_order_route_user").await;
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.router = monoize::app::build_app(ctx.state.clone());

    let (status, _) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/orders/missing",
        "",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/orders/missing",
        &user,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, error) = json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/orders/missing",
        &admin,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");

    let session_token = admin.strip_prefix("Bearer ").unwrap();
    for suffix in ["query", "close"] {
        let response = ctx
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!(
                        "/api/dashboard/store/admin/orders/missing/{suffix}"
                    ))
                    .header("cookie", format!("monoize_session={session_token}"))
                    .header("Origin", "https://attacker.example")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from("not-json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert_no_store(&response);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status, StatusCode::FORBIDDEN, "{suffix}: {body}");
        assert_eq!(body["error"]["code"], "store_origin_invalid");
    }

    let replica = monoize::app::build_app(
        ctx.state
            .clone()
            .with_node_role(monoize::node_config::NodeRole::Replica),
    );
    for suffix in ["query", "close"] {
        let response = replica
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!(
                        "/api/dashboard/store/admin/orders/missing/{suffix}"
                    ))
                    .header(AUTHORIZATION, &admin)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"attempt_id":"attempt"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert_no_store(&response);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{suffix}: {body}");
        assert_eq!(body["error"]["code"], "store_write_rejected");
    }

    let (status, _) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/orders/missing/complete",
        &admin,
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn create_admin_operation_fixture(
    ctx: &mut super::TestContext,
    query_provider: ApiPaymentQueryProvider,
    suffix: &str,
) -> (String, String, String) {
    configure_payment_fixture(ctx).await;
    configure_checkout_runtime(ctx, ApiCheckoutProvider::default()).await;
    ctx.state.payment_query_provider = Arc::new(query_provider);
    ctx.router = monoize::app::build_app(ctx.state.clone());
    let user = session(ctx, &format!("admin_operation_user_{suffix}")).await;
    let (_, order) = json_request(
        ctx,
        Method::POST,
        "/api/dashboard/store/orders",
        &user,
        Some(&format!("admin-operation-order-{suffix}")),
        Some(json!({
            "product_id":"api-payment-product",
            "payment_channel_id":"store-channel-stripe",
            "payment_currency":"CNY"
        })),
    )
    .await;
    let order_id = order["id"].as_str().unwrap().to_string();
    let (_, checkout) = json_request(
        ctx,
        Method::POST,
        &format!("/api/dashboard/store/orders/{order_id}/attempts"),
        &user,
        Some(&format!("admin-operation-attempt-{suffix}")),
        Some(json!({"expected_payment_method":"card"})),
    )
    .await;
    let attempt_id = checkout["attempt"]["id"].as_str().unwrap().to_string();
    (user, order_id, attempt_id)
}

#[tokio::test]
async fn admin_order_detail_is_no_store_and_confirmed_unpaid_close_is_visible() {
    let mut ctx = setup().await;
    let provider = ApiPaymentQueryProvider::returning(ProviderPaymentState::Unpaid);
    let (_, order_id, attempt_id) =
        create_admin_operation_fixture(&mut ctx, provider.clone(), "unpaid").await;
    let admin = admin_session(&ctx, "admin_operation_unpaid_admin").await;

    let response = ctx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/dashboard/store/admin/orders/{order_id}"))
                .header(AUTHORIZATION, &admin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("Cache-Control").unwrap(), "no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let detail: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(detail["order"]["id"], order_id);
    assert_eq!(detail["attempts"][0]["id"], attempt_id);
    assert!(detail["refunds"].as_array().unwrap().is_empty());
    let encoded = String::from_utf8(bytes.to_vec()).unwrap();
    for forbidden in [
        "ciphertext_base64",
        "nonce_base64",
        "raw_ciphertext_base64",
        "secret_key",
        "webhook_signing_secret",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }

    let (status, queried) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/admin/orders/{order_id}/query"),
        &admin,
        None,
        Some(json!({"attempt_id":attempt_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{queried}");
    assert_eq!(queried["provider_state"]["kind"], "unpaid");
    assert_eq!(queried["order"]["payment_state"], "unpaid");
    assert_eq!(queried["attempt"]["state"], "presented");
    assert_eq!(queried["closed"], false);

    let (status, closed) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/admin/orders/{order_id}/close"),
        &admin,
        None,
        Some(json!({"attempt_id":attempt_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{closed}");
    assert_eq!(closed["provider_state"]["kind"], "unpaid");
    assert_eq!(closed["order"]["payment_state"], "closed");
    assert_eq!(closed["attempt"]["state"], "expired");
    assert_eq!(closed["closed"], true);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);

    let (status, repeated) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/admin/orders/{order_id}/close"),
        &admin,
        None,
        Some(json!({"attempt_id":attempt_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{repeated}");
    assert_eq!(repeated["error"]["code"], "order_not_payable");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn admin_paid_query_projects_and_fulfills_once_with_deterministic_event() {
    let mut ctx = setup().await;
    let provider = ApiPaymentQueryProvider::returning(ProviderPaymentState::Paid {
        provider_transaction_id: "pi_admin_query_paid".to_string(),
    });
    let (_, order_id, attempt_id) =
        create_admin_operation_fixture(&mut ctx, provider.clone(), "paid").await;
    let admin = admin_session(&ctx, "admin_operation_paid_admin").await;
    let path = format!("/api/dashboard/store/admin/orders/{order_id}/query");

    let (status, first) = json_request(
        &ctx,
        Method::POST,
        &path,
        &admin,
        None,
        Some(json!({"attempt_id":attempt_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["provider_state"]["kind"], "paid");
    assert_eq!(first["projection"], "applied");
    assert_eq!(first["order"]["payment_state"], "paid");
    assert_eq!(first["order"]["fulfillment_state"], "fulfilled");

    let (status, second) = json_request(
        &ctx,
        Method::POST,
        &path,
        &admin,
        None,
        Some(json!({"attempt_id":attempt_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["projection"], "duplicate");
    let event_count: i64 = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE event_kind = 'payment_query_succeeded'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    assert_eq!(event_count, 1);
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
async fn admin_ambiguous_close_and_invalid_json_leave_order_unchanged() {
    let mut ctx = setup().await;
    let provider = ApiPaymentQueryProvider::returning(ProviderPaymentState::Ambiguous);
    let (_, order_id, attempt_id) =
        create_admin_operation_fixture(&mut ctx, provider, "ambiguous").await;
    let admin = admin_session(&ctx, "admin_operation_ambiguous_admin").await;
    let path = format!("/api/dashboard/store/admin/orders/{order_id}/close");

    let (status, invalid) = json_request(
        &ctx,
        Method::POST,
        &path,
        &admin,
        None,
        Some(json!({"attempt_id":attempt_id,"unexpected":true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
    assert_eq!(invalid["error"]["code"], "invalid_request");

    let (status, ambiguous) = json_request(
        &ctx,
        Method::POST,
        &path,
        &admin,
        None,
        Some(json!({"attempt_id":attempt_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{ambiguous}");
    assert_eq!(ambiguous["error"]["code"], "payment_provider_ambiguous");
    let (status, detail) = json_request(
        &ctx,
        Method::GET,
        &format!("/api/dashboard/store/admin/orders/{order_id}"),
        &admin,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["order"]["payment_state"], "unpaid");
    assert_eq!(detail["attempts"][0]["state"], "presented");
}

#[tokio::test]
async fn admin_order_operation_errors_are_no_store() {
    let ctx = setup().await;
    let admin = admin_session(&ctx, "admin_operation_error_header_admin").await;

    let response = raw_json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/orders/missing",
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_no_store(&response);

    let response = raw_json_request(
        &ctx,
        Method::GET,
        "/api/dashboard/store/admin/orders/missing",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_no_store(&response);

    let response = raw_json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/orders/missing/close",
        Some(&admin),
        Some("not-json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_no_store(&response);
    let mut configuration_ctx = setup().await;
    let (_, configuration_order_id, configuration_attempt_id) = create_admin_operation_fixture(
        &mut configuration_ctx,
        ApiPaymentQueryProvider::returning(ProviderPaymentState::Unpaid),
        "configuration-error-header",
    )
    .await;
    let configuration_admin =
        admin_session(&configuration_ctx, "configuration_error_header_admin").await;
    configuration_ctx.state.payment_keys = None;
    configuration_ctx.router = monoize::app::build_app(configuration_ctx.state.clone());
    let response = raw_json_request(
        &configuration_ctx,
        Method::POST,
        &format!("/api/dashboard/store/admin/orders/{configuration_order_id}/query"),
        Some(&configuration_admin),
        Some(&json!({ "attempt_id": configuration_attempt_id }).to_string()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_no_store(&response);

    for (suffix, provider, expected_status) in [
        (
            "provider-error-header",
            ApiPaymentQueryProvider::failing(AdapterError::Rejected),
            StatusCode::BAD_GATEWAY,
        ),
        (
            "ambiguous-error-header",
            ApiPaymentQueryProvider::returning(ProviderPaymentState::Ambiguous),
            StatusCode::CONFLICT,
        ),
    ] {
        let mut operation_ctx = setup().await;
        let (_, order_id, attempt_id) =
            create_admin_operation_fixture(&mut operation_ctx, provider, suffix).await;
        let operation_admin = admin_session(&operation_ctx, &format!("{suffix}-admin")).await;
        let response = raw_json_request(
            &operation_ctx,
            Method::POST,
            &format!("/api/dashboard/store/admin/orders/{order_id}/query"),
            Some(&operation_admin),
            Some(&json!({ "attempt_id": attempt_id }).to_string()),
        )
        .await;
        assert_eq!(response.status(), expected_status);
        assert_no_store(&response);
    }
}

#[tokio::test]
async fn admin_refund_create_detail_and_query_enforce_the_frozen_contract() {
    let mut ctx = setup().await;
    let payment_provider = ApiPaymentQueryProvider::returning(ProviderPaymentState::Paid {
        provider_transaction_id: "pi_admin_refund".to_string(),
    });
    let (_, order_id, attempt_id) =
        create_admin_operation_fixture(&mut ctx, payment_provider, "refund").await;
    let admin = admin_session(&ctx, "admin_refund_admin").await;
    let (status, paid) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/admin/orders/{order_id}/query"),
        &admin,
        None,
        Some(json!({"attempt_id": attempt_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{paid}");
    assert_eq!(paid["order"]["payment_state"], "paid");

    let refund_provider =
        ApiRefundProvider::new(ProviderRefundState::Pending, ProviderRefundState::Succeeded);
    ctx.state.refund_provider = Arc::new(refund_provider.clone());
    ctx.router = monoize::app::build_app(ctx.state.clone());
    let grant = issue_refund_reauth(&ctx, &admin).await;
    let create_path = format!("/api/dashboard/store/admin/orders/{order_id}/refunds");

    let response = refund_request(
        &ctx.router,
        Method::POST,
        &create_path,
        Some(&admin),
        Some(&grant),
        Some("admin-refund-create"),
        None,
        None,
        r#"{"unexpected":true}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_no_store(&response);
    assert_eq!(refund_provider.create_calls.load(Ordering::SeqCst), 0);

    let response = refund_request(
        &ctx.router,
        Method::POST,
        &create_path,
        Some(&admin),
        Some(&grant),
        None,
        None,
        None,
        "{}",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_no_store(&response);
    assert_eq!(refund_provider.create_calls.load(Ordering::SeqCst), 0);

    let response = refund_request(
        &ctx.router,
        Method::POST,
        &create_path,
        Some(&admin),
        None,
        Some("admin-refund-create"),
        None,
        None,
        "{}",
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_no_store(&response);
    assert_eq!(refund_provider.create_calls.load(Ordering::SeqCst), 0);

    let response = refund_request(
        &ctx.router,
        Method::POST,
        &create_path,
        Some(&admin),
        Some(&grant),
        Some("admin-refund-create"),
        None,
        None,
        "{}",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_no_store(&response);
    let (status, refund) = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{refund}");
    assert_eq!(refund["order_id"], order_id);
    assert_eq!(refund["state"], "pending");
    let refund_id = refund["id"].as_str().unwrap().to_string();
    assert_eq!(refund_provider.create_calls.load(Ordering::SeqCst), 1);

    let detail_path = format!("/api/dashboard/store/admin/orders/{order_id}/refunds/{refund_id}");
    let response = raw_json_request(&ctx, Method::GET, &detail_path, Some(&admin), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_no_store(&response);
    let (_, detail) = response_json(response).await;
    assert_eq!(detail["id"], refund_id);
    assert_eq!(detail["order_id"], order_id);

    let wrong_detail = raw_json_request(
        &ctx,
        Method::GET,
        &format!("/api/dashboard/store/admin/orders/other-order/refunds/{refund_id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(wrong_detail.status(), StatusCode::NOT_FOUND);
    assert_no_store(&wrong_detail);

    let wrong_query = refund_request(
        &ctx.router,
        Method::POST,
        &format!("/api/dashboard/store/admin/orders/other-order/refunds/{refund_id}/query"),
        Some(&admin),
        Some(&grant),
        None,
        None,
        None,
        "{}",
    )
    .await;
    assert_eq!(wrong_query.status(), StatusCode::NOT_FOUND);
    assert_no_store(&wrong_query);
    assert_eq!(refund_provider.query_calls.load(Ordering::SeqCst), 0);

    let query_path =
        format!("/api/dashboard/store/admin/orders/{order_id}/refunds/{refund_id}/query");
    let response = refund_request(
        &ctx.router,
        Method::POST,
        &query_path,
        Some(&admin),
        Some(&grant),
        None,
        None,
        None,
        "{}",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_no_store(&response);
    let (_, queried) = response_json(response).await;
    assert_eq!(queried["id"], refund_id);
    assert_eq!(queried["state"], "succeeded");
    assert_eq!(refund_provider.query_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn admin_refund_mutation_guard_errors_are_no_store() {
    let mut ctx = setup().await;
    let admin = admin_session(&ctx, "admin_refund_guard_admin").await;
    ctx.state.payment_public_origin = Some(url::Url::parse("https://lynshen.org").unwrap());
    ctx.router = monoize::app::build_app(ctx.state.clone());
    let session_token = admin.strip_prefix("Bearer ").unwrap();

    for path in [
        "/api/dashboard/store/admin/orders/missing/refunds",
        "/api/dashboard/store/admin/orders/missing/refunds/refund-id/query",
    ] {
        let response = refund_request(
            &ctx.router,
            Method::POST,
            path,
            None,
            Some("not-used"),
            Some("guard-key"),
            Some("https://attacker.example"),
            Some(&format!("monoize_session={session_token}")),
            "{}",
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        assert_no_store(&response);
    }

    let replica = monoize::app::build_app(
        ctx.state
            .clone()
            .with_node_role(monoize::node_config::NodeRole::Replica),
    );
    for path in [
        "/api/dashboard/store/admin/orders/missing/refunds",
        "/api/dashboard/store/admin/orders/missing/refunds/refund-id/query",
    ] {
        let response = refund_request(
            &replica,
            Method::POST,
            path,
            Some(&admin),
            Some("not-used"),
            Some("guard-key"),
            None,
            None,
            "{}",
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_no_store(&response);
    }
}

#[tokio::test]
async fn admin_provider_event_reprocess_enforces_auth_body_and_no_store_contract() {
    let ctx = setup().await;
    let admin = admin_session(&ctx, "admin_reprocess_contract").await;
    let grant = issue_reprocess_reauth(&ctx, &admin).await;
    let event_id = uuid::Uuid::new_v4().to_string();
    ctx.state
        .db_pool
        .write()
        .await
        .execute(ctx.state.db_pool.stmt(
            "INSERT INTO store_provider_events
                (id, credential_version_id, provider_event_id, event_kind,
                 body_digest, parsed_json, verification_result, projection_state,
                 state_revision, received_at, applied_at)
             VALUES ($1, 'api-reprocess-credential', 'evt-api-reprocess',
                     'payment_succeeded', $2, $3, 'verified', 'applied', 4,
                     '2026-08-28T00:00:00Z', '2026-08-28T00:00:00Z')",
            vec![
                event_id.clone().into(),
                "a".repeat(64).into(),
                json!({"event_id":"evt-api-reprocess"}).to_string().into(),
            ],
        ))
        .await
        .unwrap();
    let path = format!("/api/dashboard/store/admin/provider-events/{event_id}/reprocess");

    let response = refund_request(
        &ctx.router,
        Method::POST,
        &path,
        Some(&admin),
        Some(&grant),
        None,
        None,
        None,
        r#"{"unexpected":true}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_no_store(&response);
    let invalid_body_audits = ctx
        .state
        .db_pool
        .read()
        .query_one(ctx.state.db_pool.stmt(
            "SELECT COUNT(*) AS value FROM store_access_audits
             WHERE action = 'provider_event_reprocess'
               AND result = 'invalid_request'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(invalid_body_audits.try_get::<i64>("", "value").unwrap(), 1);

    let response = refund_request(
        &ctx.router,
        Method::POST,
        &path,
        Some(&admin),
        None,
        None,
        None,
        None,
        "{}",
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_no_store(&response);

    let response = refund_request(
        &ctx.router,
        Method::POST,
        &path,
        Some(&admin),
        Some(&grant),
        None,
        None,
        None,
        "{}",
    )
    .await;
    assert_no_store(&response);
    let (status, body) = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["event_id"], event_id);
    assert_eq!(body["projection"], "duplicate");
    assert_eq!(body["projection_state"], "applied");
    assert_eq!(body["state_revision"], 4);
    assert!(body["order_id"].is_null());
    assert!(body["attempt_id"].is_null());
}
