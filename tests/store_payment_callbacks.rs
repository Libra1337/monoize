use chrono::{TimeZone, Utc};
use futures_util::future::join_all;
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::callbacks::{
    ApplyProviderEventInput, CallbackApplyResult, PaymentCallbackStore,
    RecordUnboundProviderEventInput,
};
use monoize::store_billing::crypto::EncryptedSecret;
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderStore,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

async fn setup() -> (DbPool, String, String, String) {
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
                      'bm9uY2U=', 'Y2lwaGVydGV4dA==',
                      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                      'active',
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
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_payment_attempts SET provider_object_id = $2 WHERE id = $1",
            vec![attempt.id.clone().into(), "cs-callback".into()],
        ))
        .await
        .unwrap();
    (db, order.id, order.order_number, attempt.id)
}

fn success_event(order_id: &str, order_number: &str, attempt_id: &str) -> ApplyProviderEventInput {
    ApplyProviderEventInput {
        event_row_id: uuid::Uuid::new_v4().to_string(),
        credential_version_id: "callback-credential".to_string(),
        verification_credential_version_id: "callback-credential".to_string(),
        provider_event_id: "evt-payment-1".to_string(),
        event_kind: "payment_succeeded".to_string(),
        order_id: order_id.to_string(),
        attempt_id: attempt_id.to_string(),
        provider_transaction_id: "pi-payment-1".to_string(),
        provider_object_id: "cs-callback".to_string(),
        order_number: order_number.to_string(),
        merchant_account_identity: "a".repeat(64),
        amount_minor: "1000".to_string(),
        currency: Currency::CNY,
        body_digest: "a".repeat(64),
        parsed_json: serde_json::json!({"type":"payment_succeeded"}),
        raw_body: Some(EncryptedSecret {
            version: 1,
            key_id: "callback-key".to_string(),
            nonce_base64: "bm9uY2U=".to_string(),
            ciphertext_base64: "Y2lwaGVydGV4dA==".to_string(),
        }),
        source_ip: Some("203.0.113.1".to_string()),
        user_agent: Some("Stripe/1.0".to_string()),
        received_at: Utc::now(),
    }
}

async fn add_null_alipay_candidate(db: &DbPool, attempt_id: &str, candidate_id: &str) {
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_payment_attempts
                (id, order_id, channel_id, adapter_kind, credential_version_id,
                 merchant_account_identity, expected_payment_method,
                 payment_contract_version, state, idempotency_key,
                 created_at, updated_at)
             SELECT $2, order_id, channel_id, 'alipay', credential_version_id,
                    merchant_account_identity, expected_payment_method,
                    payment_contract_version, 'created', $3,
                    '2026-08-27T00:00:02Z', '2026-08-27T00:00:02Z'
             FROM store_payment_attempts WHERE id = $1",
            vec![
                attempt_id.into(),
                candidate_id.into(),
                format!("{candidate_id}-key").into(),
            ],
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn projection_rechecks_candidates_for_a_nonnull_expired_alipay_attempt() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_payment_attempts
             SET adapter_kind = 'alipay', state = 'expired'
             WHERE id = $1",
            vec![attempt_id.clone().into()],
        ))
        .await
        .unwrap();
    add_null_alipay_candidate(&db, &attempt_id, "callback-new-null-attempt").await;
    let mut event = success_event(&order_id, &order_number, &attempt_id);
    event.provider_event_id = "evt-nonnull-racing-candidate".to_string();

    assert_eq!(
        PaymentCallbackStore::new(db.clone())
            .apply_verified_payment(event)
            .await
            .unwrap(),
        CallbackApplyResult::ManualReview
    );
    let selected_state = db
        .read()
        .query_one(db.stmt(
            "SELECT state FROM store_payment_attempts WHERE id = $1",
            vec![attempt_id.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "state")
        .unwrap();
    assert_eq!(selected_state, "expired");
}

#[tokio::test]
async fn ambiguous_rotated_wechat_projection_is_idempotent_by_verification_credential() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_payment_attempts
             SET adapter_kind = 'wechat', state = 'expired',
                 merchant_account_identity = $2
             WHERE id = $1",
            vec![attempt_id.clone().into(), "b".repeat(64).into()],
        ))
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_payment_attempts
                (id, order_id, channel_id, adapter_kind, credential_version_id,
                 merchant_account_identity, expected_payment_method,
                 payment_contract_version, state, idempotency_key,
                 created_at, updated_at)
             SELECT 'callback-wechat-null-attempt', order_id, channel_id, 'wechat',
                    credential_version_id, merchant_account_identity,
                    expected_payment_method, payment_contract_version, 'created',
                    'callback-wechat-null-attempt-key',
                    '2026-08-27T00:00:02Z', '2026-08-27T00:00:02Z'
             FROM store_payment_attempts WHERE id = $1",
            vec![attempt_id.clone().into()],
        ))
        .await
        .unwrap();
    let mut event = success_event(&order_id, &order_number, &attempt_id);
    event.provider_event_id = "evt-wechat-rotated-race".to_string();
    event.merchant_account_identity = "b".repeat(64);
    event.verification_credential_version_id = "callback-verification-rotated".to_string();
    let store = PaymentCallbackStore::new(db.clone());
    for _ in 0..2 {
        assert_eq!(
            store.apply_verified_payment(event.clone()).await.unwrap(),
            CallbackApplyResult::ManualReview
        );
    }
    let event_counts = db
        .read()
        .query_one(db.stmt(
            "SELECT
                SUM(CASE WHEN credential_version_id = $1 THEN 1 ELSE 0 END) AS verification_count,
                SUM(CASE WHEN credential_version_id = $2 THEN 1 ELSE 0 END) AS attempt_count
             FROM store_provider_events WHERE provider_event_id = $3",
            vec![
                "callback-verification-rotated".into(),
                "callback-credential".into(),
                "evt-wechat-rotated-race".into(),
            ],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event_counts
            .try_get::<i64>("", "verification_count")
            .unwrap(),
        1
    );
    assert_eq!(event_counts.try_get::<i64>("", "attempt_count").unwrap(), 0);
    let application_count = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM store_order_event_applications",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap();
    assert_eq!(application_count, 0);
}

