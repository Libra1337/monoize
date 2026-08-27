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
use monoize::store_billing::operations::{
    PaymentOperationsError, PaymentQueryOperations, PaymentQueryProvider,
};
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderStore,
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
    state: ProviderPaymentState,
}

impl RecordingQueryProvider {
    fn returning(state: ProviderPaymentState) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            state,
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

#[async_trait]
impl PaymentQueryProvider for RecordingQueryProvider {
    async fn query_stripe_payment(
        &self,
        credential: &StripeCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.record("stripe", credential.account_id(), vec![], query);
        Ok(self.state.clone())
    }

    async fn query_alipay_payment(
        &self,
        credential: &AlipayCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        self.record("alipay", credential.seller_id(), vec![], query);
        Ok(self.state.clone())
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
        Ok(self.state.clone())
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
