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
use super::recovery::RecoveryStore;
use super::refund_operations::{RefundOperations, RefundOperationsError, RefundQueryProjection};
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
    pub refund_queries: usize,
    pub refunds_terminal: usize,
    pub refund_query_failures: usize,
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
    refund_operations: Option<RefundOperations>,
}

#[derive(Debug, Clone)]
struct RefundReconciliationCandidate {
    refund_id: String,
    order_id: String,
    pending_at: DateTime<Utc>,
}

impl StoreReconciler {
    pub fn new(db: DbPool) -> Self {
        Self {
            db,
            payment_queries: None,
            refund_operations: None,
        }
    }

    pub fn with_payment_queries(mut self, payment_queries: PaymentQueryOperations) -> Self {
        self.payment_queries = Some(payment_queries);
        self
    }

    pub fn with_refund_operations(mut self, refund_operations: RefundOperations) -> Self {
        self.refund_operations = Some(refund_operations);
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
        let refund_candidates = if self.refund_operations.is_some() {
            self.refund_query_candidates(now).await?
        } else {
            Vec::new()
        };
        let refund_alert_candidates = self.refund_alert_candidates(now).await?;
        let candidates = self.fulfillment_candidates(now).await?;
        let mut outcome = ReconciliationOutcome {
            scanned: payment_candidates.len()
                + refund_candidates.len()
                + refund_alert_candidates.len()
                + candidates.len(),
            fulfilled: 0,
            failed: 0,
            payment_queries: 0,
            payments_applied: 0,
            attempts_expired: 0,
            query_failures: 0,
            refund_queries: 0,
            refunds_terminal: 0,
            refund_query_failures: 0,
        };
        let callbacks = PaymentCallbackStore::new(self.db.clone());
        for candidate in refund_alert_candidates {
            let fence_now = reconciliation_now(now, started_at)?;
            self.open_refund_pending_case_fenced(&candidate, owner_id, epoch, fence_now)
                .await?;
        }
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
        if let Some(refund_operations) = &self.refund_operations {
            for candidate in refund_candidates {
                outcome.refund_queries += 1;
                let result = refund_operations
                    .query_provider_with_context(&candidate.order_id, &candidate.refund_id)
                    .await;
                let fence_now = reconciliation_now(now, started_at)?;
                match result {
                    Ok(query) => match query.projection {
                        projection @ (RefundQueryProjection::AlreadyTerminal
                        | RefundQueryProjection::Succeeded { .. }
                        | RefundQueryProjection::Failed { .. }) => {
                            if self
                                .complete_refund_query_fenced(
                                    &candidate, projection, owner_id, epoch, fence_now,
                                )
                                .await?
                            {
                                outcome.refunds_terminal += 1;
                            }
                        }
                        RefundQueryProjection::Pending { provider_refund_id } => {
                            self.schedule_refund_query_fenced(
                                &candidate,
                                true,
                                provider_refund_id.as_deref(),
                                None,
                                owner_id,
                                epoch,
                                fence_now,
                            )
                            .await?;
                        }
                    },
                    Err(RefundOperationsError::Storage(error)) => {
                        return Err(ReconciliationError::Storage(error));
                    }
                    Err(error) => {
                        self.schedule_refund_query_fenced(
                            &candidate,
                            false,
                            None,
                            Some(refund_operations_error_category(&error)),
                            owner_id,
                            epoch,
                            fence_now,
                        )
                        .await?;
                        outcome.refund_query_failures += 1;
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

    async fn refund_query_candidates(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<RefundReconciliationCandidate>, ReconciliationError> {
        let initial_cutoff = now - chrono::Duration::minutes(1);
        let initial = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT f.id AS refund_id, f.order_id, o.refund_pending_at AS pending_at
                 FROM store_refunds f
                 JOIN store_orders o ON o.id = f.order_id
                 LEFT JOIN store_refund_query_retries r ON r.refund_id = f.id
                 WHERE f.state = 'pending' AND o.payment_state = 'refund_pending'
                   AND r.refund_id IS NULL AND o.refund_pending_at <= $1
                 ORDER BY o.refund_pending_at ASC, f.id ASC
                 LIMIT $2",
                vec![timestamp(initial_cutoff).into(), BATCH_SIZE.into()],
            ))
            .await
            .map_err(storage)?;
        let retry = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT f.id AS refund_id, f.order_id, o.refund_pending_at AS pending_at,
                        r.next_attempt_at AS due_at
                 FROM store_refunds f
                 JOIN store_orders o ON o.id = f.order_id
                 JOIN store_refund_query_retries r ON r.refund_id = f.id
                 WHERE f.state = 'pending' AND o.payment_state = 'refund_pending'
                   AND r.next_attempt_at <= $1
                 ORDER BY r.next_attempt_at ASC, f.id ASC
                 LIMIT $2",
                vec![timestamp(now).into(), BATCH_SIZE.into()],
            ))
            .await
            .map_err(storage)?;
        let mut candidates = Vec::with_capacity(initial.len() + retry.len());
        for row in initial {
            let pending_at = row_timestamp(&row, "pending_at")?;
            let due_at = pending_at
                .checked_add_signed(chrono::Duration::minutes(1))
                .ok_or_else(|| {
                    ReconciliationError::Storage("refund query due time overflow".to_string())
                })?;
            candidates.push((
                due_at,
                RefundReconciliationCandidate {
                    refund_id: row_string(&row, "refund_id")?,
                    order_id: row_string(&row, "order_id")?,
                    pending_at,
                },
            ));
        }
        for row in retry {
            candidates.push((
                row_timestamp(&row, "due_at")?,
                RefundReconciliationCandidate {
                    refund_id: row_string(&row, "refund_id")?,
                    order_id: row_string(&row, "order_id")?,
                    pending_at: row_timestamp(&row, "pending_at")?,
                },
            ));
        }
        candidates.sort_by(|(left_due, left), (right_due, right)| {
            left_due
                .cmp(right_due)
                .then_with(|| left.refund_id.cmp(&right.refund_id))
        });
        candidates.truncate(BATCH_SIZE as usize);
        Ok(candidates
            .into_iter()
            .map(|(_, candidate)| candidate)
            .collect())
    }

    async fn refund_alert_candidates(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<RefundReconciliationCandidate>, ReconciliationError> {
        let cutoff = now - chrono::Duration::minutes(15);
        self.db
            .read()
            .query_all(self.db.stmt(
                "SELECT f.id AS refund_id, f.order_id, o.refund_pending_at AS pending_at
                 FROM store_refunds f
                 JOIN store_orders o ON o.id = f.order_id
                 LEFT JOIN store_refund_query_retries r ON r.refund_id = f.id
                 LEFT JOIN store_reconciliation_cases c
                   ON c.id = ('refund-pending:' || f.id) AND c.state = 'open'
                 WHERE f.state = 'pending' AND o.payment_state = 'refund_pending'
                   AND o.refund_pending_at <= $1
                   AND (r.refund_id IS NULL OR r.alerted_at IS NULL)
                   AND c.id IS NULL
                 ORDER BY o.refund_pending_at ASC, f.id ASC
                 LIMIT $2",
                vec![timestamp(cutoff).into(), BATCH_SIZE.into()],
            ))
            .await
            .map_err(storage)?
            .iter()
            .map(|row| {
                Ok(RefundReconciliationCandidate {
                    refund_id: row_string(row, "refund_id")?,
                    order_id: row_string(row, "order_id")?,
                    pending_at: row_timestamp(row, "pending_at")?,
                })
            })
            .collect()
    }

    async fn open_refund_pending_case_fenced(
        &self,
        candidate: &RefundReconciliationCandidate,
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
        let refund = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT f.order_id, f.state AS refund_state, o.payment_state,
                            o.refund_pending_at AS pending_at, a.channel_id
                     FROM store_refunds f
                     JOIN store_orders o ON o.id = f.order_id
                     JOIN store_payment_attempts a ON a.id = f.attempt_id
                     WHERE f.id = $1{lock}"
                ),
                vec![candidate.refund_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| ReconciliationError::Storage("refund is missing".to_string()))?;
        let pending_at = row_timestamp(&refund, "pending_at")?;
        let alert_due_at = pending_at
            .checked_add_signed(chrono::Duration::minutes(15))
            .ok_or_else(|| {
                ReconciliationError::Storage("refund alert time overflow".to_string())
            })?;
        if row_string(&refund, "order_id")? != candidate.order_id
            || pending_at != candidate.pending_at
            || row_string(&refund, "refund_state")? != "pending"
            || row_string(&refund, "payment_state")? != "refund_pending"
            || now < alert_due_at
        {
            tx.commit().await.map_err(storage)?;
            return Ok(false);
        }
        let retry = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT attempt_count, next_attempt_at, last_error_category, alerted_at
                     FROM store_refund_query_retries WHERE refund_id = $1{lock}"
                ),
                vec![candidate.refund_id.clone().into()],
            ))
            .await
            .map_err(storage)?;
        let case_id = format!("refund-pending:{}", candidate.refund_id);
        let case = tx
            .query_one(self.db.stmt(
                &format!("SELECT state FROM store_reconciliation_cases WHERE id = $1{lock}"),
                vec![case_id.clone().into()],
            ))
            .await
            .map_err(storage)?;
        if retry
            .as_ref()
            .map(|row| row_optional_string(row, "alerted_at"))
            .transpose()?
            .flatten()
            .is_some()
            || case
                .as_ref()
                .map(|row| row_string(row, "state"))
                .transpose()?
                .as_deref()
                == Some("open")
        {
            tx.commit().await.map_err(storage)?;
            return Ok(false);
        }
        let last_error_category = retry
            .as_ref()
            .map(|row| row_optional_string(row, "last_error_category"))
            .transpose()?
            .flatten();
        if retry.is_some() {
            let changed = tx
                .execute(self.db.stmt(
                    "UPDATE store_refund_query_retries
                     SET alerted_at = $2, updated_at = $2
                     WHERE refund_id = $1 AND alerted_at IS NULL",
                    vec![candidate.refund_id.clone().into(), timestamp(now).into()],
                ))
                .await
                .map_err(storage)?;
            if changed.rows_affected() != 1 {
                return Err(ReconciliationError::Storage(
                    "refund alert state changed concurrently".to_string(),
                ));
            }
        } else {
            let initial_due_at = pending_at
                .checked_add_signed(chrono::Duration::minutes(1))
                .ok_or_else(|| {
                    ReconciliationError::Storage("refund query due time overflow".to_string())
                })?;
            tx.execute(self.db.stmt(
                "INSERT INTO store_refund_query_retries
                    (refund_id, attempt_count, next_attempt_at, last_error_category,
                     alerted_at, updated_at)
                 VALUES ($1, 0, $2, NULL, $3, $3)",
                vec![
                    candidate.refund_id.clone().into(),
                    timestamp(initial_due_at).into(),
                    timestamp(now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        }
        let mut evidence = serde_json::json!({ "refund_id": candidate.refund_id });
        if let Some(category) = last_error_category {
            evidence["error_category"] = serde_json::Value::String(category);
        }
        tx.execute(self.db.stmt(
            "INSERT INTO store_reconciliation_cases
                (id, order_id, channel_id, severity, kind, state, evidence_json,
                 created_at, updated_at)
             VALUES ($1, $2, $3, 'high', 'refund_pending', 'open', $4, $5, $5)
             ON CONFLICT (id) DO UPDATE SET
                order_id = $2, channel_id = $3, severity = 'high',
                kind = 'refund_pending', state = 'open', evidence_json = $4,
                updated_at = $5, closed_at = NULL",
            vec![
                case_id.into(),
                candidate.order_id.clone().into(),
                row_string(&refund, "channel_id")?.into(),
                evidence.to_string().into(),
                timestamp(now).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(true)
    }

    async fn schedule_refund_query_fenced(
        &self,
        candidate: &RefundReconciliationCandidate,
        project_pending: bool,
        provider_refund_id: Option<&str>,
        error_category: Option<&str>,
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
        let refund = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT f.order_id, f.state AS refund_state, o.payment_state,
                            a.channel_id
                     FROM store_refunds f
                     JOIN store_orders o ON o.id = f.order_id
                     JOIN store_payment_attempts a ON a.id = f.attempt_id
                     WHERE f.id = $1{lock}"
                ),
                vec![candidate.refund_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| ReconciliationError::Storage("refund is missing".to_string()))?;
        if row_string(&refund, "order_id")? != candidate.order_id
            || row_string(&refund, "refund_state")? != "pending"
            || row_string(&refund, "payment_state")? != "refund_pending"
        {
            tx.commit().await.map_err(storage)?;
            return Ok(());
        }
        if project_pending {
            RecoveryStore::new(self.db.clone())
                .mark_refund_pending_outcome_in(
                    &*tx,
                    &candidate.refund_id,
                    provider_refund_id,
                    &timestamp(now),
                )
                .await
                .map_err(storage)?;
        }
        let retry = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT attempt_count, alerted_at
                     FROM store_refund_query_retries WHERE refund_id = $1{lock}"
                ),
                vec![candidate.refund_id.clone().into()],
            ))
            .await
            .map_err(storage)?;
        let attempt_count = retry
            .as_ref()
            .map(|row| row_i64(row, "attempt_count"))
            .transpose()?
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                ReconciliationError::Storage("refund query count overflow".to_string())
            })?;
        let delay = match attempt_count {
            1 => chrono::Duration::minutes(5),
            2 => chrono::Duration::minutes(15),
            _ => chrono::Duration::hours(1),
        };
        let next_attempt_at = now.checked_add_signed(delay).ok_or_else(|| {
            ReconciliationError::Storage("refund query retry time overflow".to_string())
        })?;
        let alert_due_at = candidate
            .pending_at
            .checked_add_signed(chrono::Duration::minutes(15))
            .ok_or_else(|| {
                ReconciliationError::Storage("refund alert time overflow".to_string())
            })?;
        let previous_alerted_at = retry
            .as_ref()
            .map(|row| row_optional_string(row, "alerted_at"))
            .transpose()?
            .flatten();
        let alerted_at = if now >= alert_due_at {
            previous_alerted_at.or_else(|| Some(timestamp(now)))
        } else {
            previous_alerted_at
        };
        tx.execute(self.db.stmt(
            "INSERT INTO store_refund_query_retries
                (refund_id, attempt_count, next_attempt_at, last_error_category,
                 alerted_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (refund_id) DO UPDATE SET
                attempt_count = $2, next_attempt_at = $3,
                last_error_category = $4, alerted_at = $5, updated_at = $6",
            vec![
                candidate.refund_id.clone().into(),
                attempt_count.into(),
                timestamp(next_attempt_at).into(),
                error_category.map(str::to_string).into(),
                alerted_at.into(),
                timestamp(now).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        if now >= alert_due_at {
            let mut evidence = serde_json::json!({ "refund_id": candidate.refund_id });
            if let Some(category) = error_category {
                evidence["error_category"] = serde_json::Value::String(category.to_string());
            }
            tx.execute(self.db.stmt(
                "INSERT INTO store_reconciliation_cases
                    (id, order_id, channel_id, severity, kind, state, evidence_json,
                     created_at, updated_at)
                 VALUES ($1, $2, $3, 'high', 'refund_pending', 'open', $4, $5, $5)
                 ON CONFLICT (id) DO UPDATE SET
                    order_id = $2, channel_id = $3, severity = 'high',
                    kind = 'refund_pending', state = 'open', evidence_json = $4,
                    updated_at = $5, closed_at = NULL",
                vec![
                    format!("refund-pending:{}", candidate.refund_id).into(),
                    candidate.order_id.clone().into(),
                    row_string(&refund, "channel_id")?.into(),
                    evidence.to_string().into(),
                    timestamp(now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        }
        tx.commit().await.map_err(storage)
    }

    async fn complete_refund_query_fenced(
        &self,
        candidate: &RefundReconciliationCandidate,
        projection: RefundQueryProjection,
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
        let refund = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT f.order_id, f.state AS refund_state, o.payment_state
                     FROM store_refunds f
                     JOIN store_orders o ON o.id = f.order_id
                     WHERE f.id = $1{lock}"
                ),
                vec![candidate.refund_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| ReconciliationError::Storage("refund is missing".to_string()))?;
        if row_string(&refund, "order_id")? != candidate.order_id {
            return Err(ReconciliationError::Storage(
                "refund order changed concurrently".to_string(),
            ));
        }
        let refund_state = row_string(&refund, "refund_state")?;
        if matches!(projection, RefundQueryProjection::AlreadyTerminal) {
            if !matches!(refund_state.as_str(), "succeeded" | "failed") {
                tx.commit().await.map_err(storage)?;
                return Ok(false);
            }
        } else if matches!(refund_state.as_str(), "succeeded" | "failed") {
        } else if refund_state != "pending"
            || row_string(&refund, "payment_state")? != "refund_pending"
        {
            tx.commit().await.map_err(storage)?;
            return Ok(false);
        } else {
            let recovery = RecoveryStore::new(self.db.clone());
            match projection {
                RefundQueryProjection::Succeeded { provider_refund_id } => {
                    if provider_refund_id.is_some() {
                        recovery
                            .mark_refund_pending_outcome_in(
                                &*tx,
                                &candidate.refund_id,
                                provider_refund_id.as_deref(),
                                &timestamp(now),
                            )
                            .await
                            .map_err(storage)?;
                    }
                    recovery
                        .complete_refund_in(&*tx, &candidate.refund_id, &timestamp(now))
                        .await
                        .map_err(storage)?;
                }
                RefundQueryProjection::Failed { provider_refund_id } => {
                    if provider_refund_id.is_some() {
                        recovery
                            .mark_refund_pending_outcome_in(
                                &*tx,
                                &candidate.refund_id,
                                provider_refund_id.as_deref(),
                                &timestamp(now),
                            )
                            .await
                            .map_err(storage)?;
                    }
                    recovery
                        .reject_refund_in(&*tx, &candidate.refund_id, &timestamp(now))
                        .await
                        .map_err(storage)?;
                }
                RefundQueryProjection::AlreadyTerminal | RefundQueryProjection::Pending { .. } => {
                    return Err(ReconciliationError::Storage(
                        "refund terminal projection is invalid".to_string(),
                    ));
                }
            }
        }
        tx.execute(self.db.stmt(
            "DELETE FROM store_refund_query_retries WHERE refund_id = $1",
            vec![candidate.refund_id.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_reconciliation_cases
             SET state = 'closed', updated_at = $2, closed_at = $2
             WHERE id = $1 AND state <> 'closed'",
            vec![
                format!("refund-pending:{}", candidate.refund_id).into(),
                timestamp(now).into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(true)
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

fn refund_operations_error_category(error: &RefundOperationsError) -> &'static str {
    match error {
        RefundOperationsError::InvalidInput => "invalid_input",
        RefundOperationsError::NotFound => "refund_not_found",
        RefundOperationsError::OrderNotRefundable => "order_not_refundable",
        RefundOperationsError::IdempotencyConflict => "idempotency_conflict",
        RefundOperationsError::InsufficientBalance => "insufficient_balance",
        RefundOperationsError::ConfigurationUnavailable => "payment_configuration_unavailable",
        RefundOperationsError::Storage(_) => "storage_error",
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
