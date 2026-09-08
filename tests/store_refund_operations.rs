use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::payment::{AdapterError, ProviderRefundState, RefundRequest};
use monoize::store_billing::recovery::{BeginRefundInput, RecoveryStore};
use monoize::store_billing::refund_operations::{
    RefundCredential, RefundOperations, RefundOperationsError, RefundProvider,
    RefundProviderContract, RefundProviderOutcome,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use sha2::{Digest, Sha256};

const ORDER_ID: &str = "refund-operations-order";
const ATTEMPT_ID: &str = "refund-operations-attempt";
const CREDENTIAL_ID: &str = "refund-operations-credential";
const CHANNEL_ID: &str = "store-channel-stripe";
const ACCOUNT_ID: &str = "acct_refund_operations";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedCall {
    operation: &'static str,
    observed_order_state: String,
    channel_id: String,
    credential_version_id: String,
    merchant_account_identity: String,
    account_id: String,
    request: RefundRequest,
}

#[derive(Clone)]
struct RecordingRefundProvider {
    db: DbPool,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    create_outcome: Result<RefundProviderOutcome, AdapterError>,
    query_outcome: Result<RefundProviderOutcome, AdapterError>,
}

impl RecordingRefundProvider {
    fn new(
        db: DbPool,
        create_outcome: Result<RefundProviderOutcome, AdapterError>,
        query_outcome: Result<RefundProviderOutcome, AdapterError>,
    ) -> Self {
        Self {
            db,
            calls: Arc::new(Mutex::new(Vec::new())),
            create_outcome,
            query_outcome,
        }
    }

    async fn record(&self, operation: &'static str, contract: &RefundProviderContract) {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT payment_state FROM store_orders WHERE id = $1",
                vec![ORDER_ID.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        let account_id = match &contract.credential {
            RefundCredential::Stripe(credential) => credential.account_id().to_string(),
            _ => panic!("expected Stripe credential"),
        };
        self.calls.lock().unwrap().push(RecordedCall {
            operation,
            observed_order_state: row.try_get("", "payment_state").unwrap(),
            channel_id: contract.channel_id.clone(),
            credential_version_id: contract.credential_version_id.clone(),
            merchant_account_identity: contract.merchant_account_identity.clone(),
            account_id,
            request: contract.request.clone(),
        });
    }
}

#[async_trait]
impl RefundProvider for RecordingRefundProvider {
    async fn create_refund(
        &self,
        contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        self.record("create", contract).await;
        self.create_outcome.clone()
    }

    async fn query_refund(
        &self,
        contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        self.record("query", contract).await;
        self.query_outcome.clone()
    }
}

struct Fixture {
    db: DbPool,
    key_ring: PaymentKeyRing,
    account_digest: String,
}

async fn fixture() -> Fixture {
    let db = DbPool::connect("sqlite::memory:").await.unwrap();
    {
        let write = db.write().await;
        Migrator::up(&*write, None).await.unwrap();
    }
    let key_ring = PaymentKeyRing::new(
        PaymentKey::new("refund-operations-key", [37_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap();
    let credential_json = br#"{
        "secret_key":"sk_test_refund_operations",
        "publishable_key":"pk_test_refund_operations",
        "webhook_signing_secret":"whsec_refund_operations",
        "api_version":"2026-08-01",
        "account_id":"acct_refund_operations",
        "live_mode":false
    }"#;
    let encrypted = key_ring
        .encrypt(
            &format!("store_channel_credentials:{CREDENTIAL_ID}:secret"),
            credential_json,
        )
        .unwrap();
    let account_digest = Sha256::digest(ACCOUNT_ID.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let write = db.write().await;
    write
        .execute_unprepared(
            "INSERT INTO store_products
                (id, kind, name, description, price_currency, price_minor,
                 duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
             VALUES ('refund-operations-product', 'balance', 'Recharge', '', 'CNY', '1000',
                     NULL, '[]', 0, 1, '2026-08-28T00:00:00Z', '2026-08-28T00:00:00Z')",
        )
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ($1, $2, 'stripe', $3, $4, $5, $6, $7, 'retired', $8)",
            vec![
                CREDENTIAL_ID.into(),
                CHANNEL_ID.into(),
                i32::from(encrypted.version).into(),
                encrypted.key_id.into(),
                encrypted.nonce_base64.into(),
                encrypted.ciphertext_base64.into(),
                account_digest.clone().into(),
                "2026-08-28T00:00:00Z".into(),
            ],
        ))
        .await
        .unwrap();
    write
        .execute_unprepared(
            "INSERT INTO store_orders
                (id, order_number, user_id, product_id, product_kind, payment_state,
                 fulfillment_state, dispute_state, payment_hold, payment_channel_id,
                 payment_currency, payment_minor, cny_per_usd, rate_numerator,
                 rate_denominator, rate_source_updated_at, quote_json, contract_version,
                 state_revision, creation_idempotency_key, creation_request_digest,
                 expires_at, created_at, updated_at, paid_at)
             VALUES ('refund-operations-order', 'LS-REFUND-OPERATIONS', 'refund-user',
                     'refund-operations-product', 'balance', 'paid', 'pending', 'none', 0,
                     'store-channel-stripe', 'CNY', '1000', '6.7000', '67', '10',
                     '2026-08-28T00:00:00Z', '{}', 2, 1, 'refund-order-key',
                     'refund-order-digest', '2026-08-28T01:00:00Z',
                     '2026-08-28T00:00:00Z', '2026-08-28T00:00:00Z',
                     '2026-08-28T00:00:00Z')",
        )
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "INSERT INTO store_payment_attempts
                (id, order_id, channel_id, adapter_kind, credential_version_id,
                 merchant_account_identity, expected_payment_method,
                 payment_contract_version, state, provider_transaction_id,
                 provider_object_id, idempotency_key, paid_at, created_at, updated_at)
             VALUES ($1, $2, $3, 'stripe', $4, $5, 'card', 2, 'paid',
                     'pi_refund_operations', 'cs_refund_operations',
                     'refund-attempt-key', $6, $6, $6)",
            vec![
                ATTEMPT_ID.into(),
                ORDER_ID.into(),
                CHANNEL_ID.into(),
                CREDENTIAL_ID.into(),
                account_digest.clone().into(),
                "2026-08-28T00:00:00Z".into(),
            ],
        ))
        .await
        .unwrap();
    drop(write);
    Fixture {
        db,
        key_ring,
        account_digest,
    }
}

fn outcome(state: ProviderRefundState, provider_refund_id: Option<&str>) -> RefundProviderOutcome {
    RefundProviderOutcome {
        state,
        provider_refund_id: provider_refund_id.map(str::to_string),
        not_found_is_definitive: false,
    }
}

#[tokio::test]
async fn begin_commits_refund_pending_before_provider_create_and_uses_historical_contract() {
    let fixture = fixture().await;
    let provider = RecordingRefundProvider::new(
        fixture.db.clone(),
        Ok(outcome(ProviderRefundState::Pending, Some("re_provider_1"))),
        Ok(outcome(ProviderRefundState::Pending, Some("re_provider_1"))),
    );
    let operations = RefundOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    let refund = operations
        .begin(ORDER_ID, "refund-admin", "refund-admin-key")
        .await
        .unwrap();

    assert_eq!(refund.state, "pending");
    assert_eq!(refund.provider_refund_id.as_deref(), Some("re_provider_1"));
    assert_eq!(
        provider.calls.lock().unwrap().as_slice(),
        &[RecordedCall {
            operation: "create",
            observed_order_state: "refund_pending".to_string(),
            channel_id: CHANNEL_ID.to_string(),
            credential_version_id: CREDENTIAL_ID.to_string(),
            merchant_account_identity: fixture.account_digest,
            account_id: ACCOUNT_ID.to_string(),
            request: RefundRequest {
                provider_transaction_id: "pi_refund_operations".to_string(),
                merchant_order_number: "LS-REFUND-OPERATIONS".to_string(),
                amount_minor: "1000".to_string(),
                currency: monoize::store_billing::Currency::CNY,
                idempotency_key: refund.id,
            },
        }]
    );
}

#[tokio::test]
async fn replay_queries_existing_refund_without_second_provider_create() {
    let fixture = fixture().await;
    let provider = RecordingRefundProvider::new(
        fixture.db.clone(),
        Ok(outcome(ProviderRefundState::Pending, Some("re_provider_2"))),
        Ok(outcome(ProviderRefundState::Pending, Some("re_provider_2"))),
    );
    let operations = RefundOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    let first = operations
        .begin(ORDER_ID, "refund-admin", "refund-replay-key")
        .await
        .unwrap();
    let replay = operations
        .begin(ORDER_ID, "refund-admin", "refund-replay-key")
        .await
        .unwrap();

    assert_eq!(replay, first);
    assert_eq!(
        provider
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|call| call.operation)
            .collect::<Vec<_>>(),
        ["create", "query"]
    );
}

#[tokio::test]
async fn replay_does_not_query_while_the_initial_provider_create_can_still_be_running() {
    let fixture = fixture().await;
    let local = RecoveryStore::new(fixture.db.clone())
        .begin_refund(BeginRefundInput {
            order_id: ORDER_ID.to_string(),
            requested_by_admin_id: "refund-admin".to_string(),
            idempotency_key: "refund-create-lease-key".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(local.state, "created");
    let provider = RecordingRefundProvider::new(
        fixture.db.clone(),
        Ok(outcome(ProviderRefundState::Ambiguous, None)),
        Ok(RefundProviderOutcome {
            state: ProviderRefundState::NotFound,
            provider_refund_id: None,
            not_found_is_definitive: true,
        }),
    );
    let operations = RefundOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    let replay = operations
        .begin(ORDER_ID, "refund-admin", "refund-create-lease-key")
        .await
        .unwrap();

    assert_eq!(replay, local);
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn provider_terminal_outcomes_complete_or_release_the_refund() {
    for (state, expected_refund, expected_order) in [
        (ProviderRefundState::Succeeded, "succeeded", "refunded"),
        (ProviderRefundState::Failed, "failed", "paid"),
    ] {
        let fixture = fixture().await;
        let provider = RecordingRefundProvider::new(
            fixture.db.clone(),
            Ok(outcome(state, Some("re_terminal"))),
            Ok(outcome(ProviderRefundState::Ambiguous, None)),
        );
        let operations = RefundOperations::new(
            fixture.db.clone(),
            Arc::new(fixture.key_ring),
            Arc::new(provider),
        );

        let refund = operations
            .begin(ORDER_ID, "refund-admin", "refund-terminal-key")
            .await
            .unwrap();
        let order = fixture
            .db
            .read()
            .query_one(fixture.db.stmt(
                "SELECT payment_state FROM store_orders WHERE id = $1",
                vec![ORDER_ID.into()],
            ))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(refund.state, expected_refund);
        assert_eq!(
            order.try_get::<String>("", "payment_state").unwrap(),
            expected_order
        );
    }
}

#[tokio::test]
async fn ambiguous_provider_error_keeps_the_refund_pending_for_query_recovery() {
    let fixture = fixture().await;
    let provider = RecordingRefundProvider::new(
        fixture.db.clone(),
        Err(AdapterError::Ambiguous),
        Ok(outcome(ProviderRefundState::Pending, None)),
    );
    let operations = RefundOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    let refund = operations
        .begin(ORDER_ID, "refund-admin", "refund-ambiguous-key")
        .await
        .unwrap();

    assert_eq!(refund.state, "pending");
    assert_eq!(refund.provider_refund_id, None);
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn query_binds_the_path_order_and_persists_a_late_provider_refund_id() {
    let fixture = fixture().await;
    let key_ring = Arc::new(fixture.key_ring);
    let create_provider = RecordingRefundProvider::new(
        fixture.db.clone(),
        Err(AdapterError::Ambiguous),
        Ok(outcome(ProviderRefundState::Pending, None)),
    );
    let create_operations = RefundOperations::new(
        fixture.db.clone(),
        key_ring.clone(),
        Arc::new(create_provider),
    );
    let refund = create_operations
        .begin(ORDER_ID, "refund-admin", "refund-query-key")
        .await
        .unwrap();
    assert_eq!(refund.provider_refund_id, None);

    let query_provider = RecordingRefundProvider::new(
        fixture.db.clone(),
        Ok(outcome(ProviderRefundState::Ambiguous, None)),
        Ok(outcome(
            ProviderRefundState::Pending,
            Some("re_discovered_later"),
        )),
    );
    let query_operations =
        RefundOperations::new(fixture.db, key_ring, Arc::new(query_provider.clone()));

    assert_eq!(
        query_operations
            .query("another-order", &refund.id)
            .await
            .unwrap_err(),
        RefundOperationsError::NotFound
    );
    let queried = query_operations.query(ORDER_ID, &refund.id).await.unwrap();

    assert_eq!(
        queried.provider_refund_id.as_deref(),
        Some("re_discovered_later")
    );
    assert_eq!(
        query_provider
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|call| call.operation)
            .collect::<Vec<_>>(),
        ["query"]
    );
}

#[tokio::test]
async fn rejected_query_does_not_prove_that_the_refund_failed() {
    let fixture = fixture().await;
    let key_ring = Arc::new(fixture.key_ring);
    let create_operations = RefundOperations::new(
        fixture.db.clone(),
        key_ring.clone(),
        Arc::new(RecordingRefundProvider::new(
            fixture.db.clone(),
            Err(AdapterError::Ambiguous),
            Ok(outcome(ProviderRefundState::Pending, None)),
        )),
    );
    let refund = create_operations
        .begin(ORDER_ID, "refund-admin", "refund-query-rejected-key")
        .await
        .unwrap();
    let query_operations = RefundOperations::new(
        fixture.db.clone(),
        key_ring,
        Arc::new(RecordingRefundProvider::new(
            fixture.db,
            Ok(outcome(ProviderRefundState::Ambiguous, None)),
            Err(AdapterError::Rejected),
        )),
    );

    let queried = query_operations.query(ORDER_ID, &refund.id).await.unwrap();

    assert_eq!(queried.state, "pending");
}

#[tokio::test]
async fn historical_merchant_binding_mismatch_fails_without_provider_call() {
    let fixture = fixture().await;
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "UPDATE store_payment_attempts SET merchant_account_identity = $2 WHERE id = $1",
            vec![ATTEMPT_ID.into(), "b".repeat(64).into()],
        ))
        .await
        .unwrap();
    let provider = RecordingRefundProvider::new(
        fixture.db.clone(),
        Ok(outcome(ProviderRefundState::Pending, None)),
        Ok(outcome(ProviderRefundState::Pending, None)),
    );
    let operations = RefundOperations::new(
        fixture.db,
        Arc::new(fixture.key_ring),
        Arc::new(provider.clone()),
    );

    assert_eq!(
        operations
            .begin(ORDER_ID, "refund-admin", "refund-binding-key")
            .await
            .unwrap_err(),
        RefundOperationsError::ConfigurationUnavailable
    );
    assert!(provider.calls.lock().unwrap().is_empty());
}
