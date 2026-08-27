use chrono::{TimeZone, Utc};
use futures_util::future::join_all;
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::callbacks::{
    ApplyProviderEventInput, CallbackApplyResult, PaymentCallbackStore,
};
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderStore,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

async fn setup() -> (DbPool, String, String) {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    {
        let write = db.write().await;
        Migrator::up(&*write, None).await.expect("run migrations");
        write
            .execute_unprepared(
                "INSERT INTO users
                    (id, username, password_hash, role, created_at, updated_at, enabled,
                     balance_nano_usd, balance_unlimited, group_id)
                 SELECT 'callback-user', 'callback-user', 'test', 'user',
                        '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z', 1, '0', 0, id
                 FROM monoize_groups WHERE is_default = 1 LIMIT 1",
            )
            .await
            .expect("insert user");
        write
            .execute_unprepared(
                "INSERT INTO store_products
                    (id, kind, name, description, price_currency, price_minor,
                     duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
                 VALUES
                    ('callback-product', 'balance', 'Recharge', '', 'CNY', '1000',
                     NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            )
            .await
            .expect("insert product");
        write
            .execute_unprepared(
                "INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
                 VALUES ('callback-product', '1000', '200')",
            )
            .await
            .expect("insert balance product");
        write
            .execute_unprepared(
                "UPDATE store_payment_channels SET enabled = 1
                 WHERE id = 'store-channel-stripe'",
            )
            .await
            .expect("enable Channel");
        write
            .execute_unprepared(
                "INSERT INTO store_channel_credentials
                    (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                     ciphertext_base64, account_identity_digest, status, created_at)
                 VALUES
                    ('callback-credential', 'store-channel-stripe', 'stripe', 1, 'key-1',
                     'bm9uY2U=', 'Y2lwaGVydGV4dA==', 'acct-digest', 'active',
                     '2026-08-27T00:00:00Z')",
            )
            .await
            .expect("insert credential");
    }
    let orders = PaymentOrderStore::new(db.clone());
    let order = orders
        .create_order(
            "callback-user",
            CreatePaymentOrderInput {
                idempotency_key: "callback-order".to_string(),
                product_id: "callback-product".to_string(),
                payment_channel_id: "store-channel-stripe".to_string(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &ExchangeRateSnapshot {
                base: "USD".to_string(),
                quote: "CNY".to_string(),
                cny_per_usd: "6.0000".to_string(),
                source_updated_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 0, 0).unwrap(),
                refreshed_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
            },
        )
        .await
        .unwrap();
    let attempt = orders
        .create_attempt(
            "callback-user",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "callback-attempt".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    (db, order.id, attempt.id)
}

fn success_event(order_id: &str, attempt_id: &str) -> ApplyProviderEventInput {
    ApplyProviderEventInput {
        credential_version_id: "callback-credential".to_string(),
        provider_event_id: "evt-payment-1".to_string(),
        event_kind: "payment_succeeded".to_string(),
        order_id: order_id.to_string(),
        attempt_id: attempt_id.to_string(),
        provider_transaction_id: "pi-payment-1".to_string(),
        amount_minor: "1000".to_string(),
        currency: Currency::CNY,
        body_digest: "a".repeat(64),
        parsed_json: serde_json::json!({"type":"payment_succeeded"}),
        received_at: Utc::now(),
    }
}

#[tokio::test]
async fn concurrent_duplicate_callbacks_fulfill_once() {
    let (db, order_id, attempt_id) = setup().await;
    let callback_store = PaymentCallbackStore::new(db.clone());
    let futures = (0..20).map(|_| {
        let store = callback_store.clone();
        let event = success_event(&order_id, &attempt_id);
        async move { store.apply_verified_payment(event).await.unwrap() }
    });
    let results = join_all(futures).await;

    assert_eq!(
        results
            .iter()
            .filter(|result| **result == CallbackApplyResult::Applied)
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == CallbackApplyResult::Duplicate)
            .count(),
        19
    );

    let read = db.read();
    let event_count: i64 = read
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM store_provider_events",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    let ledger_count: i64 = read
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger
             WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{order_id}").into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    let order = read
        .query_one(db.stmt(
            "SELECT payment_state, fulfillment_state FROM store_orders WHERE id = $1",
            vec![order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let balance: String = read
        .query_one(db.stmt(
            "SELECT balance_nano_usd FROM users WHERE id = 'callback-user'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "balance_nano_usd")
        .unwrap();

    assert_eq!(event_count, 1);
    assert_eq!(ledger_count, 1);
    assert_eq!(
        order.try_get::<String>("", "payment_state").unwrap(),
        "paid"
    );
    assert_eq!(
        order.try_get::<String>("", "fulfillment_state").unwrap(),
        "fulfilled"
    );
    assert_eq!(balance, "2000000000");
}

#[tokio::test]
async fn callback_mismatch_is_persisted_for_manual_review_without_fulfillment() {
    let (db, order_id, attempt_id) = setup().await;
    let store = PaymentCallbackStore::new(db.clone());
    let mut event = success_event(&order_id, &attempt_id);
    event.provider_event_id = "evt-mismatch".to_string();
    event.amount_minor = "999".to_string();

    assert_eq!(
        store.apply_verified_payment(event).await.unwrap(),
        CallbackApplyResult::ManualReview
    );
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT projection_state FROM store_provider_events
             WHERE provider_event_id = 'evt-mismatch'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "projection_state").unwrap(),
        "manual_review"
    );
}
