use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::adapters::alipay::AlipayCredential;
use monoize::store_billing::adapters::stripe::StripeCredential;
use monoize::store_billing::adapters::wechat::{WechatCredential, WechatPlatformVerifier};
use monoize::store_billing::callbacks::PaymentCallbackStore;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::operations::{PaymentQueryOperations, PaymentQueryProvider};
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderStore,
};
use monoize::store_billing::payment::{
    AdapterError, CheckoutAction, PaymentQuery, ProviderPaymentState, ProviderRefundState,
};
use monoize::store_billing::reconciliation::{ReconciliationError, StoreReconciler};
use monoize::store_billing::recovery::{BeginRefundInput, RecoveryStore};
use monoize::store_billing::refund_operations::{
    RefundOperations, RefundProvider, RefundProviderContract, RefundProviderOutcome,
};
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

#[derive(Clone)]
struct FixedRefundProvider {
    result: Result<RefundProviderOutcome, AdapterError>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FixedRefundProvider {
    fn returning(state: ProviderRefundState, provider_refund_id: Option<&str>) -> Self {
        Self {
            result: Ok(RefundProviderOutcome {
                state,
                provider_refund_id: provider_refund_id.map(str::to_string),
                not_found_is_definitive: false,
            }),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing(error: AdapterError) -> Self {
        Self {
            result: Err(error),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl RefundProvider for FixedRefundProvider {
    async fn create_refund(
        &self,
        _contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        panic!("refund reconciliation must not create a Provider refund")
    }

    async fn query_refund(
        &self,
        contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        self.calls
            .lock()
            .unwrap()
            .push(contract.request.idempotency_key.clone());
        self.result.clone()
    }
}

#[derive(Clone)]
struct LeaseStealingRefundProvider {
    db: DbPool,
}

#[async_trait]
impl RefundProvider for LeaseStealingRefundProvider {
    async fn create_refund(
        &self,
        _contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        panic!("refund reconciliation must not create a Provider refund")
    }

    async fn query_refund(
        &self,
        _contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        self.db
            .write()
            .await
            .execute_unprepared(
                "UPDATE store_reconciliation_leases
                 SET owner_id = 'refund-lease-thief', epoch = epoch + 1,
                     expires_at = '2099-01-01T00:00:00Z'
                 WHERE name = 'store_reconciler'",
            )
            .await
            .unwrap();
        Ok(RefundProviderOutcome {
            state: ProviderRefundState::Succeeded,
            provider_refund_id: Some("re_stolen_lease".to_string()),
            not_found_is_definitive: false,
        })
    }
}

struct PresentedFixture {
    db: DbPool,
    key_ring: Arc<PaymentKeyRing>,
    order_id: String,
    attempt_id: String,
}

async fn seed_reconciliation_governance(db: &DbPool, account_digest: &str) {
    let write = db.write().await;
    write
        .execute_unprepared(
            "INSERT INTO store_payment_compliance
                (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
             VALUES ('reconciliation-compliance', 'store-channel-stripe', '2026-08-28',
                     'reconciliation-admin', '127.0.0.1', '2026-08-27T00:00:00Z');
             INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('reconciliation-privacy', 'v1', 'CN', '[]', '{}', 'contract',
                     'reconciliation-admin',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z', 1)",
        )
        .await
        .unwrap();
    for capability in [
        "payment_query",
        "refund",
        "refund_query",
        "settlement_report",
    ] {
        write
            .execute(db.stmt(
                "INSERT INTO store_merchant_capabilities
                    (id, channel_id, capability, state, environment, merchant_account_digest,
                     provider_product, evidence_digest, verifier_admin_id, verified_at, expires_at)
                 VALUES ($1, 'store-channel-stripe', $2, 'supported', 'sandbox', $3,
                         'reconciliation',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                         'reconciliation-admin', '2026-08-27T00:00:00Z',
                         '2099-01-01T00:00:00Z')",
                vec![
                    format!("reconciliation-capability-{capability}").into(),
                    capability.into(),
                    account_digest.into(),
                ],
            ))
            .await
            .unwrap();
    }
    write
        .execute(db.stmt(
            "INSERT INTO store_channel_readiness_profiles
                (channel_id, active_credential_digest, privacy_record_id,
                 callback_verification_passed, supported_currencies_json, amount_limits_json,
                 checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                 availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES ('store-channel-stripe', $1, 'reconciliation-privacy', 1,
                     '[\"CNY\",\"USD\"]',
                     '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                     '[\"redirect\"]',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'reconciliation-admin', '2026-08-27T00:00:00Z',
                     '2099-01-01T00:00:00Z')",
            vec![account_digest.into()],
        ))
        .await
        .unwrap();
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
                account_digest.clone().into(),
            ],
        ))
        .await
        .unwrap();
    seed_reconciliation_governance(&db, &account_digest).await;

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
    let credential_json = br#"{
        "merchant_id":"1900000109",
        "app_id":"wx1234567890",
        "api_v3_key":"0123456789abcdef0123456789abcdef",
        "merchant_certificate_serial":"MERCHANT-CERTIFICATE-RECOVERY",
        "merchant_private_key_pem":"private",
        "platform_certificate_serial":"PLATFORM-CERTIFICATE-RECOVERY",
        "platform_public_key_pem":"public"
    }"#;
    let account_digest = WechatCredential::from_json(credential_json)
        .unwrap()
        .account_identity_digest();
    let encrypted = fixture
        .key_ring
        .encrypt(
            &format!("store_channel_credentials:{credential_id}:secret"),
            credential_json,
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
    seed_reconciliation_governance(
        &db,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await;
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

struct RefundPendingFixture {
    db: DbPool,
    key_ring: Arc<PaymentKeyRing>,
    order_id: String,
    refund_id: String,
    pending_at: chrono::DateTime<Utc>,
    reserved_nano_usd: String,
}

async fn refund_pending_order(suffix: &str) -> RefundPendingFixture {
    let fixture = expired_presented_order(suffix).await;
    let pending_at = Utc.with_ymd_and_hms(2026, 8, 27, 1, 0, 0).unwrap();
    {
        let write = fixture.db.write().await;
        write
            .execute(fixture.db.stmt(
                "UPDATE store_payment_attempts
                 SET state = 'paid', provider_transaction_id = $2,
                     paid_at = $3, updated_at = $3
                 WHERE id = $1",
                vec![
                    fixture.attempt_id.clone().into(),
                    format!("pi_refund_{suffix}").into(),
                    "2026-08-27T00:30:00Z".into(),
                ],
            ))
            .await
            .unwrap();
        write
            .execute(fixture.db.stmt(
                "UPDATE store_orders
                 SET payment_state = 'paid', paid_at = $2, updated_at = $2,
                     state_revision = state_revision + 1
                 WHERE id = $1",
                vec![
                    fixture.order_id.clone().into(),
                    "2026-08-27T00:30:00Z".into(),
                ],
            ))
            .await
            .unwrap();
    }
    PaymentCallbackStore::new(fixture.db.clone())
        .fulfill_paid_order(&fixture.order_id)
        .await
        .unwrap();
    let recovery = RecoveryStore::new(fixture.db.clone());
    let refund = recovery
        .begin_refund(BeginRefundInput {
            order_id: fixture.order_id.clone(),
            requested_by_admin_id: "refund-reconcile-admin".to_string(),
            idempotency_key: format!("refund-reconcile-{suffix}"),
        })
        .await
        .unwrap();
    let refund = recovery
        .mark_refund_pending_outcome(&refund.id, Some(&format!("re_{suffix}")))
        .await
        .unwrap();
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "UPDATE store_orders SET refund_pending_at = $2 WHERE id = $1",
            vec![
                fixture.order_id.clone().into(),
                pending_at.to_rfc3339().into(),
            ],
        ))
        .await
        .unwrap();
    let reserve = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT reserved_nano_usd FROM store_order_reward_recoveries WHERE order_id = $1",
            vec![fixture.order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "reserved_nano_usd")
        .unwrap();
    assert_ne!(reserve, "0");
    RefundPendingFixture {
        db: fixture.db,
        key_ring: fixture.key_ring,
        order_id: fixture.order_id,
        refund_id: refund.id,
        pending_at,
        reserved_nano_usd: reserve,
    }
}

fn refund_reconciler(
    fixture: &RefundPendingFixture,
    provider: FixedRefundProvider,
) -> StoreReconciler {
    StoreReconciler::new(fixture.db.clone()).with_refund_operations(RefundOperations::new(
        fixture.db.clone(),
        fixture.key_ring.clone(),
        Arc::new(provider),
    ))
}

#[tokio::test]
async fn refund_reconciliation_queries_after_one_minute_and_uses_documented_backoff() {
    let fixture = refund_pending_order("backoff").await;
    let provider = FixedRefundProvider::returning(ProviderRefundState::Pending, Some("re_backoff"));
    let calls = provider.calls.clone();
    let reconciler = refund_reconciler(&fixture, provider);

    let early = reconciler
        .run_once(
            "refund-owner",
            fixture.pending_at + chrono::Duration::seconds(59),
        )
        .await
        .unwrap();
    assert_eq!(early.refund_queries, 0);
    assert!(calls.lock().unwrap().is_empty());

    let schedule = [
        (
            1,
            chrono::Duration::minutes(1),
            chrono::Duration::minutes(5),
        ),
        (
            2,
            chrono::Duration::minutes(6),
            chrono::Duration::minutes(15),
        ),
        (3, chrono::Duration::minutes(21), chrono::Duration::hours(1)),
        (4, chrono::Duration::minutes(81), chrono::Duration::hours(1)),
    ];
    for (attempt_count, offset, delay) in schedule {
        let now = fixture.pending_at + offset;
        let outcome = reconciler.run_once("refund-owner", now).await.unwrap();
        assert_eq!(outcome.refund_queries, 1);
        let retry = fixture
            .db
            .read()
            .query_one(fixture.db.stmt(
                "SELECT attempt_count, next_attempt_at, last_error_category
                 FROM store_refund_query_retries WHERE refund_id = $1",
                vec![fixture.refund_id.clone().into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retry.try_get::<i64>("", "attempt_count").unwrap(),
            attempt_count
        );
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(
                &retry.try_get::<String>("", "next_attempt_at").unwrap()
            )
            .unwrap()
            .with_timezone(&Utc),
            now + delay
        );
        assert_eq!(
            retry
                .try_get::<Option<String>>("", "last_error_category")
                .unwrap(),
            None
        );
    }
    assert_eq!(calls.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn refund_reconciliation_alerts_at_fifteen_minutes_without_changing_query_backoff() {
    let fixture = refund_pending_order("alert-only").await;
    let provider =
        FixedRefundProvider::returning(ProviderRefundState::Pending, Some("re_alert-only"));
    let calls = provider.calls.clone();
    let reconciler = refund_reconciler(&fixture, provider);

    for offset in [chrono::Duration::minutes(1), chrono::Duration::minutes(6)] {
        let outcome = reconciler
            .run_once("refund-alert-owner", fixture.pending_at + offset)
            .await
            .unwrap();
        assert_eq!(outcome.refund_queries, 1);
    }

    let alert_at = fixture.pending_at + chrono::Duration::minutes(15);
    for now in [alert_at, alert_at + chrono::Duration::seconds(1)] {
        let outcome = reconciler
            .run_once("refund-alert-owner", now)
            .await
            .unwrap();
        assert_eq!(outcome.refund_queries, 0);
    }
    assert_eq!(calls.lock().unwrap().len(), 2);

    let state = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT q.attempt_count, q.next_attempt_at, q.last_error_category, q.alerted_at,
                    f.state AS refund_state, o.payment_state,
                    (SELECT COUNT(*) FROM store_reconciliation_cases c
                     WHERE c.id = $2 AND c.state = 'open') AS case_count
             FROM store_refund_query_retries q
             JOIN store_refunds f ON f.id = q.refund_id
             JOIN store_orders o ON o.id = f.order_id
             WHERE q.refund_id = $1",
            vec![
                fixture.refund_id.clone().into(),
                format!("refund-pending:{}", fixture.refund_id).into(),
            ],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.try_get::<i64>("", "attempt_count").unwrap(), 2);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(
            &state.try_get::<String>("", "next_attempt_at").unwrap()
        )
        .unwrap()
        .with_timezone(&Utc),
        fixture.pending_at + chrono::Duration::minutes(21)
    );
    assert_eq!(
        state
            .try_get::<Option<String>>("", "last_error_category")
            .unwrap(),
        None
    );
    assert!(
        state
            .try_get::<Option<String>>("", "alerted_at")
            .unwrap()
            .is_some()
    );
    assert_eq!(state.try_get::<i64>("", "case_count").unwrap(), 1);
    assert_eq!(
        state.try_get::<String>("", "refund_state").unwrap(),
        "pending"
    );
    assert_eq!(
        state.try_get::<String>("", "payment_state").unwrap(),
        "refund_pending"
    );
}

#[tokio::test]
async fn successful_refund_reconciliation_closes_case_and_deletes_retry() {
    let fixture = refund_pending_order("terminal").await;
    let due = fixture.pending_at + chrono::Duration::minutes(16);
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "INSERT INTO store_refund_query_retries
                (refund_id, attempt_count, next_attempt_at, last_error_category,
                 alerted_at, updated_at)
             VALUES ($1, 2, $2, NULL, $2, $2)",
            vec![fixture.refund_id.clone().into(), due.to_rfc3339().into()],
        ))
        .await
        .unwrap();
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "INSERT INTO store_reconciliation_cases
                (id, order_id, channel_id, severity, kind, state, evidence_json,
                 created_at, updated_at)
             VALUES ($1, $2, 'store-channel-stripe', 'high', 'refund_pending',
                     'open', '{}', $3, $3)",
            vec![
                format!("refund-pending:{}", fixture.refund_id).into(),
                fixture.order_id.clone().into(),
                due.to_rfc3339().into(),
            ],
        ))
        .await
        .unwrap();
    let provider =
        FixedRefundProvider::returning(ProviderRefundState::Succeeded, Some("re_terminal"));

    let outcome = refund_reconciler(&fixture, provider)
        .run_once("refund-terminal-owner", due)
        .await
        .unwrap();
    assert_eq!(outcome.refunds_terminal, 1);
    let state = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT f.state AS refund_state, o.payment_state, r.reserved_nano_usd,
                    r.recovered_nano_usd,
                    (SELECT COUNT(*) FROM store_refund_query_retries q
                     WHERE q.refund_id = f.id) AS retry_count,
                    (SELECT state FROM store_reconciliation_cases c
                     WHERE c.id = $2) AS case_state
             FROM store_refunds f
             JOIN store_orders o ON o.id = f.order_id
             JOIN store_order_reward_recoveries r ON r.order_id = o.id
             WHERE f.id = $1",
            vec![
                fixture.refund_id.clone().into(),
                format!("refund-pending:{}", fixture.refund_id).into(),
            ],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state.try_get::<String>("", "refund_state").unwrap(),
        "succeeded"
    );
    assert_eq!(
        state.try_get::<String>("", "payment_state").unwrap(),
        "refunded"
    );
    assert_eq!(state.try_get::<i64>("", "retry_count").unwrap(), 0);
    assert_eq!(state.try_get::<String>("", "case_state").unwrap(), "closed");
    assert_eq!(
        state.try_get::<String>("", "recovered_nano_usd").unwrap(),
        fixture.reserved_nano_usd
    );
}

#[tokio::test]
async fn refund_query_error_keeps_reserve_and_opens_one_case_after_fifteen_minutes() {
    let fixture = refund_pending_order("error").await;
    let provider = FixedRefundProvider::failing(AdapterError::Unsupported);
    let now = fixture.pending_at + chrono::Duration::minutes(16);
    let outcome = refund_reconciler(&fixture, provider)
        .run_once("refund-error-owner", now)
        .await
        .unwrap();
    assert_eq!(outcome.refund_query_failures, 1);

    let state = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT f.state AS refund_state, o.payment_state, r.reserved_nano_usd,
                    q.attempt_count, q.last_error_category, q.alerted_at,
                    (SELECT COUNT(*) FROM store_reconciliation_cases c
                     WHERE c.id = $2 AND c.state = 'open') AS case_count
             FROM store_refunds f
             JOIN store_orders o ON o.id = f.order_id
             JOIN store_order_reward_recoveries r ON r.order_id = o.id
             JOIN store_refund_query_retries q ON q.refund_id = f.id
             WHERE f.id = $1",
            vec![
                fixture.refund_id.clone().into(),
                format!("refund-pending:{}", fixture.refund_id).into(),
            ],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state.try_get::<String>("", "refund_state").unwrap(),
        "pending"
    );
    assert_eq!(
        state.try_get::<String>("", "payment_state").unwrap(),
        "refund_pending"
    );
    assert_eq!(
        state.try_get::<String>("", "reserved_nano_usd").unwrap(),
        fixture.reserved_nano_usd
    );
    assert_eq!(state.try_get::<i64>("", "attempt_count").unwrap(), 1);
    assert_eq!(
        state.try_get::<String>("", "last_error_category").unwrap(),
        "payment_configuration_unavailable"
    );
    assert!(
        state
            .try_get::<Option<String>>("", "alerted_at")
            .unwrap()
            .is_some()
    );
    assert_eq!(state.try_get::<i64>("", "case_count").unwrap(), 1);
}

#[tokio::test]
async fn refund_provider_success_after_lease_loss_does_not_mutate_economic_state() {
    let fixture = refund_pending_order("lost-lease").await;
    let reconciler =
        StoreReconciler::new(fixture.db.clone()).with_refund_operations(RefundOperations::new(
            fixture.db.clone(),
            fixture.key_ring.clone(),
            Arc::new(LeaseStealingRefundProvider {
                db: fixture.db.clone(),
            }),
        ));

    let result = reconciler
        .run_once(
            "refund-original-owner",
            fixture.pending_at + chrono::Duration::minutes(1),
        )
        .await;
    assert_eq!(result, Err(ReconciliationError::LeaseLost));

    let state = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT f.state AS refund_state, o.payment_state,
                    r.reserved_nano_usd, r.recovered_nano_usd,
                    (SELECT COUNT(*) FROM store_refund_query_retries q
                     WHERE q.refund_id = f.id) AS retry_count
             FROM store_refunds f
             JOIN store_orders o ON o.id = f.order_id
             JOIN store_order_reward_recoveries r ON r.order_id = o.id
             WHERE f.id = $1",
            vec![fixture.refund_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state.try_get::<String>("", "refund_state").unwrap(),
        "pending"
    );
    assert_eq!(
        state.try_get::<String>("", "payment_state").unwrap(),
        "refund_pending"
    );
    assert_eq!(
        state.try_get::<String>("", "reserved_nano_usd").unwrap(),
        fixture.reserved_nano_usd
    );
    assert_eq!(
        state.try_get::<String>("", "recovered_nano_usd").unwrap(),
        "0"
    );
    assert_eq!(state.try_get::<i64>("", "retry_count").unwrap(), 0);
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
