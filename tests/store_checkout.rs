use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::adapters::alipay::{
    AlipayCheckoutResult, AlipayCredential, AlipayProduct,
};
use monoize::store_billing::adapters::stripe::{StripeCheckoutResult, StripeCredential};
use monoize::store_billing::adapters::wechat::{
    WechatCheckoutResult, WechatCredential, WechatProduct,
};
use monoize::store_billing::checkout::{CheckoutError, CheckoutProvider, CheckoutService};
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderStore,
};
use monoize::store_billing::payment::{AdapterError, CheckoutAction, CheckoutRequest};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Clone)]
struct RecordingProvider {
    requests: Arc<Mutex<Vec<CheckoutRequest>>>,
}

#[derive(Clone, Default)]
struct RecoveringStripeProvider {
    calls: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<CheckoutRequest>>>,
}

#[derive(Clone, Default)]
struct RejectedProvider {
    calls: Arc<AtomicUsize>,
}

#[derive(Clone, Default)]
struct OfficialChannelProvider {
    alipay_calls: Arc<AtomicUsize>,
    wechat_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl CheckoutProvider for OfficialChannelProvider {
    async fn create_stripe_checkout(
        &self,
        _credential: &StripeCredential,
        _request: &CheckoutRequest,
    ) -> Result<StripeCheckoutResult, AdapterError> {
        Err(AdapterError::Unsupported)
    }

    async fn create_alipay_checkout(
        &self,
        _credential: &AlipayCredential,
        request: &CheckoutRequest,
        product: AlipayProduct,
        _notify_url: Url,
    ) -> Result<AlipayCheckoutResult, AdapterError> {
        self.alipay_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(product, AlipayProduct::ComputerWeb);
        Ok(AlipayCheckoutResult {
            provider_object_id: request.order_number.clone(),
            action: CheckoutAction::Form {
                action: "https://openapi.alipay.com/gateway.do".to_string(),
                fields: vec![("sign".to_string(), "signed".to_string())],
                expires_at: "2026-08-28T01:00:00Z".to_string(),
            },
        })
    }

    async fn create_wechat_checkout(
        &self,
        _credential: &WechatCredential,
        request: &CheckoutRequest,
        product: WechatProduct,
        _notify_url: Url,
        client_ip: Option<std::net::IpAddr>,
    ) -> Result<WechatCheckoutResult, AdapterError> {
        self.wechat_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(product, WechatProduct::H5);
        assert_eq!(client_ip, Some("203.0.113.9".parse().unwrap()));
        Ok(WechatCheckoutResult {
            provider_object_id: request.order_number.clone(),
            action: CheckoutAction::Redirect {
                url: "https://wx.tenpay.com/cgi-bin/mmpayweb-bin/checkmweb?prepay_id=test"
                    .to_string(),
                expires_at: "2026-08-28T01:00:00Z".to_string(),
            },
        })
    }
}

#[async_trait]
impl CheckoutProvider for RejectedProvider {
    async fn create_stripe_checkout(
        &self,
        _credential: &StripeCredential,
        _request: &CheckoutRequest,
    ) -> Result<StripeCheckoutResult, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(AdapterError::Rejected)
    }
}

#[async_trait]
impl CheckoutProvider for RecoveringStripeProvider {
    async fn create_stripe_checkout(
        &self,
        _credential: &StripeCredential,
        request: &CheckoutRequest,
    ) -> Result<StripeCheckoutResult, AdapterError> {
        self.requests.lock().unwrap().push(request.clone());
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(AdapterError::Ambiguous);
        }
        Ok(StripeCheckoutResult {
            provider_object_id: "cs_recovered".to_string(),
            action: CheckoutAction::Redirect {
                url: "https://checkout.stripe.com/c/pay_recovered".to_string(),
                expires_at: "2026-08-27T01:00:00Z".to_string(),
            },
        })
    }
}

#[async_trait]
impl CheckoutProvider for RecordingProvider {
    async fn create_stripe_checkout(
        &self,
        credential: &StripeCredential,
        request: &CheckoutRequest,
    ) -> Result<StripeCheckoutResult, AdapterError> {
        assert_eq!(credential.account_id(), "acct_checkout");
        self.requests.lock().unwrap().push(request.clone());
        Ok(StripeCheckoutResult {
            provider_object_id: "cs_test_checkout".to_string(),
            action: CheckoutAction::Redirect {
                url: "https://checkout.stripe.com/c/pay_checkout".to_string(),
                expires_at: "2026-08-27T01:00:00Z".to_string(),
            },
        })
    }
}

