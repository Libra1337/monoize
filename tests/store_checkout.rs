use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::adapters::stripe::{StripeCheckoutResult, StripeCredential};
use monoize::store_billing::checkout::{CheckoutError, CheckoutProvider, CheckoutService};
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderError, PaymentOrderStore,
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
struct AmbiguousProvider {
    calls: Arc<AtomicUsize>,
}

#[derive(Clone, Default)]
struct RejectedProvider {
    calls: Arc<AtomicUsize>,
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
impl CheckoutProvider for AmbiguousProvider {
    async fn create_stripe_checkout(
        &self,
        _credential: &StripeCredential,
        _request: &CheckoutRequest,
    ) -> Result<StripeCheckoutResult, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(AdapterError::Ambiguous)
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
                account_digest.into(),
                "2026-08-27T00:00:00Z".into(),
            ],
        ))
        .await
        .unwrap();
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
async fn ambiguous_created_attempt_is_not_sent_to_provider_twice() {
    let (db, key_ring, order_id) = checkout_fixture().await;
    let provider = AmbiguousProvider::default();
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
    assert_eq!(
        service
            .create_attempt("checkout-user", &order_id, input)
            .await
            .unwrap_err(),
        CheckoutError::ProviderAmbiguous
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
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
        CheckoutError::Order(PaymentOrderError::ProviderQueryRequired)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
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
