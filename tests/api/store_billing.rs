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
