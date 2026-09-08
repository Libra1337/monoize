use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use tokio::sync::Barrier;
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::adapters::alipay::AlipayCredential;
use monoize::store_billing::adapters::stripe::StripeCredential;
use monoize::store_billing::adapters::wechat::{WechatCredential, WechatPlatformVerifier};
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::operations::{
    AdminOrderOperationError, AdminOrderOperations, PaymentOperationsError,
    PaymentQueryOperations, PaymentQueryProvider,
};
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentAttemptFailureKind,
    PaymentOrderStore,
};
use monoize::store_billing::payment::{AdapterError, PaymentQuery, ProviderPaymentState};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedQuery {
    adapter_kind: String,
    account_id: String,
    platform_certificate_serials: Vec<String>,
    query: PaymentQuery,
}

#[derive(Clone)]
struct RecordingQueryProvider {
    calls: Arc<Mutex<Vec<RecordedQuery>>>,
    outcome: Result<ProviderPaymentState, AdapterError>,
}

impl RecordingQueryProvider {
    fn returning(state: ProviderPaymentState) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            outcome: Ok(state),
        }
    }

    fn failing(error: AdapterError) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            outcome: Err(error),
        }
    }

    fn record(
        &self,
        adapter_kind: &str,
        account_id: &str,
        platform_certificate_serials: Vec<String>,
        query: &PaymentQuery,
    ) {
        self.calls.lock().unwrap().push(RecordedQuery {
            adapter_kind: adapter_kind.to_string(),
            account_id: account_id.to_string(),
            platform_certificate_serials,
            query: query.clone(),
        });
    }
}

#[derive(Clone)]
struct BarrierQueryProvider {
    calls: Arc<Mutex<usize>>,
    barrier: Arc<Barrier>,
}

impl BarrierQueryProvider {
    fn new(parties: usize) -> Self {
        Self {
            calls: Arc::new(Mutex::new(0)),
            barrier: Arc::new(Barrier::new(parties)),
        }
    }
}

#[async_trait]
impl PaymentQueryProvider for BarrierQueryProvider {
    async fn query_stripe_payment(
        &self,
        _credential: &StripeCredential,
        _query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        *self.calls.lock().unwrap() += 1;
        self.barrier.wait().await;
        Ok(ProviderPaymentState::Unpaid)
    }

    async fn query_alipay_payment(
        &self,
        _credential: &AlipayCredential,
        _query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        unreachable!()
    }

    async fn query_wechat_payment(
        &self,
        _credential: &WechatCredential,
        _verifiers: &[WechatPlatformVerifier],
        _query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        unreachable!()
    }
}

#[async_trait]
impl PaymentQueryProvider for RecordingQueryProvider {
    async fn query_stripe_payment(
        &self,
        credential: &StripeCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.record("stripe", credential.account_id(), vec![], query);
        self.outcome.clone()
    }

    async fn query_alipay_payment(
        &self,
        credential: &AlipayCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.record("alipay", credential.seller_id(), vec![], query);
        self.outcome.clone()
    }

    async fn query_wechat_payment(
        &self,
        credential: &WechatCredential,
        verifiers: &[WechatPlatformVerifier],
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.record(
            "wechat",
            credential.merchant_id(),
            verifiers
                .iter()
                .map(|verifier| verifier.certificate_serial().to_string())
                .collect(),
            query,
        );
        self.outcome.clone()
    }
}

struct OperationsFixture {
    db: DbPool,
    key_ring: PaymentKeyRing,
    attempt_id: String,
    order_id: String,
    order_number: String,
    credential_id: String,
    provider_object_id: String,
    account_id: String,
    account_digest: String,
    platform_certificate_serials: Vec<String>,
}