#[tokio::test]
async fn applied_duplicate_with_changed_evidence_opens_one_identity_conflict_case() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    let store = PaymentCallbackStore::new(db.clone());
    let original = success_event(&order_id, &order_number, &attempt_id);
    assert_eq!(
        store
            .apply_verified_payment(original.clone())
            .await
            .unwrap(),
        CallbackApplyResult::Applied
    );
    let mut changed = original.clone();
    changed.event_row_id = uuid::Uuid::new_v4().to_string();
    changed.body_digest = "b".repeat(64);
    changed.parsed_json = serde_json::json!({"type":"payment_succeeded","changed":true});
    for _ in 0..2 {
        assert_eq!(
            store.apply_verified_payment(changed.clone()).await.unwrap(),
            CallbackApplyResult::ManualReview
        );
    }
    let stored = db
        .read()
        .query_one(db.stmt(
            "SELECT body_digest, parsed_json FROM store_provider_events
             WHERE credential_version_id = $1 AND provider_event_id = $2",
            vec![
                original.credential_version_id.into(),
                original.provider_event_id.into(),
            ],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.try_get::<String>("", "body_digest").unwrap(),
        "a".repeat(64)
    );
    assert_eq!(
        stored.try_get::<String>("", "parsed_json").unwrap(),
        original.parsed_json.to_string()
    );
    let case_count = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM store_reconciliation_cases
             WHERE kind = 'provider_event_identity_conflict'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap();
    assert_eq!(case_count, 1);
    let ledger_count = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger
             WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{order_id}").into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap();
    assert_eq!(ledger_count, 1);
}

