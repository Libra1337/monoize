use async_trait::async_trait;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use monoize::store_billing::adapters::stripe::{StripeCheckoutResult, StripeCredential};
use monoize::store_billing::checkout::CheckoutProvider;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::{
    ExchangeRateFetcher, ExchangeRateService, ExchangeRateSnapshot, ExchangeRateStore,
};
use monoize::store_billing::payment::{AdapterError, CheckoutAction, CheckoutRequest};
use monoize::users::UserRole;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