async fn operations_fixture(adapter_kind: &str) -> OperationsFixture {
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
                    ('operations-product', 'balance', 'Recharge', '', 'CNY', '1000',
                     NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
                 VALUES ('operations-product', '1000', '0')",
            )
            .await
            .unwrap();
    }

    let channel_id = format!("store-channel-{adapter_kind}");
    let credential_id = format!("operations-{adapter_kind}-credential");
    let (account_id, credential_json): (&str, &[u8]) = match adapter_kind {
        "stripe" => (
            "acct_operations",
            br#"{
                "secret_key":"sk_test_operations",
                "publishable_key":"pk_test_operations",
                "webhook_signing_secret":"whsec_operations",
                "api_version":"2026-08-01",
                "account_id":"acct_operations",
                "live_mode":false
            }"#,
        ),
        "alipay" => (
            "2088000000000001",
            br#"{
                "app_id":"2026000000000001",
                "seller_id":"2088000000000001",
                "merchant_private_key_pem":"private",
                "alipay_public_key_pem":"public",
                "environment":"sandbox"
            }"#,
        ),
        "wechat" => (
            "1900000109",
            br#"{
                "merchant_id":"1900000109",
                "app_id":"wx1234567890",
                "api_v3_key":"0123456789abcdef0123456789abcdef",
                "merchant_certificate_serial":"MERCHANT-CERTIFICATE-1",
                "merchant_private_key_pem":"private",
                "platform_certificate_serial":"PLATFORM-CERTIFICATE-1",
                "platform_public_key_pem":"public"
            }"#,
        ),
        _ => panic!("unsupported fixture adapter"),
    };
    let account_digest = if adapter_kind == "wechat" {
        WechatCredential::from_json(credential_json)
            .unwrap()
            .account_identity_digest()
    } else {
        Sha256::digest(account_id.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let key_ring = PaymentKeyRing::new(
        PaymentKey::new("operations-key", [23_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let encrypted = key_ring
        .encrypt(
            &format!("store_channel_credentials:{credential_id}:secret"),
            credential_json,
        )
        .unwrap();
    {
        let write = db.write().await;
        write
            .execute(db.stmt(
                "UPDATE store_payment_channels SET enabled = 1 WHERE id = $1",
                vec![channel_id.clone().into()],
            ))
            .await
            .unwrap();
        write
            .execute(db.stmt(
                "INSERT INTO store_channel_credentials
                    (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                     ciphertext_base64, account_identity_digest, status, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9)",
                vec![
                    credential_id.clone().into(),
                    channel_id.clone().into(),
                    adapter_kind.into(),
                    i32::from(encrypted.version).into(),
                    encrypted.key_id.into(),
                    encrypted.nonce_base64.into(),
                    encrypted.ciphertext_base64.into(),
                    account_digest.clone().into(),
                    "2026-08-27T00:00:00Z".into(),
                ],
            ))
            .await
            .unwrap();
        let (currencies, limits, actions) = match adapter_kind {
            "stripe" => (
                "[\"CNY\"]",
                "{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}",
                "[\"redirect\"]",
            ),
            "alipay" => (
                "[\"CNY\"]",
                "{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}",
                "[\"form\"]",
            ),
            "wechat" => (
                "[\"CNY\"]",
                "{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}",
                "[\"qr\"]",
            ),
            _ => unreachable!(),
        };
        write
            .execute(db.stmt(
                "INSERT INTO store_payment_compliance
                    (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
                 VALUES ($1, $2, '2026-08-28', 'operations-admin', '127.0.0.1',
                         '2026-08-28T00:00:00Z')",
                vec![format!("operations-{adapter_kind}-compliance").into(), channel_id.clone().into()],
            ))
            .await
            .unwrap();
        for capability in ["payment_query", "refund", "refund_query", "settlement_report"] {
            write
                .execute(db.stmt(
                    "INSERT INTO store_merchant_capabilities
                        (id, channel_id, capability, state, environment, merchant_account_digest,
                         provider_product, evidence_digest, verifier_admin_id, verified_at, expires_at)
                     VALUES ($1, $2, $3, 'supported', 'sandbox', $4, 'checkout',
                             'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                             'operations-admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z')",
                    vec![
                        format!("operations-{adapter_kind}-{capability}").into(),
                        channel_id.clone().into(),
                        capability.into(),
                        account_digest.clone().into(),
                    ],
                ))
                .await
                .unwrap();
        }
        write
            .execute(db.stmt(
                "INSERT INTO store_privacy_records
                    (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                     legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
                 VALUES ($1, 'v1', 'CN', '[]', '{}', 'contract', 'operations-admin',
                         'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                         '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z', 1)",
                vec![format!("operations-{adapter_kind}-privacy").into()],
            ))
            .await
            .unwrap();
        write
            .execute(db.stmt(
                "INSERT INTO store_channel_readiness_profiles
                    (channel_id, active_credential_digest, privacy_record_id,
                     callback_verification_passed, supported_currencies_json, amount_limits_json,
                     checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                     availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
                 VALUES ($1, $2, $3, 1, $4, $5, $6,
                         'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                         'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                         'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                         'operations-admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z')",
                vec![
                    channel_id.clone().into(),
                    account_digest.clone().into(),
                    format!("operations-{adapter_kind}-privacy").into(),
                    currencies.into(),
                    limits.into(),
                    actions.into(),
                ],
            ))
            .await
            .unwrap();
    }

    let orders = PaymentOrderStore::new(db.clone());
    let order = orders
        .create_order(
            "operations-user",
            CreatePaymentOrderInput {
                idempotency_key: format!("operations-{adapter_kind}-order"),
                product_id: "operations-product".to_string(),
                payment_channel_id: channel_id.clone(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &ExchangeRateSnapshot {
                base: "USD".to_string(),
                quote: "CNY".to_string(),
                cny_per_usd: "6.7370".to_string(),
                source_updated_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 0, 0).unwrap(),
                refreshed_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
            },
        )
        .await
        .unwrap();
    let attempt = orders
        .create_attempt(
            "operations-user",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: format!("operations-{adapter_kind}-attempt"),
                expected_payment_method: None,
            },
        )
        .await
        .unwrap();
    let provider_object_id = if adapter_kind == "stripe" {
        "cs_operations_1".to_string()
    } else {
        order.order_number.clone()
    };
    {
        let write = db.write().await;
        write
            .execute(db.stmt(
                "UPDATE store_payment_attempts SET provider_object_id = $2 WHERE id = $1",
                vec![attempt.id.clone().into(), provider_object_id.clone().into()],
            ))
            .await
            .unwrap();
        write
            .execute(db.stmt(
                "UPDATE store_channel_credentials
                 SET status = 'retired', retired_at = $2 WHERE id = $1",
                vec![credential_id.clone().into(), "2026-08-27T00:02:00Z".into()],
            ))
            .await
            .unwrap();
        if adapter_kind == "wechat" {
            let rotated_credential_id = "operations-wechat-credential-rotated";
            let rotated = key_ring
                .encrypt(
                    &format!("store_channel_credentials:{rotated_credential_id}:secret"),
                    br#"{
                        "merchant_id":"1900000109",
                        "app_id":"wx1234567890",
                        "api_v3_key":"0123456789abcdef0123456789abcdef",
                        "merchant_certificate_serial":"MERCHANT-CERTIFICATE-1",
                        "merchant_private_key_pem":"private",
                        "platform_certificate_serial":"PLATFORM-CERTIFICATE-2",
                        "platform_public_key_pem":"public-rotated"
                    }"#,
                )
                .unwrap();
            write
                .execute(db.stmt(
                    "INSERT INTO store_channel_credentials
                        (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                         ciphertext_base64, account_identity_digest, status, created_at)
                     VALUES ($1, $2, 'wechat', $3, $4, $5, $6, $7, 'active', $8)",
                    vec![
                        rotated_credential_id.into(),
                        channel_id.clone().into(),
                        i32::from(rotated.version).into(),
                        rotated.key_id.into(),
                        rotated.nonce_base64.into(),
                        rotated.ciphertext_base64.into(),
                        account_digest.clone().into(),
                        "2026-08-27T00:03:00Z".into(),
                    ],
                ))
                .await
                .unwrap();
        }
    }

    OperationsFixture {
        db,
        key_ring,
        attempt_id: attempt.id,
        order_id: order.id,
        order_number: order.order_number,
        credential_id,
        provider_object_id,
        account_id: account_id.to_string(),
        account_digest,
        platform_certificate_serials: if adapter_kind == "wechat" {
            vec![
                "PLATFORM-CERTIFICATE-2".to_string(),
                "PLATFORM-CERTIFICATE-1".to_string(),
            ]
        } else {
            vec![]
        },
    }
}

#[tokio::test]
async fn query_attempt_dispatches_each_historical_credential_with_exact_contract() {
    for adapter_kind in ["stripe", "alipay", "wechat"] {
        let fixture = operations_fixture(adapter_kind).await;
        let expected_state = ProviderPaymentState::Paid {
            provider_transaction_id: format!("transaction-{adapter_kind}"),
        };
        let provider = RecordingQueryProvider::returning(expected_state.clone());
        let operations = PaymentQueryOperations::new(
            fixture.db,
            Arc::new(fixture.key_ring),
            Arc::new(provider.clone()),
        );

        let result = operations
            .query_attempt_with_context(&fixture.attempt_id)
            .await
            .unwrap();

        assert_eq!(result.state, expected_state);
        assert_eq!(result.attempt_id, fixture.attempt_id);
        assert_eq!(result.order_id, fixture.order_id);
        assert_eq!(result.credential_version_id, fixture.credential_id);
        assert_eq!(result.provider_object_id, fixture.provider_object_id);
        assert_eq!(result.merchant_account_identity, fixture.account_digest);
        assert_eq!(result.order_number, fixture.order_number);
        assert_eq!(result.amount_minor, "1000");
        assert_eq!(result.currency, Currency::CNY);
        assert!(!result.payment_hold);
        assert_eq!(
            provider.calls.lock().unwrap().as_slice(),
            &[RecordedQuery {
                adapter_kind: adapter_kind.to_string(),
                account_id: fixture.account_id,
                platform_certificate_serials: fixture.platform_certificate_serials,
                query: PaymentQuery {
                    provider_object_id: result.provider_object_id,
                    merchant_order_number: result.order_number,
                    amount_minor: "1000".to_string(),
                    currency: Currency::CNY,
                },
            }]
        );
    }
}

#[tokio::test]
async fn query_attempt_returns_provider_state_without_losing_it() {
    let fixture = operations_fixture("stripe").await;
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Ambiguous);
    let operations =
        PaymentQueryOperations::new(fixture.db, Arc::new(fixture.key_ring), Arc::new(provider));

    assert_eq!(
        operations.query_attempt(&fixture.attempt_id).await.unwrap(),
        ProviderPaymentState::Ambiguous
    );
}

#[tokio::test]
async fn query_attempt_rejects_missing_historical_credential() {
    let fixture = operations_fixture("stripe").await;
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "DELETE FROM store_channel_credentials WHERE id = $1",
            vec![fixture.credential_id.clone().into()],
        ))
        .await
        .unwrap();
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
    let operations = PaymentQueryOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    assert_eq!(
        operations
            .query_attempt(&fixture.attempt_id)
            .await
            .unwrap_err(),
        PaymentOperationsError::CredentialNotFound
    );
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn query_attempt_reports_historical_credential_decryption_failure() {
    let fixture = operations_fixture("stripe").await;
    let unrelated_key_ring = PaymentKeyRing::new(
        PaymentKey::new("unrelated-key", [91_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
    let operations = PaymentQueryOperations::new(
        fixture.db,
        Arc::new(unrelated_key_ring),
        Arc::new(provider.clone()),
    );

    assert_eq!(
        operations
            .query_attempt(&fixture.attempt_id)
            .await
            .unwrap_err(),
        PaymentOperationsError::CredentialDecryptionFailed
    );
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn query_attempt_rejects_credential_binding_mismatch() {
    let fixture = operations_fixture("stripe").await;
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "UPDATE store_payment_attempts SET merchant_account_identity = $2 WHERE id = $1",
            vec![fixture.attempt_id.clone().into(), "0".repeat(64).into()],
        ))
        .await
        .unwrap();
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
    let operations = PaymentQueryOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    assert_eq!(
        operations
            .query_attempt(&fixture.attempt_id)
            .await
            .unwrap_err(),
        PaymentOperationsError::CredentialBindingMismatch
    );
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn query_attempt_rejects_a_missing_provider_object_contract() {
    let fixture = operations_fixture("stripe").await;
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "UPDATE store_payment_attempts SET provider_object_id = NULL WHERE id = $1",
            vec![fixture.attempt_id.clone().into()],
        ))
        .await
        .unwrap();
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
    let operations = PaymentQueryOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    assert_eq!(
        operations
            .query_attempt(&fixture.attempt_id)
            .await
            .unwrap_err(),
        PaymentOperationsError::PaymentContractInvalid
    );
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn query_attempt_uses_the_merchant_order_for_wechat_without_a_provider_object() {
    let fixture = operations_fixture("wechat").await;
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "UPDATE store_payment_attempts SET provider_object_id = NULL WHERE id = $1",
            vec![fixture.attempt_id.clone().into()],
        ))
        .await
        .unwrap();
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::NotFound);
    let operations = PaymentQueryOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    let result = operations
        .query_attempt_with_context(&fixture.attempt_id)
        .await
        .unwrap();
    assert_eq!(result.provider_object_id, fixture.order_number);
    assert_eq!(result.attempt_state, "created");
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn admin_confirmed_nonpaid_states_expire_only_the_selected_attempt_and_close_v2_order() {
    for (state, expected_kind) in [
        (ProviderPaymentState::NotFound, "not_found"),
        (ProviderPaymentState::Unpaid, "unpaid"),
        (ProviderPaymentState::Closed, "closed"),
    ] {
        let fixture = operations_fixture("stripe").await;
        let provider = RecordingQueryProvider::returning(state);
        let operations = AdminOrderOperations::new(
            fixture.db.clone(),
            Arc::new(fixture.key_ring),
            Arc::new(provider),
        );

        let result = operations
            .close(&fixture.order_id, &fixture.attempt_id)
            .await
            .unwrap();

        assert_eq!(result.provider_state.kind, expected_kind);
        assert_eq!(result.provider_state.provider_transaction_id, None);
        assert!(result.closed);
        assert_eq!(result.order.payment_state.as_str(), "closed");
        assert_eq!(result.attempt.state.as_str(), "expired");
    }
}

#[tokio::test]
async fn admin_order_detail_sorts_tied_attempts_and_refunds_by_id() {
    let fixture = operations_fixture("stripe").await;
    let write = fixture.db.write().await;
    write
        .execute(fixture.db.stmt(
            "UPDATE store_payment_attempts SET created_at = $2, updated_at = $2 WHERE id = $1",
            vec![
                fixture.attempt_id.clone().into(),
                "2026-08-29T01:00:00Z".into(),
            ],
        ))
        .await
        .unwrap();
    for id in ["attempt-z", "attempt-a"] {
        write
            .execute(fixture.db.stmt(
                "INSERT INTO store_payment_attempts
                    (id, order_id, channel_id, adapter_kind, credential_version_id,
                     merchant_account_identity, expected_payment_method,
                     payment_contract_version, state, failure_kind,
                     provider_transaction_id, provider_object_id, idempotency_key,
                     action_kind, action_json, provider_expires_at, presented_at,
                     paid_at, created_at, updated_at)
                 SELECT $1, order_id, channel_id, adapter_kind, credential_version_id,
                        merchant_account_identity, expected_payment_method,
                        payment_contract_version, 'expired', NULL, NULL,
                        provider_object_id, $2, action_kind, action_json,
                        provider_expires_at, presented_at, NULL, $3, $3
                 FROM store_payment_attempts WHERE id = $4",
                vec![
                    id.into(),
                    format!("detail-{id}").into(),
                    "2026-08-29T00:00:00Z".into(),
                    fixture.attempt_id.clone().into(),
                ],
            ))
            .await
            .unwrap();
    }
    write
        .execute(fixture.db.stmt(
            "INSERT INTO store_order_reward_recoveries
                (id, order_id, original_nano_usd, reserved_nano_usd,
                 recovered_nano_usd, state, created_at, updated_at)
             VALUES ('detail-recovery', $1, '100', '0', '0', 'open', $2, $2)",
            vec![
                fixture.order_id.clone().into(),
                "2026-08-29T00:00:00Z".into(),
            ],
        ))
        .await
        .unwrap();
    for id in ["refund-z", "refund-a"] {
        write
            .execute(fixture.db.stmt(
                "INSERT INTO store_refunds
                    (id, order_id, attempt_id, provider_refund_id,
                     idempotency_key, state, amount_minor, currency,
                     requested_by_admin_id, created_at, updated_at)
                 VALUES ($1, $2, $3, NULL, $4, 'created', '1000', 'CNY',
                         'detail-admin', $5, $5)",
                vec![
                    id.into(),
                    fixture.order_id.clone().into(),
                    fixture.attempt_id.clone().into(),
                    format!("detail-{id}").into(),
                    "2026-08-29T00:00:00Z".into(),
                ],
            ))
            .await
            .unwrap();
        write
            .execute(fixture.db.stmt(
                "INSERT INTO store_order_recovery_claims
                    (id, recovery_id, credential_version_id, provider_claim_id,
                     provider_event_row_id, kind, amount_nano_usd, state, created_at)
                 VALUES ($1, 'detail-recovery', $2, $3, NULL, 'refund',
                         '100', 'open', $4)",
                vec![
                    format!("claim-{id}").into(),
                    fixture.credential_id.clone().into(),
                    format!("detail-{id}").into(),
                    "2026-08-29T00:00:00Z".into(),
                ],
            ))
            .await
            .unwrap();
    }
    drop(write);

    let detail = AdminOrderOperations::detail_from_db(&fixture.db, &fixture.order_id)
        .await
        .unwrap();
    assert_eq!(
        detail
            .attempts
            .iter()
            .map(|attempt| attempt.id.as_str())
            .collect::<Vec<_>>(),
        ["attempt-a", "attempt-z", fixture.attempt_id.as_str()]
    );
    assert_eq!(
        detail
            .refunds
            .iter()
            .map(|refund| refund.id.as_str())
            .collect::<Vec<_>>(),
        ["refund-a", "refund-z"]
    );
}

#[tokio::test]
async fn admin_query_requires_the_attempt_to_belong_to_the_path_order() {
    let fixture = operations_fixture("stripe").await;
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
    let operations = AdminOrderOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider),
    );

    assert_eq!(
        operations
            .query("another-order", &fixture.attempt_id)
            .await
            .unwrap_err(),
        AdminOrderOperationError::NotFound
    );
}

#[tokio::test]
async fn admin_order_identifiers_use_unicode_character_counts_and_reject_all_whitespace() {
    let fixture = operations_fixture("stripe").await;
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
    let operations = AdminOrderOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    assert_eq!(
        operations
            .query(&"界".repeat(128), &fixture.attempt_id)
            .await
            .unwrap_err(),
        AdminOrderOperationError::NotFound
    );
    for invalid in ["界".repeat(129), "attempt internal".to_string(), "attempt\u{3000}id".to_string()] {
        assert_eq!(
            operations
                .query(&fixture.order_id, &invalid)
                .await
                .unwrap_err(),
            AdminOrderOperationError::InvalidInput
        );
    }
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn admin_close_preflight_rejects_v1_nonunpaid_and_repeated_close_without_provider_calls() {
    for (contract_version, payment_state) in [(1, "unpaid"), (2, "paid")] {
        let fixture = operations_fixture("stripe").await;
        if contract_version == 1 {
            fixture
                .db
                .write()
                .await
                .execute_unprepared("DROP TRIGGER trg_store_orders_quote_immutable")
                .await
                .unwrap();
        }
        fixture
            .db
            .write()
            .await
            .execute(fixture.db.stmt(
                "UPDATE store_orders SET contract_version = $2, payment_state = $3 WHERE id = $1",
                vec![
                    fixture.order_id.clone().into(),
                    contract_version.into(),
                    payment_state.into(),
                ],
            ))
            .await
            .unwrap();
        let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
        let operations = AdminOrderOperations::new(
            fixture.db,
            Arc::new(fixture.key_ring),
            Arc::new(provider.clone()),
        );
        assert_eq!(
            operations
                .close(&fixture.order_id, &fixture.attempt_id)
                .await
                .unwrap_err(),
            AdminOrderOperationError::OrderNotPayable
        );
        assert!(provider.calls.lock().unwrap().is_empty());
    }

    let fixture = operations_fixture("stripe").await;
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::NotFound);
    let operations = AdminOrderOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );
    operations
        .close(&fixture.order_id, &fixture.attempt_id)
        .await
        .unwrap();
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
    assert_eq!(
        operations
            .close(&fixture.order_id, &fixture.attempt_id)
            .await
            .unwrap_err(),
        AdminOrderOperationError::OrderNotPayable
    );
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn admin_close_rejects_a_failed_selection_when_a_newer_attempt_is_presented() {
    let fixture = operations_fixture("stripe").await;
    let orders = PaymentOrderStore::new(fixture.db.clone());
    orders
        .fail_attempt(
            "operations-user",
            &fixture.attempt_id,
            PaymentAttemptFailureKind::ConfigurationUnavailable,
        )
        .await
        .unwrap();
    let newer_id = "newer-presented-attempt";
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "INSERT INTO store_payment_attempts
                (id, order_id, channel_id, adapter_kind, credential_version_id,
                 merchant_account_identity, expected_payment_method,
                 payment_contract_version, state, failure_kind,
                 provider_transaction_id, provider_object_id, idempotency_key,
                 action_kind, action_json, provider_expires_at, presented_at,
                 paid_at, created_at, updated_at)
             SELECT $1, order_id, channel_id, adapter_kind, credential_version_id,
                    merchant_account_identity, 'card', payment_contract_version,
                    'presented', NULL, NULL, 'cs_newer_presented', $2,
                    'redirect', $3, '2099-01-01T00:00:00Z', $4, NULL, $4, $4
             FROM store_payment_attempts WHERE id = $5",
            vec![
                newer_id.into(),
                "newer-presented-attempt-key".into(),
                r#"{"kind":"redirect","url":"https://checkout.example/newer","expires_at":"2099-01-01T00:00:00Z"}"#.into(),
                "2026-08-29T00:00:00Z".into(),
                fixture.attempt_id.clone().into(),
            ],
        ))
        .await
        .unwrap();
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
    let operations = AdminOrderOperations::new(
        fixture.db.clone(),
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    assert_eq!(
        operations
            .close(&fixture.order_id, &fixture.attempt_id)
            .await
            .unwrap_err(),
        AdminOrderOperationError::OrderNotPayable
    );
    let detail = operations.detail(&fixture.order_id).await.unwrap();
    assert_eq!(detail.order.payment_state.as_str(), "unpaid");
    assert_eq!(
        detail
            .attempts
            .iter()
            .find(|attempt| attempt.id == fixture.attempt_id)
            .unwrap()
            .state
            .as_str(),
        "failed"
    );
    assert_eq!(
        detail
            .attempts
            .iter()
            .find(|attempt| attempt.id == newer_id)
            .unwrap()
            .state
            .as_str(),
        "presented"
    );
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_admin_closes_allow_exactly_one_state_transition() {
    let fixture = operations_fixture("stripe").await;
    let provider = BarrierQueryProvider::new(2);
    let operations = AdminOrderOperations::new(
        fixture.db.clone(),
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );
    let first = operations.clone();
    let second = operations.clone();
    let first_order_id = fixture.order_id.clone();
    let first_attempt_id = fixture.attempt_id.clone();
    let second_order_id = fixture.order_id.clone();
    let second_attempt_id = fixture.attempt_id.clone();

    let (first_result, second_result) = tokio::join!(
        async move { first.close(&first_order_id, &first_attempt_id).await },
        async move { second.close(&second_order_id, &second_attempt_id).await },
    );
    let results = [first_result, second_result];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(AdminOrderOperationError::OrderNotPayable)))
            .count(),
        1
    );
    assert_eq!(*provider.calls.lock().unwrap(), 2);
    let detail = operations.detail(&fixture.order_id).await.unwrap();
    assert_eq!(detail.order.payment_state.as_str(), "closed");
    assert_eq!(detail.order.state_revision, 1);
    assert_eq!(detail.attempts[0].state.as_str(), "expired");
}

#[tokio::test]
async fn concurrent_attempt_creation_and_admin_close_preserve_one_valid_outcome() {
    let fixture = operations_fixture("stripe").await;
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "UPDATE store_channel_credentials
             SET status = 'active', retired_at = NULL WHERE id = $1",
            vec![fixture.credential_id.clone().into()],
        ))
        .await
        .unwrap();
    let orders = PaymentOrderStore::new(fixture.db.clone());
    orders
        .fail_attempt(
            "operations-user",
            &fixture.attempt_id,
            PaymentAttemptFailureKind::ConfigurationUnavailable,
        )
        .await
        .unwrap();
    let provider = BarrierQueryProvider::new(2);
    let barrier = provider.barrier.clone();
    let operations = AdminOrderOperations::new(
        fixture.db.clone(),
        Arc::new(fixture.key_ring),
        Arc::new(provider),
    );
    let close_operations = operations.clone();
    let close_order_id = fixture.order_id.clone();
    let close_attempt_id = fixture.attempt_id.clone();
    let create_order_id = fixture.order_id.clone();

    let (close_result, create_result) = tokio::join!(
        async move {
            close_operations
                .close(&close_order_id, &close_attempt_id)
                .await
        },
        async move {
            barrier.wait().await;
            orders
                .create_attempt(
                    "operations-user",
                    &create_order_id,
                    CreatePaymentAttemptInput {
                        idempotency_key: "concurrent-new-attempt".to_string(),
                        expected_payment_method: Some("card".to_string()),
                    },
                )
                .await
        },
    );
    assert_ne!(close_result.is_ok(), create_result.is_ok());
    let detail = operations.detail(&fixture.order_id).await.unwrap();
    if close_result.is_ok() {
        assert_eq!(
            create_result.unwrap_err(),
            monoize::store_billing::order::PaymentOrderError::OrderNotPayable
        );
        assert_eq!(detail.order.payment_state.as_str(), "closed");
        assert_eq!(detail.attempts.len(), 1);
        assert_eq!(detail.attempts[0].state.as_str(), "expired");
    } else {
        assert_eq!(
            close_result.unwrap_err(),
            AdminOrderOperationError::OrderNotPayable
        );
        assert_eq!(detail.order.payment_state.as_str(), "unpaid");
        assert_eq!(detail.attempts.len(), 2);
        assert_eq!(
            detail
                .attempts
                .iter()
                .find(|attempt| attempt.id == fixture.attempt_id)
                .unwrap()
                .state
                .as_str(),
            "failed"
        );
        assert!(detail.attempts.iter().any(|attempt| {
            attempt.idempotency_key == "concurrent-new-attempt"
                && attempt.state.as_str() == "created"
        }));
    }
}

