use chrono::{TimeZone, Utc};
use futures_util::future::join_all;
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::StoreBillingStore;
use monoize::store_billing::callbacks::{
    ApplyProviderEventInput, CallbackApplyResult, PaymentCallbackStore,
    RecordUnboundProviderEventInput,
};
use monoize::store_billing::crypto::{EncryptedSecret, PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::models::{CreateProductInput, PlanQuotaInput, ProductKind, WindowKind};
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderStore,
};
use monoize::store_billing::quota_gate::{GateSlot, QuotaGateStore, QuotaManifest};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use sha2::Digest as _;

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
        write
            .execute_unprepared(
                "INSERT INTO store_payment_compliance
                    (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
                 VALUES ('callback-compliance', 'store-channel-stripe', '2026-08-28',
                         'callback-admin', '127.0.0.1', '2026-08-28T00:00:00Z')",
            )
            .await
            .expect("insert compliance");
        for capability in [
            "payment_query",
            "refund",
            "refund_query",
            "settlement_report",
        ] {
            write
                .execute(db.stmt(
                    "INSERT INTO store_merchant_capabilities
                        (id, channel_id, capability, state, environment,
                         merchant_account_digest, provider_product, evidence_digest,
                         verifier_admin_id, verified_at, expires_at)
                     VALUES ($1, 'store-channel-stripe', $2, 'supported', 'sandbox',
                             'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                             'checkout',
                             'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                             'callback-admin', '2026-08-28T00:00:00Z',
                             '2099-01-01T00:00:00Z')",
                    vec![format!("callback-{capability}").into(), capability.into()],
                ))
                .await
                .expect("insert capability");
        }
        write
            .execute_unprepared(
                "INSERT INTO store_privacy_records
                    (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                     legal_basis, reviewer_id, evidence_digest, approved_at,
                     next_review_at, accepted)
                 VALUES ('callback-privacy', 'v1', 'CN', '[]', '{}', 'contract',
                         'callback-admin',
                         'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                         '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z', 1)",
            )
            .await
            .expect("insert privacy record");
        write
            .execute_unprepared(
                "INSERT INTO store_channel_readiness_profiles
                    (channel_id, active_credential_digest, privacy_record_id,
                     callback_verification_passed, supported_currencies_json,
                     amount_limits_json, checkout_action_kinds_json,
                     license_evidence_digest, runtime_evidence_digest,
                     availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
                 VALUES ('store-channel-stripe',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                         'callback-privacy', 1, '[\"CNY\",\"USD\"]',
                         '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"50\",\"max_minor\":\"100000000\"}}',
                         '[\"redirect\"]',
                         'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                         'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                         'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
                         'callback-admin', '2026-08-28T00:00:00Z',
                         '2099-01-01T00:00:00Z')",
            )
            .await
            .expect("insert readiness profile");
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
async fn pending_sqlite_gate_blocks_plan_fulfillment() {
    let (db, _, _, _) = setup().await;
    let gate = QuotaGateStore::new(db.clone());
    let environment = gate.live_environment().await.unwrap();
    gate.import_manifest(
        GateSlot::Current,
        QuotaManifest::passed(
            environment.clone(),
            "callback-test",
            "callback-drill",
            Utc::now(),
            "callback-admin",
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let product = StoreBillingStore::new(db.clone())
        .create_product(CreateProductInput {
            kind: ProductKind::Plan,
            name: "Gated plan".to_string(),
            description: String::new(),
            price_currency: Currency::CNY,
            price_minor: "5900".to_string(),
            duration_seconds: Some(2_592_000),
            group_ids: vec![],
            sort_order: 0,
            enabled: true,
            balance: None,
            quotas: vec![PlanQuotaInput {
                window_kind: WindowKind::Day,
                window_seconds: 86_400,
                quota_fen_cny: "2000".to_string(),
                sort_order: 0,
            }],
        })
        .await
        .unwrap();
    let order = PaymentOrderStore::new(db.clone())
        .create_order(
            "callback-user",
            CreatePaymentOrderInput {
                idempotency_key: "gated-plan-order".to_string(),
                product_id: product.id,
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
    gate.record_failure(
        GateSlot::Current,
        environment,
        "callback-gate-failure",
        Utc::now(),
    )
    .await
    .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_orders
             SET payment_state = 'paid', paid_at = $2
             WHERE id = $1",
            vec![order.id.clone().into(), "2026-08-28T00:00:00Z".into()],
        ))
        .await
        .unwrap();

    let error = PaymentCallbackStore::new(db.clone())
        .fulfill_paid_order(&order.id)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("quota_gate_unavailable"));
    let state = db
        .read()
        .query_one(db.stmt(
            "SELECT fulfillment_state,
                    (SELECT COUNT(*) FROM store_plan_entitlement_generations) AS entitlements
             FROM store_orders WHERE id = $1",
            vec![order.id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state.try_get::<String>("", "fulfillment_state").unwrap(),
        "pending"
    );
    assert_eq!(state.try_get::<i64>("", "entitlements").unwrap(), 0);
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

#[test]
fn reprocess_rechecks_identity_conflict_after_the_order_lock() {
    let source = include_str!("../src/store_billing/callbacks.rs");
    let reprocess = source
        .split("pub async fn reprocess_verified_event")
        .nth(1)
        .unwrap()
        .split("pub async fn")
        .next()
        .unwrap();
    let order_lock = reprocess
        .find("SELECT id FROM store_orders WHERE id = $1{lock}")
        .unwrap();
    let conflict_checks = reprocess
        .match_indices("has_open_reprocess_identity_conflict")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    assert_eq!(conflict_checks.len(), 2);
    assert!(conflict_checks[0] < order_lock);
    assert!(conflict_checks[1] > order_lock);
}

#[tokio::test]
async fn verified_manual_review_event_reprocesses_once_and_audits_each_request() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    let key_ring =
        PaymentKeyRing::new(PaymentKey::new("callback-key", [7_u8; 32]).unwrap(), vec![]).unwrap();
    let event_row_id = uuid::Uuid::new_v4().to_string();
    let raw = b"stored verified callback";
    let raw_body = key_ring
        .encrypt(
            &format!("store_provider_events:{event_row_id}:raw_body"),
            raw,
        )
        .unwrap();
    let body_digest = sha2::Sha256::digest(raw)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    PaymentCallbackStore::new(db.clone())
        .record_unbound_verified_event(RecordUnboundProviderEventInput {
            event_row_id: event_row_id.clone(),
            credential_version_id: "callback-credential".to_string(),
            provider_event_id: "evt-reprocess-once".to_string(),
            event_kind: "payment_succeeded".to_string(),
            body_digest,
            parsed_json: serde_json::json!({
                "event_id": "evt-reprocess-once",
                "event_kind": "payment_succeeded",
                "checkout_session_id": "cs-callback",
                "payment_intent_id": "pi-reprocess-once",
                "attempt_id": &attempt_id,
                "order_number": &order_number,
                "amount_minor": "1000",
                "currency": "CNY",
                "account_identity": "a".repeat(64),
            }),
            raw_body,
            source_ip: None,
            user_agent: None,
            received_at: Utc::now(),
        })
        .await
        .unwrap();

    let store = PaymentCallbackStore::new(db.clone());
    let applied = store
        .reprocess_verified_event(&event_row_id, Some(&key_ring), "admin-reprocess")
        .await
        .unwrap();
    assert_eq!(applied.projection, "applied");
    assert_eq!(applied.projection_state, "applied");
    assert_eq!(applied.state_revision, 1);
    assert_eq!(applied.order_id.as_deref(), Some(order_id.as_str()));
    assert_eq!(applied.attempt_id.as_deref(), Some(attempt_id.as_str()));

    let duplicate = store
        .reprocess_verified_event(&event_row_id, Some(&key_ring), "admin-reprocess")
        .await
        .unwrap();
    assert_eq!(duplicate.projection, "duplicate");
    assert_eq!(duplicate.state_revision, 1);

    let counts = db
        .read()
        .query_one(db.stmt(
            "SELECT
                (SELECT COUNT(*) FROM store_order_event_applications
                 WHERE provider_event_row_id = $1) AS applications,
                (SELECT COUNT(*) FROM billing_ledger
                 WHERE idempotency_key = $2) AS ledger_entries,
                (SELECT COUNT(*) FROM store_access_audits
                 WHERE action = 'provider_event_reprocess' AND actor_id = $3) AS audits",
            vec![
                event_row_id.into(),
                format!("store:fulfillment:{order_id}").into(),
                "admin-reprocess".into(),
            ],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counts.try_get::<i64>("", "applications").unwrap(), 1);
    assert_eq!(counts.try_get::<i64>("", "ledger_entries").unwrap(), 1);
    assert_eq!(counts.try_get::<i64>("", "audits").unwrap(), 2);
}

#[tokio::test]
async fn reprocess_rejects_a_tampered_raw_digest_without_payment_projection() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    let key_ring =
        PaymentKeyRing::new(PaymentKey::new("callback-key", [9_u8; 32]).unwrap(), vec![]).unwrap();
    let event_row_id = uuid::Uuid::new_v4().to_string();
    let raw = b"verified callback with tampered digest";
    let raw_body = key_ring
        .encrypt(
            &format!("store_provider_events:{event_row_id}:raw_body"),
            raw,
        )
        .unwrap();
    PaymentCallbackStore::new(db.clone())
        .record_unbound_verified_event(RecordUnboundProviderEventInput {
            event_row_id: event_row_id.clone(),
            credential_version_id: "callback-credential".to_string(),
            provider_event_id: "evt-reprocess-tampered".to_string(),
            event_kind: "payment_succeeded".to_string(),
            body_digest: "f".repeat(64),
            parsed_json: serde_json::json!({
                "event_id": "evt-reprocess-tampered",
                "event_kind": "payment_succeeded",
                "checkout_session_id": "cs-callback",
                "payment_intent_id": "pi-reprocess-tampered",
                "attempt_id": &attempt_id,
                "order_number": &order_number,
                "amount_minor": "1000",
                "currency": "CNY",
                "account_identity": "a".repeat(64),
            }),
            raw_body,
            source_ip: None,
            user_agent: None,
            received_at: Utc::now(),
        })
        .await
        .unwrap();

    let error = PaymentCallbackStore::new(db.clone())
        .reprocess_verified_event(&event_row_id, Some(&key_ring), "admin-reprocess")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        monoize::store_billing::callbacks::ReprocessProviderEventError::IdentityConflict
    );
    let state = db
        .read()
        .query_one(db.stmt(
            "SELECT payment_state,
                    (SELECT projection_state FROM store_provider_events WHERE id = $2)
                        AS projection_state,
                    (SELECT COUNT(*) FROM store_order_event_applications
                     WHERE provider_event_row_id = $2) AS applications,
                    (SELECT COUNT(*) FROM store_access_audits
                     WHERE action = 'provider_event_reprocess' AND actor_id = $3) AS audits
             FROM store_orders WHERE id = $1",
            vec![
                order_id.into(),
                event_row_id.into(),
                "admin-reprocess".into(),
            ],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state.try_get::<String>("", "payment_state").unwrap(),
        "unpaid"
    );
    assert_eq!(
        state.try_get::<String>("", "projection_state").unwrap(),
        "manual_review"
    );
    assert_eq!(state.try_get::<i64>("", "applications").unwrap(), 0);
    assert_eq!(state.try_get::<i64>("", "audits").unwrap(), 1);
}

#[tokio::test]
async fn reprocess_rejects_an_applied_event_that_was_not_verified() {
    let (db, _, _, _) = setup().await;
    let event_row_id = uuid::Uuid::new_v4().to_string();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_provider_events
                (id, credential_version_id, provider_event_id, event_kind,
                 body_digest, parsed_json, verification_result, projection_state,
                 state_revision, received_at, applied_at)
             VALUES ($1, 'callback-credential', 'evt-applied-unverified',
                     'payment_succeeded', $2, '{}', 'rejected', 'applied', 2,
                     '2026-08-28T00:00:00Z', '2026-08-28T00:00:00Z')",
            vec![event_row_id.clone().into(), "a".repeat(64).into()],
        ))
        .await
        .unwrap();

    let error = PaymentCallbackStore::new(db.clone())
        .reprocess_verified_event(&event_row_id, None, "admin-reprocess")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        monoize::store_billing::callbacks::ReprocessProviderEventError::NotReprocessable
    );
    let audits = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM store_access_audits
             WHERE action = 'provider_event_reprocess' AND actor_id = 'admin-reprocess'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audits.try_get::<i64>("", "value").unwrap(), 1);
}

#[tokio::test]
async fn invalid_and_missing_reprocess_events_are_audited_with_null_prior_state() {
    let (db, _, _, _) = setup().await;
    let store = PaymentCallbackStore::new(db.clone());
    assert_eq!(
        store
            .reprocess_verified_event("not-a-uuid", None, "admin-reprocess")
            .await
            .unwrap_err(),
        monoize::store_billing::callbacks::ReprocessProviderEventError::InvalidInput
    );
    let missing = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        store
            .reprocess_verified_event(&missing, None, "admin-reprocess")
            .await
            .unwrap_err(),
        monoize::store_billing::callbacks::ReprocessProviderEventError::NotFound
    );
    let audits = db
        .read()
        .query_all(db.stmt(
            "SELECT scope_json FROM store_access_audits
             WHERE action = 'provider_event_reprocess' AND actor_id = 'admin-reprocess'
             ORDER BY created_at, id",
            vec![],
        ))
        .await
        .unwrap();
    assert_eq!(audits.len(), 2);
    for audit in audits {
        let scope: serde_json::Value =
            serde_json::from_str(&audit.try_get::<String>("", "scope_json").unwrap()).unwrap();
        assert!(scope["prior_projection_state"].is_null());
        assert!(scope["prior_state_revision"].is_null());
    }
}

#[tokio::test]
async fn reprocess_requires_a_provider_query_for_refund_pending_payment() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    let key_ring = PaymentKeyRing::new(
        PaymentKey::new("callback-key", [10_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let event_row_id = uuid::Uuid::new_v4().to_string();
    let raw = b"verified payment during refund";
    PaymentCallbackStore::new(db.clone())
        .record_unbound_verified_event(RecordUnboundProviderEventInput {
            event_row_id: event_row_id.clone(),
            credential_version_id: "callback-credential".to_string(),
            provider_event_id: "evt-refund-pending-reprocess".to_string(),
            event_kind: "payment_succeeded".to_string(),
            body_digest: sha2::Sha256::digest(raw)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            parsed_json: serde_json::json!({
                "event_id": "evt-refund-pending-reprocess",
                "event_kind": "payment_succeeded",
                "checkout_session_id": "cs-callback",
                "payment_intent_id": "pi-refund-pending",
                "attempt_id": &attempt_id,
                "order_number": &order_number,
                "amount_minor": "1000",
                "currency": "CNY",
                "account_identity": "a".repeat(64),
            }),
            raw_body: key_ring
                .encrypt(
                    &format!("store_provider_events:{event_row_id}:raw_body"),
                    raw,
                )
                .unwrap(),
            source_ip: None,
            user_agent: None,
            received_at: Utc::now(),
        })
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_orders SET payment_state = 'paid' WHERE id = $1",
            vec![order_id.clone().into()],
        ))
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_orders SET payment_state = 'refund_pending' WHERE id = $1",
            vec![order_id.into()],
        ))
        .await
        .unwrap();

    assert_eq!(
        PaymentCallbackStore::new(db)
            .reprocess_verified_event(&event_row_id, Some(&key_ring), "admin-reprocess")
            .await
            .unwrap_err(),
        monoize::store_billing::callbacks::ReprocessProviderEventError::ProviderQueryRequired
    );
}

