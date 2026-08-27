use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::adapters::alipay::AlipayCredential;
use monoize::store_billing::adapters::stripe::StripeCredential;
use monoize::store_billing::adapters::wechat::{WechatCredential, WechatPlatformVerifier};
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::operations::{PaymentQueryOperations, PaymentQueryProvider};
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderStore,
};
use monoize::store_billing::payment::{
    AdapterError, CheckoutAction, PaymentQuery, ProviderPaymentState,
};
use monoize::store_billing::reconciliation::{ReconciliationError, StoreReconciler};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use sha2::{Digest, Sha256};

#[derive(Clone)]
struct FixedPaymentQueryProvider {
    result: Result<ProviderPaymentState, AdapterError>,
    calls: Arc<Mutex<Vec<PaymentQuery>>>,
}

impl FixedPaymentQueryProvider {
    fn returning(state: ProviderPaymentState) -> Self {
        Self {
            result: Ok(state),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl PaymentQueryProvider for FixedPaymentQueryProvider {
    async fn query_stripe_payment(
        &self,
        _credential: &StripeCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.calls.lock().unwrap().push(query.clone());
        self.result.clone()
    }

    async fn query_alipay_payment(
        &self,
        _credential: &AlipayCredential,
        _query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        Err(AdapterError::Unsupported)
    }

    async fn query_wechat_payment(
        &self,
        _credential: &WechatCredential,
        _verifiers: &[WechatPlatformVerifier],
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.calls.lock().unwrap().push(query.clone());
        self.result.clone()
    }
}

struct PresentedFixture {
    db: DbPool,
    key_ring: Arc<PaymentKeyRing>,
    order_id: String,
    attempt_id: String,
}

async fn expired_presented_order(suffix: &str) -> PresentedFixture {
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
                 SELECT 'query-user', 'query-user', 'test', 'user',
                        '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z', 1, '0', 0, id
                 FROM monoize_groups WHERE is_default = 1 LIMIT 1",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_products
                    (id, kind, name, description, price_currency, price_minor,
                     duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
                 VALUES
                    ('query-product', 'balance', 'Recharge', '', 'CNY', '1000',
                     NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
                 VALUES ('query-product', '1000', '200')",
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
    }

    let credential_id = format!("query-credential-{suffix}");
    let account_id = "acct_reconciliation";
    let account_digest = Sha256::digest(account_id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let key_ring = Arc::new(
        PaymentKeyRing::new(
            PaymentKey::new(format!("query-key-{suffix}"), [41_u8; 32]).unwrap(),
            vec![],
        )
        .unwrap(),
    );
    let encrypted = key_ring
        .encrypt(
            &format!("store_channel_credentials:{credential_id}:secret"),
            br#"{
                "secret_key":"sk_test_reconciliation",
                "publishable_key":"pk_test_reconciliation",
                "webhook_signing_secret":"whsec_reconciliation",
                "api_version":"2026-08-01",
                "account_id":"acct_reconciliation",
                "live_mode":false
            }"#,
        )
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ($1, 'store-channel-stripe', 'stripe', $2, $3, $4, $5, $6,
                     'active', '2026-08-27T00:00:00Z')",
            vec![
                credential_id.into(),
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                account_digest.into(),
            ],
        ))
        .await
        .unwrap();

    let orders = PaymentOrderStore::new(db.clone());
    let order = orders
        .create_order(
            "query-user",
            CreatePaymentOrderInput {
                idempotency_key: format!("query-order-{suffix}"),
                product_id: "query-product".to_string(),
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
            "query-user",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: format!("query-attempt-{suffix}"),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    orders
        .present_attempt(
            "query-user",
            &attempt.id,
            &format!("cs_query_{suffix}"),
            &CheckoutAction::Redirect {
                url: "https://checkout.stripe.com/test".to_string(),
                expires_at: "2026-08-27T00:00:30Z".to_string(),
            },
        )
        .await
        .unwrap();
    PresentedFixture {
        db,
        key_ring,
        order_id: order.id,
        attempt_id: attempt.id,
    }
}

async fn make_attempt_recoverable_wechat(fixture: &PresentedFixture, suffix: &str, state: &str) {
    let credential_id = format!("query-credential-{suffix}");
    let account_digest = Sha256::digest(b"1900000109")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let encrypted = fixture
        .key_ring
        .encrypt(
            &format!("store_channel_credentials:{credential_id}:secret"),
            br#"{
                "merchant_id":"1900000109",
                "app_id":"wx1234567890",
                "api_v3_key":"0123456789abcdef0123456789abcdef",
                "merchant_certificate_serial":"MERCHANT-CERTIFICATE-RECOVERY",
                "merchant_private_key_pem":"private",
                "platform_certificate_serial":"PLATFORM-CERTIFICATE-RECOVERY",
                "platform_public_key_pem":"public"
            }"#,
        )
        .unwrap();
    let failure_kind = (state == "failed").then_some("provider_rejected");
    let write = fixture.db.write().await;
    write
        .execute_unprepared(
            "UPDATE store_payment_channels SET adapter_kind = 'wechat'
             WHERE id = 'store-channel-stripe'",
        )
        .await
        .unwrap();
    write
        .execute(fixture.db.stmt(
            "UPDATE store_channel_credentials
             SET adapter_kind = 'wechat', format_version = $2, key_id = $3,
                 nonce_base64 = $4, ciphertext_base64 = $5,
                 account_identity_digest = $6
             WHERE id = $1",
            vec![
                credential_id.into(),
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
        .execute(fixture.db.stmt(
            "UPDATE store_payment_attempts
             SET adapter_kind = 'wechat', merchant_account_identity = $2,
                 state = $3, failure_kind = $4, provider_object_id = NULL,
                 action_kind = NULL, action_json = NULL, provider_expires_at = NULL,
                 presented_at = NULL, updated_at = '2026-08-27T00:00:00Z'
             WHERE id = $1",
            vec![
                fixture.attempt_id.clone().into(),
                account_digest.into(),
                state.into(),
                failure_kind.into(),
            ],
        ))
        .await
        .unwrap();
}

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

#[tokio::test]
async fn reconciler_projects_a_verified_paid_query_once() {
    let fixture = expired_presented_order("paid").await;
    let provider = FixedPaymentQueryProvider::returning(ProviderPaymentState::Paid {
        provider_transaction_id: "pi_query_paid".to_string(),
    });
    let operations = PaymentQueryOperations::new(
        fixture.db.clone(),
        fixture.key_ring,
        Arc::new(provider.clone()),
    );
    let reconciler = StoreReconciler::new(fixture.db.clone()).with_payment_queries(operations);
    let now = Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap();

    let first = reconciler.run_once("query-owner", now).await.unwrap();
    assert_eq!(first.payment_queries, 1);
    assert_eq!(first.payments_applied, 1);
    assert_eq!(first.attempts_expired, 0);
    let second = reconciler.run_once("query-owner", now).await.unwrap();
    assert_eq!(second.payment_queries, 0);

    let order = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT payment_state, fulfillment_state FROM store_orders WHERE id = $1",
            vec![fixture.order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let attempt = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT state, provider_transaction_id FROM store_payment_attempts WHERE id = $1",
            vec![fixture.attempt_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let event_count = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE event_kind = 'payment_query_succeeded'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    let ledger_count = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{}", fixture.order_id).into()],
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
        "fulfilled"
    );
    assert_eq!(attempt.try_get::<String>("", "state").unwrap(), "paid");
    assert_eq!(
        attempt
            .try_get::<String>("", "provider_transaction_id")
            .unwrap(),
        "pi_query_paid"
    );
    assert_eq!(event_count.try_get::<i64>("", "value").unwrap(), 1);
    assert_eq!(ledger_count.try_get::<i64>("", "value").unwrap(), 1);
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn reconciler_closes_an_expired_attempt_only_after_a_definite_query() {
    for (suffix, state) in [
        ("not-found", ProviderPaymentState::NotFound),
        ("unpaid", ProviderPaymentState::Unpaid),
        ("closed", ProviderPaymentState::Closed),
    ] {
        let fixture = expired_presented_order(suffix).await;
        let provider = FixedPaymentQueryProvider::returning(state);
        let operations =
            PaymentQueryOperations::new(fixture.db.clone(), fixture.key_ring, Arc::new(provider));
        let reconciler = StoreReconciler::new(fixture.db.clone()).with_payment_queries(operations);
        let outcome = reconciler
            .run_once(
                &format!("query-owner-{suffix}"),
                Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(outcome.payment_queries, 1);
        assert_eq!(outcome.attempts_expired, 1);

        let states = fixture
            .db
            .read()
            .query_one(fixture.db.stmt(
                "SELECT a.state AS attempt_state, o.payment_state
                 FROM store_payment_attempts a
                 JOIN store_orders o ON o.id = a.order_id
                 WHERE a.id = $1",
                vec![fixture.attempt_id.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            states.try_get::<String>("", "attempt_state").unwrap(),
            "expired"
        );
        assert_eq!(
            states.try_get::<String>("", "payment_state").unwrap(),
            "closed"
        );
    }
}

#[tokio::test]
async fn reconciler_keeps_an_expired_attempt_open_when_query_is_ambiguous() {
    let fixture = expired_presented_order("ambiguous").await;
    let provider = FixedPaymentQueryProvider::returning(ProviderPaymentState::Ambiguous);
    let operations =
        PaymentQueryOperations::new(fixture.db.clone(), fixture.key_ring, Arc::new(provider));
    let reconciler = StoreReconciler::new(fixture.db.clone()).with_payment_queries(operations);

    let outcome = reconciler
        .run_once(
            "query-owner-ambiguous",
            Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(outcome.payment_queries, 1);
    assert_eq!(outcome.query_failures, 1);
    assert_eq!(outcome.attempts_expired, 0);
    let states = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT a.state AS attempt_state, o.payment_state
             FROM store_payment_attempts a
             JOIN store_orders o ON o.id = a.order_id
             WHERE a.id = $1",
            vec![fixture.attempt_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        states.try_get::<String>("", "attempt_state").unwrap(),
        "presented"
    );
    assert_eq!(
        states.try_get::<String>("", "payment_state").unwrap(),
        "unpaid"
    );
    let case = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT kind, state, evidence_json FROM store_reconciliation_cases
             WHERE id = $1",
            vec![format!("payment-query:{}", fixture.attempt_id).into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(case.try_get::<String>("", "kind").unwrap(), "payment_query");
    assert_eq!(case.try_get::<String>("", "state").unwrap(), "open");
    let evidence: serde_json::Value =
        serde_json::from_str(&case.try_get::<String>("", "evidence_json").unwrap()).unwrap();
    assert_eq!(evidence["category"], "provider_ambiguous");
    assert_eq!(
        reconciler
            .run_once(
                "query-owner-ambiguous",
                Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 59).unwrap(),
            )
            .await
            .unwrap()
            .payment_queries,
        0
    );
}

#[tokio::test]
async fn reconciler_records_a_held_payment_without_fulfillment() {
    let fixture = expired_presented_order("held-paid").await;
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "UPDATE store_orders SET payment_hold = 1 WHERE id = $1",
            vec![fixture.order_id.clone().into()],
        ))
        .await
        .unwrap();
    let provider = FixedPaymentQueryProvider::returning(ProviderPaymentState::Paid {
        provider_transaction_id: "pi_query_held".to_string(),
    });
    let operations =
        PaymentQueryOperations::new(fixture.db.clone(), fixture.key_ring, Arc::new(provider));
    let reconciler = StoreReconciler::new(fixture.db.clone()).with_payment_queries(operations);

    let outcome = reconciler
        .run_once(
            "query-owner-held",
            Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(outcome.payments_applied, 1);
    assert_eq!(outcome.fulfilled, 0);
    let order = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT payment_state, fulfillment_state FROM store_orders WHERE id = $1",
            vec![fixture.order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    let ledger_count = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{}", fixture.order_id).into()],
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
async fn reconciler_releases_a_rejected_wechat_attempt_after_verified_not_found() {
    let fixture = expired_presented_order("wechat-rejected").await;
    make_attempt_recoverable_wechat(&fixture, "wechat-rejected", "failed").await;
    let provider = FixedPaymentQueryProvider::returning(ProviderPaymentState::NotFound);
    let operations = PaymentQueryOperations::new(
        fixture.db.clone(),
        fixture.key_ring.clone(),
        Arc::new(provider),
    );
    let reconciler = StoreReconciler::new(fixture.db.clone()).with_payment_queries(operations);

    let outcome = reconciler
        .run_once(
            "query-owner-wechat-rejected",
            Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(outcome.payment_queries, 1);
    assert_eq!(outcome.attempts_expired, 1);
    let states = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT a.state AS attempt_state, a.failure_kind, o.payment_state
             FROM store_payment_attempts a
             JOIN store_orders o ON o.id = a.order_id WHERE a.id = $1",
            vec![fixture.attempt_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        states.try_get::<String>("", "attempt_state").unwrap(),
        "expired"
    );
    assert_eq!(
        states
            .try_get::<Option<String>>("", "failure_kind")
            .unwrap(),
        None
    );
    assert_eq!(
        states.try_get::<String>("", "payment_state").unwrap(),
        "unpaid"
    );
}

#[tokio::test]
async fn reconciler_projects_a_created_wechat_attempt_when_query_confirms_payment() {
    let fixture = expired_presented_order("wechat-created-paid").await;
    make_attempt_recoverable_wechat(&fixture, "wechat-created-paid", "created").await;
    let provider = FixedPaymentQueryProvider::returning(ProviderPaymentState::Paid {
        provider_transaction_id: "wechat-created-transaction".to_string(),
    });
    let operations = PaymentQueryOperations::new(
        fixture.db.clone(),
        fixture.key_ring.clone(),
        Arc::new(provider),
    );
    let reconciler = StoreReconciler::new(fixture.db.clone()).with_payment_queries(operations);

    let outcome = reconciler
        .run_once(
            "query-owner-wechat-created",
            Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(outcome.payments_applied, 1);
    assert_eq!(outcome.fulfilled, 1);
    let states = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT a.state AS attempt_state, o.payment_state, o.fulfillment_state
             FROM store_payment_attempts a
             JOIN store_orders o ON o.id = a.order_id WHERE a.id = $1",
            vec![fixture.attempt_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        states.try_get::<String>("", "attempt_state").unwrap(),
        "paid"
    );
    assert_eq!(
        states.try_get::<String>("", "payment_state").unwrap(),
        "paid"
    );
    assert_eq!(
        states.try_get::<String>("", "fulfillment_state").unwrap(),
        "fulfilled"
    );
}