#[tokio::test]
async fn payment_query_rejects_mismatched_or_unknown_contract_before_provider_call() {
    for attempt_contract_version in [1, 3] {
        let fixture = operations_fixture("stripe").await;
        fixture
            .db
            .write()
            .await
            .execute(fixture.db.stmt(
                "UPDATE store_payment_attempts SET payment_contract_version = $2 WHERE id = $1",
                vec![fixture.attempt_id.clone().into(), attempt_contract_version.into()],
            ))
            .await
            .unwrap();
        let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
        let operations = PaymentQueryOperations::new(
            fixture.db,
            Arc::new(fixture.key_ring),
            Arc::new(provider.clone()),
        );
        assert_eq!(
            operations
                .query_attempt(&fixture.attempt_id)
                .await
                .unwrap_err(),
            PaymentOperationsError::PaymentContractInvalid
        );
        assert!(provider.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn admin_query_maps_configuration_and_transport_failures_without_state_changes() {
    for (adapter_error, expected) in [
        (
            AdapterError::InvalidConfiguration,
            AdminOrderOperationError::ConfigurationUnavailable,
        ),
        (
            AdapterError::InvalidRequest,
            AdminOrderOperationError::ConfigurationUnavailable,
        ),
        (
            AdapterError::Ambiguous,
            AdminOrderOperationError::ProviderQueryFailed,
        ),
    ] {
        let fixture = operations_fixture("stripe").await;
        let provider = RecordingQueryProvider::failing(adapter_error);
        let operations = AdminOrderOperations::new(
            fixture.db.clone(),
            Arc::new(fixture.key_ring),
            Arc::new(provider),
        );
        assert_eq!(
            operations
                .query(&fixture.order_id, &fixture.attempt_id)
                .await
                .unwrap_err(),
            expected
        );
        let row = fixture
            .db
            .read()
            .query_one(fixture.db.stmt(
                "SELECT o.payment_state, a.state AS attempt_state
                 FROM store_orders o JOIN store_payment_attempts a ON a.order_id = o.id
                 WHERE o.id = $1 AND a.id = $2",
                vec![fixture.order_id.into(), fixture.attempt_id.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "payment_state").unwrap(), "unpaid");
        assert_eq!(row.try_get::<String>("", "attempt_state").unwrap(), "created");
    }
}

#[tokio::test]
async fn query_attempt_rejects_decrypted_account_identity_mismatch() {
    let fixture = operations_fixture("stripe").await;
    let wrong_digest = Sha256::digest(b"acct_different")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    {
        let write = fixture.db.write().await;
        write
            .execute(fixture.db.stmt(
                "UPDATE store_channel_credentials SET account_identity_digest = $2 WHERE id = $1",
                vec![
                    fixture.credential_id.clone().into(),
                    wrong_digest.clone().into(),
                ],
            ))
            .await
            .unwrap();
        write
            .execute(fixture.db.stmt(
                "UPDATE store_payment_attempts SET merchant_account_identity = $2 WHERE id = $1",
                vec![fixture.attempt_id.clone().into(), wrong_digest.into()],
            ))
            .await
            .unwrap();
    }
    let provider = RecordingQueryProvider::returning(ProviderPaymentState::Unpaid);
    let operations = PaymentQueryOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    assert_eq!(
        operations
            .query_attempt(&fixture.attempt_id)
            .await
            .unwrap_err(),
        PaymentOperationsError::AccountIdentityMismatch
    );
    assert!(provider.calls.lock().unwrap().is_empty());
}