#[tokio::test]
async fn wechat_reprocess_preserves_distinct_attempt_and_verification_credentials() {
    let (db, order_id, order_number, attempt_id) = setup().await;
    for statement in [
        "UPDATE store_payment_channels SET adapter_kind = 'wechat'
         WHERE id = 'store-channel-stripe'",
        "UPDATE store_channel_credentials SET adapter_kind = 'wechat'
         WHERE id = 'callback-credential'",
        "INSERT INTO store_channel_credentials
            (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
             ciphertext_base64, account_identity_digest, status, created_at)
         VALUES ('callback-wechat-verifier', 'store-channel-stripe', 'wechat', 1,
                 'key-1', 'bm9uY2U=', 'Y2lwaGVydGV4dA==',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'retired', '2026-08-27T00:00:01Z')",
    ] {
        db.write()
            .await
            .execute_unprepared(statement)
            .await
            .unwrap();
    }
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_payment_attempts
             SET adapter_kind = 'wechat', provider_object_id = NULL
             WHERE id = $1",
            vec![attempt_id.clone().into()],
        ))
        .await
        .unwrap();
    let key_ring = PaymentKeyRing::new(
        PaymentKey::new("callback-key", [11_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let event_row_id = uuid::Uuid::new_v4().to_string();
    let raw = b"verified wechat callback";
    PaymentCallbackStore::new(db.clone())
        .record_unbound_verified_event(RecordUnboundProviderEventInput {
            event_row_id: event_row_id.clone(),
            credential_version_id: "callback-credential".to_string(),
            provider_event_id: "evt-wechat-distinct-credentials".to_string(),
            event_kind: "payment_succeeded".to_string(),
            body_digest: sha2::Sha256::digest(raw)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            parsed_json: serde_json::json!({
                "event_id": "evt-wechat-distinct-credentials",
                "event_kind": "payment_succeeded",
                "transaction_id": "wechat-transaction-distinct",
                "order_number": &order_number,
                "amount_minor": "1000",
                "currency": "CNY",
                "account_identity": "a".repeat(64),
                "verification_credential_version_id": "callback-wechat-verifier",
            }),
            raw_body: key_ring
                .encrypt(
                    &format!("store_provider_events:{event_row_id}:raw_body"),
                    raw,
                )
                .unwrap(),
            source_ip: None,
            user_agent: None,
            received_at: Utc::now(),
        })
        .await
        .unwrap();

    let result = PaymentCallbackStore::new(db.clone())
        .reprocess_verified_event(&event_row_id, Some(&key_ring), "admin-reprocess")
        .await
        .unwrap();
    assert_eq!(result.projection, "applied");
    assert_eq!(result.order_id.as_deref(), Some(order_id.as_str()));
    assert_eq!(result.attempt_id.as_deref(), Some(attempt_id.as_str()));
}
