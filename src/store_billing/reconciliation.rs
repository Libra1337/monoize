use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, QueryResult};

use super::callbacks::{CallbackStoreError, PaymentCallbackStore};
use crate::db::DbPool;

const LEASE_NAME: &str = "store_reconciler";
const LEASE_SECONDS: i64 = 90;
const BATCH_SIZE: i64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationOutcome {
    pub scanned: usize,
    pub fulfilled: usize,
    pub failed: usize,
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

#[derive(Debug, Clone)]
pub struct StoreReconciler {
    db: DbPool,
}

impl StoreReconciler {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn run_once(
        &self,
        owner_id: &str,
        now: DateTime<Utc>,
    ) -> Result<ReconciliationOutcome, ReconciliationError> {
        validate_owner_id(owner_id)?;
        let epoch = self.acquire_lease(owner_id, now).await?;
        let candidates = self.fulfillment_candidates(now).await?;
        let mut outcome = ReconciliationOutcome {
            scanned: candidates.len(),
            fulfilled: 0,
            failed: 0,
        };
        let callbacks = PaymentCallbackStore::new(self.db.clone());
        for order_id in candidates {
            match callbacks
                .fulfill_paid_order_fenced(&order_id, owner_id, epoch, now)
                .await
            {
                Ok(()) => outcome.fulfilled += 1,
                Err(CallbackStoreError::Fulfillment(_)) => {
                    self.schedule_fulfillment_retry(&order_id, owner_id, epoch, now)
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
