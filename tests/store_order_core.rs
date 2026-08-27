use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderError, PaymentOrderStore,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

async fn setup() -> (DbPool, PaymentOrderStore) {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    {
        let write = db.write().await;
        Migrator::up(&*write, None).await.expect("run migrations");
        write
            .execute_unprepared(
                "INSERT INTO store_products
                    (id, kind, name, description, price_currency, price_minor,
                     duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
                 VALUES
                    ('balance-1', 'balance', 'Recharge', '', 'CNY', '1000',
                     NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            )
            .await
            .expect("insert product");
        write
            .execute_unprepared(
                "INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
                 VALUES ('balance-1', '1000', '200')",
            )
            .await
            .expect("insert balance details");
        write
            .execute_unprepared(
                "UPDATE store_payment_channels SET enabled = 1
                 WHERE id = 'store-channel-stripe'",
            )
            .await
            .expect("enable Stripe Channel");
        write
            .execute_unprepared(
                "INSERT INTO store_channel_credentials
                    (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                     ciphertext_base64, account_identity_digest, status, created_at)
                 VALUES
                    ('credential-1', 'store-channel-stripe', 'stripe', 1, 'key-1',
                     'bm9uY2U=', 'Y2lwaGVydGV4dA==', 'acct-digest', 'active',
                     '2026-08-27T00:00:00Z')",
            )
            .await
            .expect("insert credential version");
    }
    let store = PaymentOrderStore::new(db.clone());
    (db, store)
}

fn rate() -> ExchangeRateSnapshot {
    ExchangeRateSnapshot {
        base: "USD".to_string(),
        quote: "CNY".to_string(),
        cny_per_usd: "6.7370".to_string(),
        source_updated_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 0, 0).unwrap(),
        refreshed_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
    }
}

fn order_input(key: &str) -> CreatePaymentOrderInput {
    CreatePaymentOrderInput {
        idempotency_key: key.to_string(),
        product_id: "balance-1".to_string(),
        payment_channel_id: "store-channel-stripe".to_string(),
        payment_currency: Currency::CNY,
        custom_recharge_minor: None,
    }
}

#[tokio::test]
async fn order_creation_is_user_scoped_and_idempotent() {
    let (_db, store) = setup().await;
    let first = store
        .create_order("user-1", order_input("checkout-1"), &rate())
        .await
        .unwrap();
    let replay = store
        .create_order("user-1", order_input("checkout-1"), &rate())
        .await
        .unwrap();

    assert_eq!(replay.id, first.id);
    assert_eq!(first.payment_state.as_str(), "unpaid");
    assert_eq!(first.fulfillment_state.as_str(), "pending");
    assert_eq!(first.payment_minor, "1000");
    assert_eq!(first.rate_numerator, "6737");
    assert_eq!(first.rate_denominator, "1000");
    assert_eq!((first.expires_at - first.created_at).num_minutes(), 30);
    assert_eq!(
        store
            .list_orders_for_user("user-1", 100)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .list_orders_for_user("user-2", 100)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn idempotency_key_reuse_with_different_input_is_rejected() {
    let (_db, store) = setup().await;
    store
        .create_order("user-1", order_input("checkout-1"), &rate())
        .await
        .unwrap();
    let mut changed = order_input("checkout-1");
    changed.payment_currency = Currency::USD;

    assert_eq!(
        store
            .create_order("user-1", changed, &rate())
            .await
            .unwrap_err(),
        PaymentOrderError::IdempotencyConflict
    );
}

#[tokio::test]
async fn one_order_has_at_most_one_active_payment_attempt() {
    let (_db, store) = setup().await;
    let order = store
        .create_order("user-1", order_input("checkout-1"), &rate())
        .await
        .unwrap();
    let attempt = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "attempt-1".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();

    assert_eq!(attempt.state.as_str(), "created");
    assert_eq!(attempt.order_id, order.id);
    assert_eq!(attempt.credential_version_id, "credential-1");
    assert_eq!(
        store
            .create_attempt(
                "user-1",
                &order.id,
                CreatePaymentAttemptInput {
                    idempotency_key: "attempt-2".to_string(),
                    expected_payment_method: Some("card".to_string()),
                },
            )
            .await
            .unwrap_err(),
        PaymentOrderError::ActiveAttemptExists
    );
    assert_eq!(
        store
            .create_attempt(
                "user-2",
                &order.id,
                CreatePaymentAttemptInput {
                    idempotency_key: "attempt-other".to_string(),
                    expected_payment_method: None,
                },
            )
            .await
            .unwrap_err(),
        PaymentOrderError::OrderNotFound
    );
}
