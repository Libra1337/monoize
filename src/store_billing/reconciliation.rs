use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use sha2::{Digest, Sha256};
use std::time::Instant;
use uuid::Uuid;

use super::callbacks::{
    ApplyProviderEventInput, CallbackApplyResult, CallbackStoreError, PaymentCallbackStore,
};
use super::operations::{PaymentOperationsError, PaymentQueryOperations, PaymentQueryOutcome};
use super::payment::ProviderPaymentState;
use crate::db::DbPool;

const LEASE_NAME: &str = "store_reconciler";
const LEASE_SECONDS: i64 = 90;
const BATCH_SIZE: i64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationOutcome {
    pub scanned: usize,
    pub fulfilled: usize,
    pub failed: usize,
    pub payment_queries: usize,
    pub payments_applied: usize,
    pub attempts_expired: usize,
    pub query_failures: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReconciliationError {
    #[error("the Store reconciliation lease belongs to another owner")]
    LeaseUnavailable,
    #[error("the Store reconciliation lease was lost")]
    LeaseLost,
    #[error("Store reconciliation storage failed: {0}")]
    Storage(String),
}

#[derive(Clone)]
pub struct StoreReconciler {
    db: DbPool,
    payment_queries: Option<PaymentQueryOperations>,
}

impl StoreReconciler {
    pub fn new(db: DbPool) -> Self {
        Self {
            db,
            payment_queries: None,
        }
    }

    pub fn with_payment_queries(mut self, payment_queries: PaymentQueryOperations) -> Self {
        self.payment_queries = Some(payment_queries);
        self
    }

    pub async fn run_once(
        &self,
        owner_id: &str,
        now: DateTime<Utc>,
    ) -> Result<ReconciliationOutcome, ReconciliationError> {
        validate_owner_id(owner_id)?;
        let started_at = Instant::now();
        let epoch = self.acquire_lease(owner_id, now).await?;
        let payment_candidates = if self.payment_queries.is_some() {
            self.expired_payment_candidates(now).await?
        } else {
            Vec::new()
        };
        let candidates = self.fulfillment_candidates(now).await?;
        let mut outcome = ReconciliationOutcome {
            scanned: payment_candidates.len() + candidates.len(),
            fulfilled: 0,
            failed: 0,
            payment_queries: 0,
            payments_applied: 0,
            attempts_expired: 0,
            query_failures: 0,
        };
        let callbacks = PaymentCallbackStore::new(self.db.clone());
        if let Some(payment_queries) = &self.payment_queries {
            for attempt_id in payment_candidates {
                outcome.payment_queries += 1;
                let query = match payment_queries
                    .query_attempt_with_context(&attempt_id)
                    .await
                {
                    Ok(query) => query,
                    Err(PaymentOperationsError::Storage(error)) => {
                        return Err(ReconciliationError::Storage(error));
                    }
                    Err(error) => {
                        self.upsert_payment_query_case(
                            &attempt_id,
                            None,
                            None,
                            payment_operations_error_category(&error),
                            "high",
                            now,
                        )
                        .await?;
                        outcome.query_failures += 1;
                        continue;
                    }
                };
                let fence_now = reconciliation_now(now, started_at)?;
                match query.state.clone() {
                    ProviderPaymentState::Paid {
                        provider_transaction_id,
                    } => {
                        match callbacks
                            .apply_verified_payment_fenced(
                                payment_query_event(&query, &provider_transaction_id, fence_now)?,
                                owner_id,
                                epoch,
                                fence_now,
                            )
                            .await
                        {
                            Ok(CallbackApplyResult::Applied) => {
                                outcome.payments_applied += 1;
                                self.close_payment_query_case(&attempt_id, fence_now)
                                    .await?;
                                if query.payment_hold {
                                    continue;
                                }
                                let fulfillment_now = reconciliation_now(now, started_at)?;
                                match callbacks
                                    .fulfill_paid_order_fenced(
                                        &query.order_id,
                                        owner_id,
                                        epoch,
                                        fulfillment_now,
                                    )
                                    .await
                                {
                                    Ok(()) => outcome.fulfilled += 1,
                                    Err(CallbackStoreError::Fulfillment(_)) => {
                                        let retry_now = reconciliation_now(now, started_at)?;
                                        self.schedule_fulfillment_retry(
                                            &query.order_id,
                                            owner_id,
                                            epoch,
                                            retry_now,
                                        )
                                        .await?;
                                        outcome.failed += 1;
                                    }
                                    Err(CallbackStoreError::Storage(error))
                                        if error == "reconciliation lease was lost" =>
                                    {
                                        return Err(ReconciliationError::LeaseLost);
                                    }
                                    Err(error) => {
                                        return Err(ReconciliationError::Storage(
                                            error.to_string(),
                                        ));
                                    }
                                }
                            }
                            Ok(CallbackApplyResult::Duplicate) => {
                                self.close_payment_query_case(&attempt_id, fence_now)
                                    .await?;
                            }
                            Ok(CallbackApplyResult::ManualReview) => {
                                self.upsert_payment_query_case(
                                    &attempt_id,
                                    Some(&query.order_id),
                                    Some(&query.channel_id),
                                    "projection_mismatch",
                                    "critical",
                                    fence_now,
                                )
                                .await?;
                                outcome.query_failures += 1;
                            }
                            Err(CallbackStoreError::Storage(error))
                                if error == "reconciliation lease was lost" =>
                            {
                                return Err(ReconciliationError::LeaseLost);
                            }
                            Err(error) => {
                                return Err(ReconciliationError::Storage(error.to_string()));
                            }
                        }
                    }
                    ProviderPaymentState::NotFound
                    | ProviderPaymentState::Unpaid
                    | ProviderPaymentState::Closed => {
                        if query.attempt_state == "presented" {
                            if self
                                .expire_attempt_fenced(&attempt_id, owner_id, epoch, fence_now)
                                .await?
                            {
                                outcome.attempts_expired += 1;
                                self.close_payment_query_case(&attempt_id, fence_now)
                                    .await?;
                            }
                        } else if !matches!(query.state, ProviderPaymentState::Unpaid)
                            && self
                                .release_unpresented_attempt_fenced(
                                    &attempt_id,
                                    owner_id,
                                    epoch,
                                    fence_now,
                                )
                                .await?
                        {
                            outcome.attempts_expired += 1;
                            self.close_payment_query_case(&attempt_id, fence_now)
                                .await?;
                        } else {
                            self.upsert_payment_query_case(
                                &attempt_id,
                                Some(&query.order_id),
                                Some(&query.channel_id),
                                "provider_unpaid",
                                "medium",
                                fence_now,
                            )
                            .await?;
                            outcome.query_failures += 1;
                        }
                    }
                    ProviderPaymentState::Ambiguous => {
                        self.upsert_payment_query_case(
                            &attempt_id,
                            Some(&query.order_id),
                            Some(&query.channel_id),
                            "provider_ambiguous",
                            "high",
                            fence_now,
                        )
                        .await?;
                        outcome.query_failures += 1;
                    }
                }
            }
        }
        for order_id in candidates {
            let fence_now = reconciliation_now(now, started_at)?;
            match callbacks
                .fulfill_paid_order_fenced(&order_id, owner_id, epoch, fence_now)
                .await
            {
                Ok(()) => outcome.fulfilled += 1,
                Err(CallbackStoreError::Fulfillment(_)) => {
                    let retry_now = reconciliation_now(now, started_at)?;
                    self.schedule_fulfillment_retry(&order_id, owner_id, epoch, retry_now)
                        .await?;
                    outcome.failed += 1;
                }
                Err(CallbackStoreError::Storage(error))
                    if error == "reconciliation lease was lost" =>
                {
                    return Err(ReconciliationError::LeaseLost);
                }
                Err(error) => return Err(ReconciliationError::Storage(error.to_string())),
            }
        }
        Ok(outcome)
    }

    async fn expired_payment_candidates(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, ReconciliationError> {
        let recovery_cutoff = now - chrono::Duration::seconds(30);
        let case_retry_cutoff = now - chrono::Duration::seconds(60);
        self.db
            .read()
            .query_all(self.db.stmt(
                "SELECT a.id
                 FROM store_payment_attempts a
                 JOIN store_orders o ON o.id = a.order_id
                 LEFT JOIN store_reconciliation_cases rc
                   ON rc.id = ('payment-query:' || a.id) AND rc.state = 'open'
                 WHERE o.payment_state = 'unpaid'
                   AND (rc.id IS NULL OR rc.updated_at <= $3) AND (
                    (a.state = 'presented' AND a.provider_object_id IS NOT NULL
                     AND a.provider_expires_at IS NOT NULL AND a.provider_expires_at <= $1)
                    OR
                    (a.adapter_kind IN ('alipay', 'wechat') AND a.updated_at <= $2
                     AND (a.state = 'created'
                          OR (a.state = 'failed' AND a.failure_kind = 'provider_rejected')))
                 )
                 ORDER BY COALESCE(a.provider_expires_at, a.updated_at) ASC, a.id ASC
                 LIMIT $4",
                vec![
                    timestamp(now).into(),
                    timestamp(recovery_cutoff).into(),
                    timestamp(case_retry_cutoff).into(),
                    BATCH_SIZE.into(),
                ],
            ))
            .await
            .map_err(storage)?
            .iter()
            .map(|row| row_string(row, "id"))
            .collect()
    }

    async fn release_unpresented_attempt_fenced(
        &self,
        attempt_id: &str,
        owner_id: &str,
        epoch: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, ReconciliationError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        validate_fence(&self.db, &*tx, owner_id, epoch, now, lock).await?;
        let row = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT a.state AS attempt_state, a.failure_kind, o.payment_state
                     FROM store_payment_attempts a
                     JOIN store_orders o ON o.id = a.order_id
                     WHERE a.id = $1{lock}"
                ),
                vec![attempt_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| {
                ReconciliationError::Storage("payment attempt is missing".to_string())
            })?;
        let attempt_state = row_string(&row, "attempt_state")?;
        let failure_kind = row_optional_string(&row, "failure_kind")?;
        if row_string(&row, "payment_state")? != "unpaid"
            || !matches!(
                (attempt_state.as_str(), failure_kind.as_deref()),
                ("created", None) | ("failed", Some("provider_rejected"))
            )
        {
            tx.commit().await.map_err(storage)?;
            return Ok(false);
        }
        let changed = tx
            .execute(self.db.stmt(
                "UPDATE store_payment_attempts
                 SET state = 'expired', failure_kind = NULL, updated_at = $2
                 WHERE id = $1 AND state = $3",
                vec![
                    attempt_id.into(),
                    timestamp(now).into(),
                    attempt_state.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if changed.rows_affected() != 1 {
            return Err(ReconciliationError::Storage(
                "recoverable payment attempt changed concurrently".to_string(),
            ));
        }
        tx.commit().await.map_err(storage)?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    async fn upsert_payment_query_case(
        &self,
        attempt_id: &str,
        order_id: Option<&str>,
        channel_id: Option<&str>,
        category: &str,
        severity: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ReconciliationError> {
        let evidence = serde_json::json!({
            "attempt_id": attempt_id,
            "category": category,
        })
        .to_string();
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "INSERT INTO store_reconciliation_cases
                    (id, order_id, channel_id, severity, kind, state, evidence_json,
                     created_at, updated_at)
                 VALUES ($1, $2, $3, $4, 'payment_query', 'open', $5, $6, $6)
                 ON CONFLICT (id) DO UPDATE SET
                    order_id = $2, channel_id = $3, severity = $4, state = 'open',
                    evidence_json = $5, updated_at = $6, closed_at = NULL",
                vec![
                    format!("payment-query:{attempt_id}").into(),
                    order_id.map(str::to_string).into(),
                    channel_id.map(str::to_string).into(),
                    severity.into(),
                    evidence.into(),
                    timestamp(now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn close_payment_query_case(
        &self,
        attempt_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ReconciliationError> {
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "UPDATE store_reconciliation_cases
                 SET state = 'closed', updated_at = $2, closed_at = $2
                 WHERE id = $1 AND state <> 'closed'",
                vec![
                    format!("payment-query:{attempt_id}").into(),
                    timestamp(now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn expire_attempt_fenced(
        &self,
        attempt_id: &str,
        owner_id: &str,
        epoch: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, ReconciliationError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        validate_fence(&self.db, &*tx, owner_id, epoch, now, lock).await?;
        let row = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT a.order_id, a.state AS attempt_state, a.provider_expires_at,
                            o.payment_state, o.state_revision
                     FROM store_payment_attempts a
                     JOIN store_orders o ON o.id = a.order_id
                     WHERE a.id = $1{lock}"
                ),
                vec![attempt_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| {
                ReconciliationError::Storage("payment attempt is missing".to_string())
            })?;
        if row_string(&row, "attempt_state")? != "presented"
            || row_string(&row, "payment_state")? != "unpaid"
            || row_timestamp(&row, "provider_expires_at")? > now
        {
            tx.commit().await.map_err(storage)?;
            return Ok(false);
        }
        let order_id = row_string(&row, "order_id")?;
        let changed_attempt = tx
            .execute(self.db.stmt(
                "UPDATE store_payment_attempts
                 SET state = 'expired', updated_at = $2
                 WHERE id = $1 AND state = 'presented'",
                vec![attempt_id.into(), timestamp(now).into()],
            ))
            .await
            .map_err(storage)?;
        let changed_order = tx
            .execute(self.db.stmt(
                "UPDATE store_orders
                 SET payment_state = 'closed', closed_at = $2, updated_at = $2,
                     state_revision = state_revision + 1
                 WHERE id = $1 AND payment_state = 'unpaid' AND state_revision = $3",
                vec![
                    order_id.into(),
                    timestamp(now).into(),
                    row_i64(&row, "state_revision")?.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if changed_attempt.rows_affected() != 1 || changed_order.rows_affected() != 1 {
            return Err(ReconciliationError::Storage(
                "expired payment state changed concurrently".to_string(),
            ));
        }
        tx.commit().await.map_err(storage)?;
        Ok(true)
    }

    async fn acquire_lease(
        &self,
        owner_id: &str,
        now: DateTime<Utc>,
    ) -> Result<i64, ReconciliationError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        tx.execute(self.db.stmt(
            "INSERT INTO store_reconciliation_leases
                (name, owner_id, epoch, expires_at, updated_at)
             VALUES ($1, '', 0, '1970-01-01T00:00:00.000000Z', $2)
             ON CONFLICT (name) DO NOTHING",
            vec![LEASE_NAME.into(), timestamp(now).into()],
        ))
        .await
        .map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        let row = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT owner_id, epoch, expires_at FROM store_reconciliation_leases
                     WHERE name = $1{lock}"
                ),
                vec![LEASE_NAME.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| ReconciliationError::Storage("inserted lease is missing".to_string()))?;
        let current_owner = row_string(&row, "owner_id")?;
        let expires_at = row_timestamp(&row, "expires_at")?;
        if !current_owner.is_empty() && current_owner != owner_id && expires_at > now {
            tx.commit().await.map_err(storage)?;
            return Err(ReconciliationError::LeaseUnavailable);
        }
        let epoch = row_i64(&row, "epoch")?
            .checked_add(1)
            .ok_or_else(|| ReconciliationError::Storage("lease epoch overflow".to_string()))?;
        let expires_at = now + chrono::Duration::seconds(LEASE_SECONDS);
        tx.execute(self.db.stmt(
            "UPDATE store_reconciliation_leases
             SET owner_id = $2, epoch = $3, expires_at = $4, updated_at = $5
             WHERE name = $1",
            vec![
                LEASE_NAME.into(),
                owner_id.into(),
                epoch.into(),
                timestamp(expires_at).into(),
                timestamp(now).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(epoch)
    }

    async fn fulfillment_candidates(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, ReconciliationError> {
        let initial_cutoff = now - chrono::Duration::seconds(30);
        self.db
            .read()
            .query_all(self.db.stmt(
                "SELECT o.id
                 FROM store_orders o
                 LEFT JOIN store_fulfillment_retries r ON r.order_id = o.id
                 WHERE o.payment_state = 'paid'
                   AND o.fulfillment_state IN ('pending', 'failed')
                   AND o.payment_hold = 0 AND o.paid_at IS NOT NULL
                   AND ((r.order_id IS NULL AND o.paid_at <= $1)
                        OR (r.order_id IS NOT NULL AND r.next_attempt_at <= $2))
                 ORDER BY o.paid_at ASC, o.id ASC
                 LIMIT $3",
                vec![
                    timestamp(initial_cutoff).into(),
                    timestamp(now).into(),
                    BATCH_SIZE.into(),
                ],
            ))
            .await
            .map_err(storage)?
            .iter()
            .map(|row| row_string(row, "id"))
            .collect()
    }

    async fn schedule_fulfillment_retry(
        &self,
        order_id: &str,
        owner_id: &str,
        epoch: i64,
        now: DateTime<Utc>,
    ) -> Result<(), ReconciliationError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        validate_fence(&self.db, &*tx, owner_id, epoch, now, lock).await?;
        let order = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT payment_state, fulfillment_state FROM store_orders
                     WHERE id = $1{lock}"
                ),
                vec![order_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| ReconciliationError::Storage("retry order is missing".to_string()))?;
        if row_string(&order, "fulfillment_state")? == "fulfilled" {
            tx.execute(self.db.stmt(
                "DELETE FROM store_fulfillment_retries WHERE order_id = $1",
                vec![order_id.into()],
            ))
            .await
            .map_err(storage)?;
            tx.commit().await.map_err(storage)?;
            return Ok(());
        }
        if row_string(&order, "payment_state")? != "paid" {
            return Err(ReconciliationError::Storage(
                "retry order is no longer paid".to_string(),
            ));
        }
        let retry = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT attempt_count FROM store_fulfillment_retries
                     WHERE order_id = $1{lock}"
                ),
                vec![order_id.into()],
            ))
            .await
            .map_err(storage)?;
        let attempt_count = retry
            .as_ref()
            .map(|row| row_i64(row, "attempt_count"))
            .transpose()?
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| ReconciliationError::Storage("retry count overflow".to_string()))?;
        let delay_seconds = match attempt_count {
            1 => 120,
            2 => 600,
            _ => 3600,
        };
        let next_attempt_at = now + chrono::Duration::seconds(delay_seconds);
        tx.execute(self.db.stmt(
            "INSERT INTO store_fulfillment_retries
                (order_id, attempt_count, next_attempt_at, last_error_category, updated_at)
             VALUES ($1, $2, $3, 'fulfillment_failed', $4)
             ON CONFLICT (order_id) DO UPDATE SET
                attempt_count = $2, next_attempt_at = $3,
                last_error_category = 'fulfillment_failed', updated_at = $4",
            vec![
                order_id.into(),
                attempt_count.into(),
                timestamp(next_attempt_at).into(),
                timestamp(now).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }
}

fn payment_query_event(
    query: &PaymentQueryOutcome,
    provider_transaction_id: &str,
    now: DateTime<Utc>,
) -> Result<ApplyProviderEventInput, ReconciliationError> {
    let parsed_json = serde_json::json!({
        "source": "payment_query",
        "attempt_id": &query.attempt_id,
        "provider_object_id": &query.provider_object_id,
        "provider_transaction_id": provider_transaction_id,
        "order_number": &query.order_number,
        "amount_minor": &query.amount_minor,
        "currency": query.currency,
    });
    let evidence = serde_json::to_vec(&parsed_json).map_err(storage)?;
    let body_digest = Sha256::digest(&evidence)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(ApplyProviderEventInput {
        event_row_id: Uuid::new_v4().to_string(),
        credential_version_id: query.credential_version_id.clone(),
        verification_credential_version_id: query.credential_version_id.clone(),
        provider_event_id: format!(
            "payment-query:{}:{provider_transaction_id}",
            query.attempt_id
        ),
        event_kind: "payment_query_succeeded".to_string(),
        order_id: query.order_id.clone(),
        attempt_id: query.attempt_id.clone(),
        provider_transaction_id: provider_transaction_id.to_string(),
        provider_object_id: query.provider_object_id.clone(),
        order_number: query.order_number.clone(),
        merchant_account_identity: query.merchant_account_identity.clone(),
        amount_minor: query.amount_minor.clone(),
        currency: query.currency,
        body_digest,
        parsed_json,
        raw_body: None,
        source_ip: None,
        user_agent: Some("monoize-store-reconciler".to_string()),
        received_at: now,
    })
}

fn payment_operations_error_category(error: &PaymentOperationsError) -> &'static str {
    match error {
        PaymentOperationsError::AttemptNotFound => "attempt_not_found",
        PaymentOperationsError::CredentialNotFound => "credential_not_found",
        PaymentOperationsError::CredentialBindingMismatch => "credential_binding_mismatch",
        PaymentOperationsError::CredentialDecryptionFailed => "credential_decryption_failed",
        PaymentOperationsError::CredentialInvalid => "credential_invalid",
        PaymentOperationsError::AccountIdentityMismatch => "account_identity_mismatch",
        PaymentOperationsError::PaymentContractInvalid => "payment_contract_invalid",
        PaymentOperationsError::UnsupportedAdapter => "unsupported_adapter",
        PaymentOperationsError::Provider(super::payment::AdapterError::Ambiguous) => {
            "provider_transport_ambiguous"
        }
        PaymentOperationsError::Provider(super::payment::AdapterError::Verification) => {
            "provider_verification_failed"
        }
        PaymentOperationsError::Provider(super::payment::AdapterError::InvalidConfiguration) => {
            "provider_configuration_invalid"
        }
        PaymentOperationsError::Provider(super::payment::AdapterError::InvalidRequest) => {
            "provider_request_invalid"
        }
        PaymentOperationsError::Provider(super::payment::AdapterError::Rejected) => {
            "provider_query_rejected"
        }
        PaymentOperationsError::Provider(super::payment::AdapterError::Unsupported) => {
            "provider_query_unsupported"
        }
        PaymentOperationsError::Storage(_) => "storage_error",
    }
}

fn reconciliation_now(
    started_at_wall: DateTime<Utc>,
    started_at_monotonic: Instant,
) -> Result<DateTime<Utc>, ReconciliationError> {
    let elapsed_seconds = i64::try_from(started_at_monotonic.elapsed().as_secs())
        .map_err(|_| ReconciliationError::Storage("reconciliation time overflow".to_string()))?;
    let elapsed = chrono::Duration::seconds(elapsed_seconds);
    started_at_wall
        .checked_add_signed(elapsed)
        .ok_or_else(|| ReconciliationError::Storage("reconciliation time overflow".to_string()))
}

async fn validate_fence<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    owner_id: &str,
    epoch: i64,
    now: DateTime<Utc>,
    lock: &str,
) -> Result<(), ReconciliationError> {
    let row = connection
        .query_one(db.stmt(
            &format!(
                "SELECT owner_id, epoch, expires_at FROM store_reconciliation_leases
                 WHERE name = $1{lock}"
            ),
            vec![LEASE_NAME.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(ReconciliationError::LeaseLost)?;
    if row_string(&row, "owner_id")? != owner_id
        || row_i64(&row, "epoch")? != epoch
        || row_timestamp(&row, "expires_at")? <= now
    {
        return Err(ReconciliationError::LeaseLost);
    }
    Ok(())
}

fn validate_owner_id(owner_id: &str) -> Result<(), ReconciliationError> {
    if owner_id.is_empty() || owner_id.len() > 128 || owner_id.trim() != owner_id {
        return Err(ReconciliationError::Storage(
            "reconciliation owner ID is invalid".to_string(),
        ));
    }
    Ok(())
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, ReconciliationError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_string(
    row: &QueryResult,
    column: &str,
) -> Result<Option<String>, ReconciliationError> {
    row.try_get("", column).map_err(storage)
}

fn row_i64(row: &QueryResult, column: &str) -> Result<i64, ReconciliationError> {
    row.try_get("", column).map_err(storage)
}

fn row_timestamp(row: &QueryResult, column: &str) -> Result<DateTime<Utc>, ReconciliationError> {
    DateTime::parse_from_rfc3339(&row_string(row, column)?)
        .map(|value| value.with_timezone(&Utc))
        .map_err(storage)
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn storage(error: impl ToString) -> ReconciliationError {
    ReconciliationError::Storage(error.to_string())
}
