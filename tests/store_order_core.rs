use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentAttemptFailureKind,
    PaymentOrderError, PaymentOrderStore,
};
use monoize::store_billing::payment::CheckoutAction;
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use std::sync::Arc;
use tokio::sync::Barrier;

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
                     'bm9uY2U=', 'Y2lwaGVydGV4dA==',
                     '1111111111111111111111111111111111111111111111111111111111111111',
                     'active',
                     '2026-08-27T00:00:00Z')",
            )
            .await
            .expect("insert credential version");
        write
            .execute_unprepared(
                "INSERT INTO store_payment_compliance
                    (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
                 VALUES
                    ('compliance-1', 'store-channel-stripe', '2026-08-28', 'admin-1',
                     '127.0.0.1', '2026-08-27T00:00:00Z');
                 INSERT INTO store_merchant_capabilities
                    (id, channel_id, capability, state, environment, merchant_account_digest,
                     provider_product, evidence_digest, verifier_admin_id, verified_at, expires_at)
                 VALUES
                    ('cap-payment-query', 'store-channel-stripe', 'payment_query', 'supported',
                     'sandbox', '1111111111111111111111111111111111111111111111111111111111111111', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin-1',
                     '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z'),
                    ('cap-refund', 'store-channel-stripe', 'refund', 'supported',
                     'sandbox', '1111111111111111111111111111111111111111111111111111111111111111', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin-1',
                     '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z'),
                    ('cap-refund-query', 'store-channel-stripe', 'refund_query', 'supported',
                     'sandbox', '1111111111111111111111111111111111111111111111111111111111111111', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin-1',
                     '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z'),
                    ('cap-settlement', 'store-channel-stripe', 'settlement_report', 'supported',
                     'sandbox', '1111111111111111111111111111111111111111111111111111111111111111', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin-1',
                     '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z');
                 INSERT INTO store_privacy_records
                    (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                     legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
                 VALUES ('order-privacy', 'v1', 'CN', '[]', '{}', 'contract', 'admin-1',
                         'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                         '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z', 1);
                 INSERT INTO store_channel_readiness_profiles
                    (channel_id, active_credential_digest, privacy_record_id,
                     callback_verification_passed, supported_currencies_json, amount_limits_json,
                     checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                     availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
                 VALUES ('store-channel-stripe',
                         '1111111111111111111111111111111111111111111111111111111111111111',
                         'order-privacy', 1,
                         '[\"CNY\",\"USD\"]',
                         '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                         '[\"redirect\"]',
                         'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                         'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                         'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                         'admin-1', '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z')",
            )
            .await
            .expect("insert payment governance fixtures");
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

async fn age_orders_past_creation_window(db: &DbPool, user_id: &str) {
    let created_at = (Utc::now() - chrono::Duration::minutes(2)).to_rfc3339();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_orders SET created_at = $1 WHERE user_id = $2",
            vec![created_at.into(), user_id.into()],
        ))
        .await
        .expect("age order fixtures");
}

#[test]
fn postgres_order_creation_locks_user_before_limit_counts() {
    let source = include_str!("../src/store_billing/order.rs").replace("\r\n", "\n");
    let function_start = source
        .find("pub async fn create_order(")
        .expect("create_order must exist");
    let function_end = source[function_start..]
        .find("pub async fn list_orders_for_user(")
        .map(|offset| function_start + offset)
        .expect("create_order must end before list_orders_for_user");
    let body = &source[function_start..function_end];
    let transaction = body
        .find("let tx = self.db.begin_write().await.map_err(storage)?")
        .expect("create_order must start a write transaction");
    let transactional_body = &body[transaction..];
    let lock = transactional_body
        .find("lock_order_creation_user(&self.db, &*tx, user_id).await?")
        .expect("create_order must lock its PostgreSQL user row");
    let idempotency_recheck = transactional_body
        .find("query_order_by_creation_key(&self.db, &*tx, user_id, &input.idempotency_key).await?")
        .expect("create_order must recheck idempotency inside the transaction");
    let recent_count = transactional_body
        .find("let recent_count =")
        .expect("recent order count must exist");
    let open_count = transactional_body
        .find("let open_count =")
        .expect("open order count must exist");

    assert!(lock < idempotency_recheck);
    assert!(idempotency_recheck < recent_count);
    assert!(idempotency_recheck < open_count);
    assert!(lock < recent_count);
    assert!(lock < open_count);
    assert!(source.contains(
        "const POSTGRES_ORDER_CREATION_USER_LOCK_SQL: &str = \"SELECT id FROM users WHERE id = $1 FOR UPDATE\";"
    ));
}

#[test]
fn order_and_attempt_lock_the_channel_before_governance_reads() {
    let source = include_str!("../src/store_billing/order.rs").replace("\r\n", "\n");
    let create_order_start = source
        .find("pub async fn create_order(")
        .expect("create_order must exist");
    let create_order_end = source[create_order_start..]
        .find("pub async fn list_orders_for_user(")
        .map(|offset| create_order_start + offset)
        .expect("create_order must have a bounded body");
    let create_order = &source[create_order_start..create_order_end];
    let user_lock = create_order
        .find("lock_order_creation_user(&self.db, &*tx, user_id).await?")
        .expect("create_order must lock the user");
    let channel_lock = create_order
        .find("lock_channel(&self.db, &*tx, &input.payment_channel_id)")
        .expect("create_order must lock the Channel");
    let evaluation = create_order
        .find("evaluate_channel_for_payment(")
        .expect("create_order must evaluate the Channel");
    assert!(user_lock < channel_lock);
    assert!(channel_lock < evaluation);

    let create_attempt_start = source
        .find("pub async fn create_attempt_with_outcome(")
        .expect("create_attempt_with_outcome must exist");
    let create_attempt_end = source[create_attempt_start..]
        .find("pub async fn present_attempt(")
        .map(|offset| create_attempt_start + offset)
        .expect("create_attempt_with_outcome must have a bounded body");
    let create_attempt = &source[create_attempt_start..create_attempt_end];
    let order_lock = create_attempt
        .find("query_order_by_id_for_update(")
        .expect("create_attempt must lock the order");
    let channel_lock = create_attempt
        .find("lock_channel(&self.db, &*tx, &order.payment_channel_id)")
        .expect("create_attempt must lock the Channel");
    let evaluation = create_attempt
        .find("evaluate_channel_for_payment(")
        .expect("create_attempt must re-evaluate governance");
    let credential_read = create_attempt
        .find("FROM store_channel_credentials")
        .expect("create_attempt must read the active credential");
    assert!(order_lock < channel_lock);
    assert!(channel_lock < evaluation);
    assert!(channel_lock < credential_read);

    let governance = include_str!("../src/store_billing/governance.rs").replace("\r\n", "\n");
    assert!(governance.contains("SELECT id FROM store_payment_channels WHERE id = $1 FOR UPDATE"));
    assert!(
        governance.contains("UPDATE store_payment_channels SET revision = revision WHERE id = $1")
    );
    let credentials = include_str!("../src/store_billing/credentials.rs");
    assert!(credentials.contains("lock_channel(&self.db, &*tx, channel_id)"));
}

#[tokio::test]
async fn concurrent_order_creation_enforces_per_minute_limit() {
    let (_db, store) = setup().await;
    let barrier = Arc::new(Barrier::new(7));
    let mut tasks = Vec::new();
    for index in 0..6 {
        let store = store.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .create_order(
                    "minute-limit-user",
                    order_input(&format!("minute-limit-{index}")),
                    &rate(),
                )
                .await
        }));
    }
    barrier.wait().await;

    let mut created = 0;
    let mut rate_limited = 0;
    for task in tasks {
        match task.await.expect("order creation task") {
            Ok(_) => created += 1,
            Err(PaymentOrderError::CreationRateLimited) => rate_limited += 1,
            Err(error) => panic!("unexpected order creation error: {error}"),
        }
    }

    assert_eq!(created, 5);
    assert_eq!(rate_limited, 1);
    assert_eq!(
        store
            .list_orders_for_user("minute-limit-user", 100)
            .await
            .unwrap()
            .len(),
        5
    );
}

#[tokio::test]
async fn concurrent_same_idempotency_key_replays_one_order() {
    let (_db, store) = setup().await;
    let barrier = Arc::new(Barrier::new(3));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let store = store.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .create_order("same-key-user", order_input("same-key-concurrent"), &rate())
                .await
        }));
    }
    barrier.wait().await;

    let first = tasks
        .remove(0)
        .await
        .expect("first order creation task")
        .expect("first order creation");
    let second = tasks
        .remove(0)
        .await
        .expect("second order creation task")
        .expect("second order creation");

    assert_eq!(first.id, second.id);
    assert_eq!(
        store
            .list_orders_for_user("same-key-user", 100)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn concurrent_order_creation_enforces_open_order_limit() {
    let (db, store) = setup().await;
    for index in 0..5 {
        store
            .create_order(
                "open-limit-user",
                order_input(&format!("open-fixture-{index}")),
                &rate(),
            )
            .await
            .unwrap();
    }
    age_orders_past_creation_window(&db, "open-limit-user").await;
    for index in 5..9 {
        store
            .create_order(
                "open-limit-user",
                order_input(&format!("open-fixture-{index}")),
                &rate(),
            )
            .await
            .unwrap();
    }
    age_orders_past_creation_window(&db, "open-limit-user").await;

    let barrier = Arc::new(Barrier::new(12));
    let mut tasks = Vec::new();
    for index in 0..11 {
        let store = store.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .create_order(
                    "open-limit-user",
                    order_input(&format!("open-contender-{index}")),
                    &rate(),
                )
                .await
        }));
    }
    barrier.wait().await;

    let mut created = 0;
    let mut open_limited = 0;
    for task in tasks {
        match task.await.expect("order creation task") {
            Ok(_) => created += 1,
            Err(PaymentOrderError::OpenOrderLimit) => open_limited += 1,
            Err(error) => panic!("unexpected order creation error: {error}"),
        }
    }

    assert_eq!(created, 1);
    assert_eq!(open_limited, 10);
    assert_eq!(
        store
            .list_orders_for_user("open-limit-user", 100)
            .await
            .unwrap()
            .len(),
        10
    );
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
async fn pending_sqlite_gate_rejects_plan_orders_but_not_balance_orders() {
    let (db, store) = setup().await;
    db.write()
        .await
        .execute_unprepared(
            "INSERT INTO store_products
                (id, kind, name, description, price_currency, price_minor,
                 duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
             VALUES ('plan-1', 'plan', 'Plan', '', 'CNY', '5900', 2592000, '[]', 0, 1,
                     '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z');
             INSERT INTO store_plan_quotas
                (id, product_id, window_kind, window_seconds, quota_fen_cny, sort_order)
             VALUES ('plan-1-day', 'plan-1', 'day', 86400, '2000', 0);",
        )
        .await
        .unwrap();
    let plan = CreatePaymentOrderInput {
        idempotency_key: "plan-order-gated".to_string(),
        product_id: "plan-1".to_string(),
        payment_channel_id: "store-channel-stripe".to_string(),
        payment_currency: Currency::CNY,
        custom_recharge_minor: None,
    };

    assert_eq!(
        store
            .create_order("user-1", plan, &rate())
            .await
            .unwrap_err(),
        PaymentOrderError::ProductUnavailable
    );
    assert!(
        store
            .create_order("user-1", order_input("balance-still-allowed"), &rate())
            .await
            .is_ok()
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
async fn order_replay_does_not_require_an_exchange_rate() {
    let (_db, store) = setup().await;
    let input = order_input("checkout-replay-without-rate");
    let created = store
        .create_order("user-1", input.clone(), &rate())
        .await
        .unwrap();

    assert_eq!(
        store.replay_order("user-1", &input).await.unwrap(),
        Some(created)
    );
    let mut changed = input;
    changed.payment_currency = Currency::USD;
    assert_eq!(
        store.replay_order("user-1", &changed).await.unwrap_err(),
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
    assert_eq!(attempt.payment_contract_version, 2);
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

#[tokio::test]
async fn attempt_creation_fails_closed_when_governance_expires_after_order_creation() {
    let (db, store) = setup().await;
    let order = store
        .create_order("user-1", order_input("governance-expiry-order"), &rate())
        .await
        .unwrap();
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_readiness_profiles
             SET expires_at = '2026-01-01T00:00:00Z'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();

    assert_eq!(
        store
            .create_attempt(
                "user-1",
                &order.id,
                CreatePaymentAttemptInput {
                    idempotency_key: "governance-expiry-attempt".to_string(),
                    expected_payment_method: Some("card".to_string()),
                },
            )
            .await
            .unwrap_err(),
        PaymentOrderError::ChannelUnavailable
    );
    let attempts = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM store_payment_attempts WHERE order_id = $1",
            vec![order.id.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap();
    assert_eq!(attempts, 0);
}

#[tokio::test]
async fn wechat_qr_only_readiness_rejects_h5_before_attempt_insert() {
    let (db, store) = setup().await;
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_payment_channels
             SET adapter_kind = 'wechat' WHERE id = 'store-channel-stripe';
             UPDATE store_channel_credentials
             SET adapter_kind = 'wechat' WHERE id = 'credential-1';
             UPDATE store_channel_readiness_profiles
             SET supported_currencies_json = '[\"CNY\"]',
                 amount_limits_json = '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                 checkout_action_kinds_json = '[\"qr\"]'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();
    let order = store
        .create_order("user-1", order_input("wechat-qr-order"), &rate())
        .await
        .unwrap();

    assert_eq!(
        store
            .create_attempt(
                "user-1",
                &order.id,
                CreatePaymentAttemptInput {
                    idempotency_key: "wechat-h5-attempt".to_string(),
                    expected_payment_method: Some("h5".to_string()),
                },
            )
            .await
            .unwrap_err(),
        PaymentOrderError::ChannelUnavailable
    );
    let native = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "wechat-native-attempt".to_string(),
                expected_payment_method: Some("native".to_string()),
            },
        )
        .await
        .unwrap();
    assert_eq!(native.expected_payment_method.as_deref(), Some("native"));
}

#[tokio::test]
async fn presented_checkout_action_is_persisted_and_replayed() {
    let (db, store) = setup().await;
    let order = store
        .create_order("user-1", order_input("checkout-present"), &rate())
        .await
        .unwrap();
    let attempt = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "attempt-present".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    let action = CheckoutAction::Redirect {
        url: "https://checkout.stripe.com/c/pay_test".to_string(),
        expires_at: "2026-08-27T01:00:00Z".to_string(),
    };

    let presented = store
        .present_attempt("user-1", &attempt.id, "cs_test_1", &action)
        .await
        .unwrap();
    assert_eq!(presented.state.as_str(), "presented");
    assert_eq!(presented.provider_object_id.as_deref(), Some("cs_test_1"));
    assert_eq!(presented.action.as_ref(), Some(&action));
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_readiness_profiles
             SET expires_at = '2026-01-01T00:00:00Z'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();

    let replay = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "attempt-present".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    assert_eq!(replay, presented);
    assert_eq!(
        store
            .create_attempt(
                "user-2",
                &order.id,
                CreatePaymentAttemptInput {
                    idempotency_key: "attempt-present".to_string(),
                    expected_payment_method: Some("card".to_string()),
                },
            )
            .await
            .unwrap_err(),
        PaymentOrderError::OrderNotFound
    );
}

#[tokio::test]
async fn failed_attempt_replays_persisted_failure_after_governance_expires() {
    let (db, store) = setup().await;
    let order = store
        .create_order("user-1", order_input("checkout-failed-replay"), &rate())
        .await
        .unwrap();
    let attempt = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "attempt-failed-replay".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    let failed = store
        .fail_attempt(
            "user-1",
            &attempt.id,
            PaymentAttemptFailureKind::ProviderRejected,
        )
        .await
        .unwrap();
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_readiness_profiles
             SET expires_at = '2026-01-01T00:00:00Z'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();

    let replay = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "attempt-failed-replay".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    assert_eq!(replay, failed);
    assert_eq!(
        replay.failure_kind,
        Some(PaymentAttemptFailureKind::ProviderRejected)
    );
}

#[tokio::test]
async fn created_attempt_replay_rejects_a_different_active_merchant() {
    let (db, store) = setup().await;
    let order = store
        .create_order("user-1", order_input("merchant-rotation-order"), &rate())
        .await
        .unwrap();
    let attempt = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "merchant-rotation-attempt".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    assert_eq!(attempt.state.as_str(), "created");
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_channel_credentials SET status = 'retired', retired_at = '2026-08-28T00:00:00Z'
             WHERE id = 'credential-1';
             INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ('credential-2', 'store-channel-stripe', 'stripe', 1, 'key-2',
                     'bm9uY2U=', 'Y2lwaGVydGV4dA==',
                     '2222222222222222222222222222222222222222222222222222222222222222',
                     'active', '2026-08-28T00:00:00Z');
             UPDATE store_merchant_capabilities
             SET merchant_account_digest = '2222222222222222222222222222222222222222222222222222222222222222'
             WHERE channel_id = 'store-channel-stripe';
             UPDATE store_channel_readiness_profiles
             SET active_credential_digest = '2222222222222222222222222222222222222222222222222222222222222222'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();

    assert_eq!(
        store
            .create_attempt(
                "user-1",
                &order.id,
                CreatePaymentAttemptInput {
                    idempotency_key: "merchant-rotation-attempt".to_string(),
                    expected_payment_method: Some("card".to_string()),
                },
            )
            .await
            .unwrap_err(),
        PaymentOrderError::ChannelUnavailable
    );
}

#[tokio::test]
async fn created_attempt_replay_uses_persisted_none_payment_method_default() {
    let (db, store) = setup().await;
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_payment_channels
             SET adapter_kind = 'wechat' WHERE id = 'store-channel-stripe';
             UPDATE store_channel_credentials
             SET adapter_kind = 'wechat' WHERE id = 'credential-1';
             UPDATE store_channel_readiness_profiles
             SET supported_currencies_json = '[\"CNY\"]',
                 amount_limits_json = '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                 checkout_action_kinds_json = '[\"qr\"]'
             WHERE channel_id = 'store-channel-stripe'",
        )
        .await
        .unwrap();
    let order = store
        .create_order("user-1", order_input("persisted-default-order"), &rate())
        .await
        .unwrap();
    let created = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "persisted-default-attempt".to_string(),
                expected_payment_method: None,
            },
        )
        .await
        .unwrap();

    let replay = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "persisted-default-attempt".to_string(),
                expected_payment_method: Some("unknown".to_string()),
            },
        )
        .await
        .unwrap();
    assert_eq!(replay, created);
    assert_eq!(replay.expected_payment_method, None);
}

#[tokio::test]
async fn checkout_actions_reject_insecure_redirect_and_form_urls() {
    let (_db, store) = setup().await;
    let order = store
        .create_order("user-1", order_input("checkout-https"), &rate())
        .await
        .unwrap();
    let attempt = store
        .create_attempt(
            "user-1",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "attempt-https".to_string(),
                expected_payment_method: None,
            },
        )
        .await
        .unwrap();

    for action in [
        CheckoutAction::Redirect {
            url: "http://checkout.example/pay".to_string(),
            expires_at: "2026-08-27T01:00:00Z".to_string(),
        },
        CheckoutAction::Form {
            action: "http://checkout.example/form".to_string(),
            fields: vec![],
            expires_at: "2026-08-27T01:00:00Z".to_string(),
        },
    ] {
        assert_eq!(
            store
                .present_attempt("user-1", &attempt.id, "provider-object", &action)
                .await
                .unwrap_err(),
            PaymentOrderError::InvalidInput
        );
    }
}