async fn checkout_fixture() -> (DbPool, PaymentKeyRing, String) {
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
                    ('checkout-product', 'balance', 'Recharge', '', 'CNY', '1000',
                     NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
                 VALUES ('checkout-product', '1000', '0')",
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

    let key_ring = PaymentKeyRing::new(
        PaymentKey::new("checkout-key", [17_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let credential_json = br#"{
        "secret_key":"sk_test_checkout",
        "publishable_key":"pk_test_checkout",
        "webhook_signing_secret":"whsec_checkout",
        "api_version":"2026-08-01",
        "account_id":"acct_checkout",
        "live_mode":false
    }"#;
    let encrypted = key_ring
        .encrypt(
            "store_channel_credentials:checkout-credential:secret",
            credential_json,
        )
        .unwrap();
    let account_digest = Sha256::digest(b"acct_checkout")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9)",
            vec![
                "checkout-credential".into(),
                "store-channel-stripe".into(),
                "stripe".into(),
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
    seed_checkout_governance(&db, &account_digest).await;
    let order = PaymentOrderStore::new(db.clone())
        .create_order(
            "checkout-user",
            CreatePaymentOrderInput {
                idempotency_key: "checkout-order-key".to_string(),
                product_id: "checkout-product".to_string(),
                payment_channel_id: "store-channel-stripe".to_string(),
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
    (db, key_ring, order.id)
}

async fn seed_checkout_governance(db: &DbPool, account_digest: &str) {
    let write = db.write().await;
    write
        .execute_unprepared(
            "INSERT INTO store_payment_compliance
                (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
             VALUES ('checkout-compliance', 'store-channel-stripe', '2026-08-28',
                     'checkout-admin', '127.0.0.1', '2026-08-27T00:00:00Z');
             INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('checkout-privacy', 'v1', 'CN', '[]', '{}', 'contract', 'checkout-admin',
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
                         'checkout',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                         'checkout-admin', '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z')",
                vec![
                    format!("checkout-cap-{capability}").into(),
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
             VALUES ('store-channel-stripe', $1, 'checkout-privacy', 1, '[\"CNY\",\"USD\"]',
                     '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                     '[\"redirect\"]',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'checkout-admin', '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z')",
            vec![account_digest.into()],
        ))
        .await
        .unwrap();
}

async fn replace_checkout_adapter(
    db: &DbPool,
    key_ring: &PaymentKeyRing,
    adapter_kind: &str,
    account_id: &str,
    credential_json: &[u8],
) {
    let encrypted = key_ring
        .encrypt(
            "store_channel_credentials:checkout-credential:secret",
            credential_json,
        )
        .unwrap();
    let digest = if adapter_kind == "wechat" {
        WechatCredential::from_json(credential_json)
            .unwrap()
            .account_identity_digest()
    } else {
        Sha256::digest(account_id.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let write = db.write().await;
    write
        .execute(db.stmt(
            "UPDATE store_payment_channels SET adapter_kind = $2 WHERE id = $1",
            vec!["store-channel-stripe".into(), adapter_kind.into()],
        ))
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "UPDATE store_channel_credentials
             SET adapter_kind = $2, format_version = $3, key_id = $4,
                 nonce_base64 = $5, ciphertext_base64 = $6,
                 account_identity_digest = $7
             WHERE id = 'checkout-credential'",
            vec![
                "checkout-credential".into(),
                adapter_kind.into(),
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                digest.clone().into(),
            ],
        ))
        .await
        .unwrap();
    let (currencies, limits, actions) = match adapter_kind {
        "alipay" => (
            "[\"CNY\"]",
            "{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}",
            "[\"form\"]",
        ),
        "wechat" => (
            "[\"CNY\"]",
            "{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}",
            "[\"qr\",\"redirect\"]",
        ),
        _ => (
            "[\"CNY\",\"USD\"]",
            "{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}",
            "[\"redirect\"]",
        ),
    };
    write
        .execute(db.stmt(
            "UPDATE store_merchant_capabilities SET merchant_account_digest = $2
             WHERE channel_id = $1",
            vec!["store-channel-stripe".into(), digest.clone().into()],
        ))
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "UPDATE store_channel_readiness_profiles
             SET active_credential_digest = $2, supported_currencies_json = $3,
                 amount_limits_json = $4, checkout_action_kinds_json = $5
             WHERE channel_id = $1",
            vec![
                "store-channel-stripe".into(),
                digest.into(),
                currencies.into(),
                limits.into(),
                actions.into(),
            ],
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn checkout_dispatches_alipay_and_wechat_credentials() {
    let (db, key_ring, order_id) = checkout_fixture().await;
    let provider = OfficialChannelProvider::default();
    replace_checkout_adapter(
        &db,
        &key_ring,
        "alipay",
        "2088000000000001",
        br#"{
            "app_id":"2026000000000001","seller_id":"2088000000000001",
            "merchant_private_key_pem":"private","alipay_public_key_pem":"public",
            "environment":"production"
        }"#,
    )
    .await;
    let alipay = CheckoutService::new(
        db.clone(),
        Some(Arc::new(key_ring)),
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    )
    .create_attempt(
        "checkout-user",
        &order_id,
        CreatePaymentAttemptInput {
            idempotency_key: "checkout-alipay".to_string(),
            expected_payment_method: Some("computer_web".to_string()),
        },
    )
    .await
    .unwrap();
    assert!(matches!(alipay.action, CheckoutAction::Form { .. }));
    assert_eq!(provider.alipay_calls.load(Ordering::SeqCst), 1);

    let (db, key_ring, order_id) = checkout_fixture().await;
    replace_checkout_adapter(
        &db,
        &key_ring,
        "wechat",
        "1900000109",
        br#"{
            "merchant_id":"1900000109","app_id":"wx1234567890",
            "api_v3_key":"0123456789abcdef0123456789abcdef",
            "merchant_certificate_serial":"7777777777777777777777777777777777777777",
            "merchant_private_key_pem":"private",
            "platform_certificate_serial":"PLATFORM-CERTIFICATE-1",
            "platform_public_key_pem":"public"
        }"#,
    )
    .await;
    let wechat = CheckoutService::new(
        db,
        Some(Arc::new(key_ring)),
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    )
    .with_client_ip(Some("203.0.113.9".parse().unwrap()))
    .create_attempt(
        "checkout-user",
        &order_id,
        CreatePaymentAttemptInput {
            idempotency_key: "checkout-wechat".to_string(),
            expected_payment_method: Some("h5".to_string()),
        },
    )
    .await
    .unwrap();
    assert!(matches!(wechat.action, CheckoutAction::Redirect { .. }));
    assert_eq!(provider.wechat_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stripe_checkout_decrypts_bound_credential_and_persists_action() {
    let (db, key_ring, order_id) = checkout_fixture().await;
    let provider = RecordingProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let service = CheckoutService::new(
        db,
        Some(Arc::new(key_ring)),
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    );

    let result = service
        .create_attempt(
            "checkout-user",
            &order_id,
            CreatePaymentAttemptInput {
                idempotency_key: "checkout-attempt-key".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();

    assert!(!result.replayed);
    assert_eq!(result.attempt.state.as_str(), "presented");
    assert_eq!(result.attempt.action.as_ref(), Some(&result.action));
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].amount_minor, "1000");
    assert_eq!(requests[0].currency, Currency::CNY);
    assert_eq!(
        requests[0].success_url.as_str(),
        format!("https://lynshen.org/dashboard/store?order_id={order_id}&checkout=success")
    );
    assert_eq!(
        requests[0].cancel_url.as_str(),
        format!("https://lynshen.org/dashboard/store?order_id={order_id}&checkout=cancel")
    );
}

#[tokio::test]
async fn presented_attempt_replays_persisted_action_without_provider_call() {
    let (db, key_ring, order_id) = checkout_fixture().await;
    let provider = RecordingProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let service = CheckoutService::new(
        db,
        Some(Arc::new(key_ring)),
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    );
    let input = CreatePaymentAttemptInput {
        idempotency_key: "checkout-replay-key".to_string(),
        expected_payment_method: Some("card".to_string()),
    };

    let first = service
        .create_attempt("checkout-user", &order_id, input.clone())
        .await
        .unwrap();
    let replay = service
        .create_attempt("checkout-user", &order_id, input)
        .await
        .unwrap();

    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(replay.attempt, first.attempt);
    assert_eq!(replay.action, first.action);
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn missing_payment_keys_fails_after_persisting_the_attempt() {
    let (db, _key_ring, order_id) = checkout_fixture().await;
    let provider = RecordingProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let service = CheckoutService::new(
        db.clone(),
        None,
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    );

    let error = service
        .create_attempt(
            "checkout-user",
            &order_id,
            CreatePaymentAttemptInput {
                idempotency_key: "checkout-missing-keys".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap_err();

    assert_eq!(error, CheckoutError::ConfigurationUnavailable);
    assert!(provider.requests.lock().unwrap().is_empty());
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT state FROM store_payment_attempts WHERE idempotency_key = $1",
            vec!["checkout-missing-keys".into()],
        ))
        .await
        .unwrap()
        .expect("attempt was committed before configuration validation");
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "failed");
}

#[tokio::test]
async fn ambiguous_stripe_attempt_replays_the_same_provider_mutation() {
    let (db, key_ring, order_id) = checkout_fixture().await;
    let provider = RecoveringStripeProvider::default();
    let service = CheckoutService::new(
        db,
        Some(Arc::new(key_ring)),
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    );
    let input = CreatePaymentAttemptInput {
        idempotency_key: "checkout-ambiguous".to_string(),
        expected_payment_method: Some("card".to_string()),
    };

    assert_eq!(
        service
            .create_attempt("checkout-user", &order_id, input.clone())
            .await
            .unwrap_err(),
        CheckoutError::ProviderAmbiguous
    );
    let recovered = service
        .create_attempt("checkout-user", &order_id, input)
        .await
        .unwrap();
    assert!(recovered.replayed);
    assert_eq!(recovered.attempt.state.as_str(), "presented");
    assert_eq!(
        recovered.attempt.provider_object_id.as_deref(),
        Some("cs_recovered")
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
}

#[tokio::test]
async fn stripe_ambiguous_replay_keeps_attempt_blocking_when_configuration_is_unavailable() {
    let (db, key_ring, order_id) = checkout_fixture().await;
    let key_ring = Arc::new(key_ring);
    let provider = RecoveringStripeProvider::default();
    let configured = CheckoutService::new(
        db.clone(),
        Some(key_ring.clone()),
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    );
    let input = CreatePaymentAttemptInput {
        idempotency_key: "checkout-ambiguous-config".to_string(),
        expected_payment_method: Some("card".to_string()),
    };

    assert_eq!(
        configured
            .create_attempt("checkout-user", &order_id, input.clone())
            .await
            .unwrap_err(),
        CheckoutError::ProviderAmbiguous
    );
    let unavailable = CheckoutService::new(
        db.clone(),
        None,
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    );
    assert_eq!(
        unavailable
            .create_attempt("checkout-user", &order_id, input.clone())
            .await
            .unwrap_err(),
        CheckoutError::ConfigurationUnavailable
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT state, failure_kind FROM store_payment_attempts
             WHERE idempotency_key = $1",
            vec![input.idempotency_key.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "created");
    assert_eq!(
        row.try_get::<Option<String>>("", "failure_kind").unwrap(),
        None
    );
    assert_eq!(
        unavailable
            .create_attempt(
                "checkout-user",
                &order_id,
                CreatePaymentAttemptInput {
                    idempotency_key: "checkout-ambiguous-config-new-key".to_string(),
                    expected_payment_method: Some("card".to_string()),
                },
            )
            .await
            .unwrap_err(),
        CheckoutError::Order(monoize::store_billing::order::PaymentOrderError::ActiveAttemptExists)
    );

    let recovered = configured
        .create_attempt("checkout-user", &order_id, input)
        .await
        .unwrap();
    assert_eq!(recovered.attempt.state.as_str(), "presented");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let attempt_count: i64 = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM store_payment_attempts WHERE order_id = $1",
            vec![order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();
    assert_eq!(attempt_count, 1);
}

#[tokio::test]
async fn definite_provider_rejection_marks_attempt_failed() {
    let (db, key_ring, order_id) = checkout_fixture().await;
    let provider = RejectedProvider::default();
    let service = CheckoutService::new(
        db.clone(),
        Some(Arc::new(key_ring)),
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    );
    let input = CreatePaymentAttemptInput {
        idempotency_key: "checkout-rejected".to_string(),
        expected_payment_method: Some("card".to_string()),
    };

    assert_eq!(
        service
            .create_attempt("checkout-user", &order_id, input.clone())
            .await
            .unwrap_err(),
        CheckoutError::ProviderRejected
    );
    assert_eq!(
        service
            .create_attempt("checkout-user", &order_id, input)
            .await
            .unwrap_err(),
        CheckoutError::ProviderRejected
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        service
            .create_attempt(
                "checkout-user",
                &order_id,
                CreatePaymentAttemptInput {
                    idempotency_key: "checkout-rejected-second-key".to_string(),
                    expected_payment_method: Some("card".to_string()),
                },
            )
            .await
            .unwrap_err(),
        CheckoutError::ProviderRejected
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT state FROM store_payment_attempts WHERE idempotency_key = $1",
            vec!["checkout-rejected".into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "failed");
}

#[tokio::test]
async fn paid_attempt_replays_its_persisted_action() {
    let (db, key_ring, order_id) = checkout_fixture().await;
    let provider = RecordingProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let service = CheckoutService::new(
        db.clone(),
        Some(Arc::new(key_ring)),
        Some(Url::parse("https://lynshen.org").unwrap()),
        Arc::new(provider.clone()),
    );
    let input = CreatePaymentAttemptInput {
        idempotency_key: "checkout-paid-replay".to_string(),
        expected_payment_method: Some("card".to_string()),
    };
    let first = service
        .create_attempt("checkout-user", &order_id, input.clone())
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_payment_attempts SET state = 'paid' WHERE id = $1",
            vec![first.attempt.id.clone().into()],
        ))
        .await
        .unwrap();

    let replay = service
        .create_attempt("checkout-user", &order_id, input)
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.action, first.action);
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
}