#[tokio::test]
async fn unbound_duplicate_with_changed_evidence_opens_one_identity_conflict_case() {
    let (db, _, _, _) = setup().await;
    let store = PaymentCallbackStore::new(db.clone());
    let original = RecordUnboundProviderEventInput {
        event_row_id: uuid::Uuid::new_v4().to_string(),
        credential_version_id: "callback-verification-credential".to_string(),
        provider_event_id: "evt-unbound-evidence-conflict".to_string(),
        event_kind: "payment_succeeded".to_string(),
        body_digest: "c".repeat(64),
        parsed_json: serde_json::json!({"event_id":"evt-unbound-evidence-conflict"}),
        raw_body: EncryptedSecret {
            version: 1,
            key_id: "callback-key".to_string(),
            nonce_base64: "bm9uY2U=".to_string(),
            ciphertext_base64: "Y2lwaGVydGV4dA==".to_string(),
        },
        source_ip: None,
        user_agent: None,
        received_at: Utc::now(),
    };
    assert_eq!(
        store
            .record_unbound_verified_event(original.clone())
            .await
            .unwrap(),
        CallbackApplyResult::ManualReview
    );
    let mut changed = original.clone();
    changed.event_row_id = uuid::Uuid::new_v4().to_string();
    changed.body_digest = "d".repeat(64);
    for _ in 0..2 {
        assert_eq!(
            store
                .record_unbound_verified_event(changed.clone())
                .await
                .unwrap(),
            CallbackApplyResult::ManualReview
        );
    }
    let stored_digest = db
        .read()
        .query_one(db.stmt(
            "SELECT body_digest FROM store_provider_events
             WHERE credential_version_id = $1 AND provider_event_id = $2",
            vec![
                original.credential_version_id.into(),
                original.provider_event_id.into(),
            ],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "body_digest")
        .unwrap();
    assert_eq!(stored_digest, "c".repeat(64));
    let case_count = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM store_reconciliation_cases
             WHERE kind = 'provider_event_identity_conflict'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap();
    assert_eq!(case_count, 1);
}

#[tokio::test]
async fn projection_rejects_a_second_null_provider_candidate_created_after_lookup() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_payment_attempts
             SET adapter_kind = 'alipay', state = 'created', provider_object_id = NULL
             WHERE id = $1",
            vec![attempt_id.clone().into()],
        ))
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_payment_attempts
                (id, order_id, channel_id, adapter_kind, credential_version_id,
                 merchant_account_identity, expected_payment_method,
                 payment_contract_version, state, failure_kind, idempotency_key,
                 created_at, updated_at)
             SELECT 'callback-racing-attempt', order_id, channel_id, adapter_kind,
                    credential_version_id, merchant_account_identity,
                    expected_payment_method, payment_contract_version, 'failed',
                    'provider_rejected', 'callback-racing-attempt-key',
                    '2026-08-27T00:00:01Z', '2026-08-27T00:00:01Z'
             FROM store_payment_attempts WHERE id = $1",
            vec![attempt_id.clone().into()],
        ))
        .await
        .unwrap();
    let mut event = success_event(&order_id, &order_number, &attempt_id);
    event.provider_event_id = "evt-racing-candidate".to_string();
    event.provider_object_id = order_number;

    assert_eq!(
        PaymentCallbackStore::new(db.clone())
            .apply_verified_payment(event)
            .await
            .unwrap(),
        CallbackApplyResult::ManualReview
    );
    let event = db
        .read()
        .query_one(db.stmt(
            "SELECT projection_state FROM store_provider_events
             WHERE provider_event_id = 'evt-racing-candidate'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event.try_get::<String>("", "projection_state").unwrap(),
        "manual_review"
    );
    let application_count = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM store_order_event_applications",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(application_count.try_get::<i64>("", "value").unwrap(), 0);
}

#[tokio::test]
async fn concurrent_duplicate_callbacks_fulfill_once() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    let callback_store = PaymentCallbackStore::new(db.clone());
    let futures = (0..20).map(|_| {
        let store = callback_store.clone();
        let event = success_event(&order_id, &order_number, &attempt_id);
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
async fn verified_late_payment_clears_a_prior_attempt_failure() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_payment_attempts
             SET state = 'failed', failure_kind = 'provider_rejected'
             WHERE id = $1",
            vec![attempt_id.clone().into()],
        ))
        .await
        .unwrap();

    PaymentCallbackStore::new(db.clone())
        .apply_verified_payment(success_event(&order_id, &order_number, &attempt_id))
        .await
        .unwrap();

    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT state, failure_kind FROM store_payment_attempts WHERE id = $1",
            vec![attempt_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "paid");
    assert_eq!(
        row.try_get::<Option<String>>("", "failure_kind").unwrap(),
        None
    );
}

#[tokio::test]
async fn callback_mismatch_is_persisted_for_manual_review_without_fulfillment() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    let store = PaymentCallbackStore::new(db.clone());
    let mut event = success_event(&order_id, &order_number, &attempt_id);
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

#[tokio::test]
async fn verified_payment_during_hold_is_recorded_without_fulfillment() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_orders SET payment_hold = 1 WHERE id = $1",
            vec![order_id.clone().into()],
        ))
        .await
        .unwrap();

    assert_eq!(
        PaymentCallbackStore::new(db.clone())
            .apply_verified_payment(success_event(&order_id, &order_number, &attempt_id))
            .await
            .unwrap(),
        CallbackApplyResult::Applied
    );
    let order = db
        .read()
        .query_one(db.stmt(
            "SELECT payment_state, fulfillment_state FROM store_orders WHERE id = $1",
            vec![order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let ledger_count = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{order_id}").into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        order.try_get::<String>("", "payment_state").unwrap(),
        "paid"
    );
    assert_eq!(
        order.try_get::<String>("", "fulfillment_state").unwrap(),
        "pending"
    );
    assert_eq!(ledger_count.try_get::<i64>("", "value").unwrap(), 0);
}

#[tokio::test]
async fn public_callback_projection_rejects_a_synthetic_query_event() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    let mut event = success_event(&order_id, &order_number, &attempt_id);
    event.event_kind = "payment_query_succeeded".to_string();
    event.provider_event_id = "payment-query:forged".to_string();
    event.raw_body = None;

    assert_eq!(
        PaymentCallbackStore::new(db)
            .apply_verified_payment(event)
            .await
            .unwrap_err(),
        monoize::store_billing::callbacks::CallbackStoreError::InvalidInput
    );
}
