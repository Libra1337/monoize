use async_trait::async_trait;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use monoize::store_billing::exchange_rate::{
    ExchangeRateFetcher, ExchangeRateService, ExchangeRateSnapshot, ExchangeRateStore,
};
use monoize::users::UserRole;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
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

    let (status, attempt) = json_request(
        &ctx,
        Method::POST,
        &format!("/api/dashboard/store/orders/{order_id}/attempts"),
        &user,
        Some("attempt-api-1"),
        Some(json!({"expected_payment_method":"card"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{attempt}");
    assert_eq!(attempt["state"], "created");
    assert_eq!(attempt["order_id"], order["id"]);
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
