use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderStore,
};
use monoize::store_billing::reconciliation::{ReconciliationError, StoreReconciler};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

async fn paid_pending_order() -> (DbPool, String) {
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
                 SELECT 'reconcile-user', 'reconcile-user', 'test', 'user',
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
                    ('reconcile-product', 'balance', 'Recharge', '', 'CNY', '1000',
                     NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            )
            .await
            .expect("insert product");
        write
            .execute_unprepared(
                "INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
                 VALUES ('reconcile-product', '1000', '200')",
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
                    ('reconcile-credential', 'store-channel-stripe', 'stripe', 1, 'key-1',
                     'bm9uY2U=', 'Y2lwaGVydGV4dA==',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'active', '2026-08-27T00:00:00Z')",
            )
            .await
            .expect("insert credential");
    }
    let orders = PaymentOrderStore::new(db.clone());
    let order = orders
        .create_order(
            "reconcile-user",
            CreatePaymentOrderInput {
                idempotency_key: "reconcile-order".to_string(),
                product_id: "reconcile-product".to_string(),
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
            "reconcile-user",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "reconcile-attempt".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    let write = db.write().await;
    write
        .execute(db.stmt(
            "UPDATE store_payment_attempts
             SET state = 'paid', provider_object_id = 'cs-reconcile',
                 provider_transaction_id = 'pi-reconcile', paid_at = $2, updated_at = $2
             WHERE id = $1",
            vec![attempt.id.into(), "2026-08-27T00:00:00Z".into()],
        ))
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "UPDATE store_orders
             SET payment_state = 'paid', paid_at = $2, updated_at = $2,
                 state_revision = state_revision + 1
             WHERE id = $1",
            vec![order.id.clone().into(), "2026-08-27T00:00:00Z".into()],
        ))
        .await
        .unwrap();
    drop(write);
    (db, order.id)
}

#[tokio::test]
async fn reconciler_fulfills_paid_pending_balance_once() {
    let (db, order_id) = paid_pending_order().await;
    let reconciler = StoreReconciler::new(db.clone());
    let now = Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap();

    let first = reconciler.run_once("owner-a", now).await.unwrap();
    assert_eq!(first.scanned, 1);
    assert_eq!(first.fulfilled, 1);
    assert_eq!(first.failed, 0);
    let second = reconciler.run_once("owner-a", now).await.unwrap();
    assert_eq!(second.scanned, 0);

    let read = db.read();
    let order = read
        .query_one(db.stmt(
            "SELECT fulfillment_state FROM store_orders WHERE id = $1",
            vec![order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let balance = read
        .query_one(db.stmt(
            "SELECT balance_nano_usd FROM users WHERE id = 'reconcile-user'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    let ledger_count = read
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{order_id}").into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        order.try_get::<String>("", "fulfillment_state").unwrap(),
        "fulfilled"
    );
    assert_eq!(
        balance.try_get::<String>("", "balance_nano_usd").unwrap(),
        "2000000000"
    );
    assert_eq!(ledger_count.try_get::<i64>("", "value").unwrap(), 1);
}

#[tokio::test]
async fn reconciler_lease_fences_another_owner_until_expiry() {
    let (db, _) = paid_pending_order().await;
    let reconciler = StoreReconciler::new(db);
    let now = Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap();

    reconciler.run_once("owner-a", now).await.unwrap();
    assert_eq!(
        reconciler.run_once("owner-b", now).await.unwrap_err(),
        ReconciliationError::LeaseUnavailable
    );
    reconciler
        .run_once("owner-b", now + chrono::Duration::seconds(91))
        .await
        .unwrap();
}

#[tokio::test]
async fn reconciler_schedules_bounded_backoff_after_fulfillment_failure() {
    let (db, order_id) = paid_pending_order().await;
    db.write()
        .await
        .execute_unprepared(
            "UPDATE users SET balance_nano_usd = '170141183460469231731687303715884105727'
             WHERE id = 'reconcile-user'",
        )
        .await
        .unwrap();
    let reconciler = StoreReconciler::new(db.clone());
    let first_at = Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap();

    let first = reconciler.run_once("owner-a", first_at).await.unwrap();
    assert_eq!(first.failed, 1);
    let retry = db
        .read()
        .query_one(db.stmt(
            "SELECT attempt_count, next_attempt_at, last_error_category
             FROM store_fulfillment_retries WHERE order_id = $1",
            vec![order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry.try_get::<i64>("", "attempt_count").unwrap(), 1);
    assert_eq!(
        retry.try_get::<String>("", "next_attempt_at").unwrap(),
        "2026-08-27T00:03:00.000000Z"
    );
    assert_eq!(
        retry.try_get::<String>("", "last_error_category").unwrap(),
        "fulfillment_failed"
    );
    assert_eq!(
        reconciler
            .run_once("owner-a", first_at + chrono::Duration::seconds(119))
            .await
            .unwrap()
            .scanned,
        0
    );
    assert_eq!(
        reconciler
            .run_once("owner-a", first_at + chrono::Duration::seconds(120))
            .await
            .unwrap()
            .failed,
        1
    );
    let second_retry = db
        .read()
        .query_one(db.stmt(
            "SELECT attempt_count, next_attempt_at FROM store_fulfillment_retries
             WHERE order_id = $1",
            vec![order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second_retry.try_get::<i64>("", "attempt_count").unwrap(), 2);
    assert_eq!(
        second_retry
            .try_get::<String>("", "next_attempt_at")
            .unwrap(),
        "2026-08-27T00:13:00.000000Z"
    );
    assert_eq!(
        reconciler
            .run_once("owner-a", first_at + chrono::Duration::minutes(12))
            .await
            .unwrap()
            .failed,
        1
    );
    let third_retry = db
        .read()
        .query_one(db.stmt(
            "SELECT attempt_count, next_attempt_at FROM store_fulfillment_retries
             WHERE order_id = $1",
            vec![order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(third_retry.try_get::<i64>("", "attempt_count").unwrap(), 3);
    assert_eq!(
        third_retry
            .try_get::<String>("", "next_attempt_at")
            .unwrap(),
        "2026-08-27T01:13:00.000000Z"
    );
}
