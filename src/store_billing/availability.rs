use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, DbErr, QueryResult, SqlErr};
use serde::Serialize;
use thiserror::Error;

use crate::db::DbPool;

pub const STORE_PRIMARY_LEASE_NAME: &str = "store_primary";
pub const STORE_PRIMARY_LEASE_SECONDS: i64 = 15;
pub const STORE_PRIMARY_RENEWAL_SECONDS: u64 = 5;
const STORE_PRIMARY_RENEWAL_SAFETY_SECONDS: i64 = 5;
const STORE_PRIMARY_RETRY_DELAYS_MS: [u64; 3] = [100, 250, 500];
const STORE_PRIMARY_RENEWAL_ROUND_TIMEOUT_MS: u64 = 2_000;
const BACKGROUND_SHUTDOWN_POLL_MS: u64 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorePrimaryLeaseError {
    #[error("Store Primary owner ID is invalid")]
    InvalidOwner,
    #[error("Store Primary lease is unavailable")]
    Unavailable,
    #[error("Store Primary lease is missing")]
    Missing,
    #[error("Store Primary lease owner does not match")]
    OwnerMismatch,
    #[error("Store Primary lease epoch does not match")]
    EpochMismatch,
    #[error("Store Primary lease is expired")]
    Expired,
    #[error("Store Primary lease renewal has failed")]
    RenewalFailed,
    #[error("Store Primary lease epoch overflow")]
    EpochOverflow,
    #[error("Store Primary lease storage failed: {0}")]
    Storage(String),
}

impl From<DbErr> for StorePrimaryLeaseError {
    fn from(error: DbErr) -> Self {
        Self::Storage(error.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct StorePrimaryLease {
    db: DbPool,
    owner_id: Arc<str>,
    epoch: i64,
    renewal_failed: Arc<AtomicBool>,
    expires_at: Arc<tokio::sync::RwLock<DateTime<Utc>>>,
    last_successful_renewal_at: Arc<tokio::sync::RwLock<Option<DateTime<Utc>>>>,
    consecutive_failures: Arc<AtomicU64>,
    last_failure_kind: Arc<tokio::sync::RwLock<Option<String>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorePrimaryLeaseStatus {
    pub state: String,
    pub owner_id: Option<String>,
    pub epoch: Option<i64>,
    pub expires_at: Option<String>,
    pub last_successful_renewal_at: Option<String>,
    pub consecutive_failures: u64,
    pub last_failure_kind: Option<String>,
}

#[derive(Debug)]
struct StoredLease {
    owner_id: String,
    epoch: i64,
    expires_at: DateTime<Utc>,
}

impl StorePrimaryLease {
    pub async fn acquire(
        db: DbPool,
        owner_id: impl Into<String>,
    ) -> Result<Self, StorePrimaryLeaseError> {
        Self::acquire_inner(db, owner_id.into(), None).await
    }

    pub async fn acquire_at(
        db: DbPool,
        owner_id: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, StorePrimaryLeaseError> {
        Self::acquire_inner(db, owner_id.into(), Some(canonical_time(now)?)).await
    }

    async fn acquire_inner(
        db: DbPool,
        owner_id: String,
        now: Option<DateTime<Utc>>,
    ) -> Result<Self, StorePrimaryLeaseError> {
        if owner_id.trim().is_empty() {
            return Err(StorePrimaryLeaseError::InvalidOwner);
        }
        let acquisition_time = now.unwrap_or_else(Utc::now);
        let transaction_db = db.clone();
        let transaction_owner = owner_id.clone();
        let epoch = if db.is_sqlite() {
            db.with_immediate_write(move |connection| {
                Box::pin(async move {
                    acquire_locked(
                        &transaction_db,
                        connection,
                        &transaction_owner,
                        Some(acquisition_time),
                        false,
                    )
                    .await
                })
            })
            .await?
        } else {
            let tx = db.begin_write().await?;
            let outcome = acquire_locked(&db, &*tx, &owner_id, Some(acquisition_time), true).await;
            finish_transaction(tx, outcome).await?
        };
        Ok(Self {
            db,
            owner_id: owner_id.into(),
            epoch,
            renewal_failed: Arc::new(AtomicBool::new(false)),
            expires_at: Arc::new(tokio::sync::RwLock::new(
                acquisition_time
                    .checked_add_signed(Duration::seconds(STORE_PRIMARY_LEASE_SECONDS))
                    .ok_or_else(|| {
                        StorePrimaryLeaseError::Storage("lease expiry overflow".to_string())
                    })?,
            )),
            last_successful_renewal_at: Arc::new(tokio::sync::RwLock::new(None)),
            consecutive_failures: Arc::new(AtomicU64::new(0)),
            last_failure_kind: Arc::new(tokio::sync::RwLock::new(None)),
        })
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn epoch(&self) -> i64 {
        self.epoch
    }

    pub async fn renew(&self) -> Result<(), StorePrimaryLeaseError> {
        self.renew_inner(None).await
    }

    pub async fn renew_at(&self, now: DateTime<Utc>) -> Result<(), StorePrimaryLeaseError> {
        let now = match canonical_time(now) {
            Ok(now) => now,
            Err(error) => {
                self.renewal_failed.store(true, Ordering::Release);
                return Err(error);
            }
        };
        self.renew_inner(Some(now)).await
    }

    async fn renew_inner(&self, now: Option<DateTime<Utc>>) -> Result<(), StorePrimaryLeaseError> {
        if self.renewal_failed.load(Ordering::Acquire) {
            return Err(StorePrimaryLeaseError::RenewalFailed);
        }
        let renewal_time = now.unwrap_or_else(Utc::now);
        let transaction_db = self.db.clone();
        let owner_id = self.owner_id.to_string();
        let epoch = self.epoch;
        let outcome = if self.db.is_sqlite() {
            self.db
                .with_immediate_write(move |connection| {
                    Box::pin(async move {
                        renew_locked(
                            &transaction_db,
                            connection,
                            &owner_id,
                            epoch,
                            Some(renewal_time),
                            false,
                        )
                        .await
                    })
                })
                .await
        } else {
            match self.db.begin_write().await {
                Ok(tx) => {
                    let outcome = renew_locked(
                        &self.db,
                        &*tx,
                        &self.owner_id,
                        self.epoch,
                        Some(renewal_time),
                        true,
                    )
                    .await;
                    finish_transaction(tx, outcome).await
                }
                Err(error) => Err(error.into()),
            }
        };
        if outcome.is_ok() {
            *self.expires_at.write().await = renewal_time
                .checked_add_signed(Duration::seconds(STORE_PRIMARY_LEASE_SECONDS))
                .ok_or_else(|| {
                    StorePrimaryLeaseError::Storage("lease expiry overflow".to_string())
                })?;
            *self.last_successful_renewal_at.write().await = Some(renewal_time);
            self.consecutive_failures.store(0, Ordering::Release);
            *self.last_failure_kind.write().await = None;
        } else {
            self.consecutive_failures.fetch_add(1, Ordering::AcqRel);
            *self.last_failure_kind.write().await =
                Some(failure_kind(outcome.as_ref().err().expect("error")));
        }
        if outcome.as_ref().is_err_and(|error| is_fencing_loss(error)) {
            self.renewal_failed.store(true, Ordering::Release);
        }
        outcome
    }

    async fn remaining_ttl(&self) -> Duration {
        *self.expires_at.read().await - Utc::now()
    }

    pub async fn status_at(
        &self,
        now: DateTime<Utc>,
    ) -> Result<StorePrimaryLeaseStatus, StorePrimaryLeaseError> {
        let now = canonical_time(now)?;
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT owner_id, epoch, expires_at FROM store_primary_leases WHERE name = $1",
                vec![STORE_PRIMARY_LEASE_NAME.into()],
            ))
            .await?;
        let stored = row.map(|row| stored_lease(&row)).transpose()?;
        let fenced = match stored.as_ref() {
            None => true,
            Some(stored) => validate_token(stored, &self.owner_id, self.epoch, now).is_err(),
        };
        let state = if self.renewal_failed.load(Ordering::Acquire) || fenced {
            "lease_lost"
        } else if self.consecutive_failures.load(Ordering::Acquire) > 0 {
            "degraded"
        } else {
            "healthy"
        };
        Ok(StorePrimaryLeaseStatus {
            state: state.to_string(),
            owner_id: stored.as_ref().map(|lease| lease.owner_id.clone()),
            epoch: stored.as_ref().map(|lease| lease.epoch),
            expires_at: stored.as_ref().map(|lease| format_time(lease.expires_at)),
            last_successful_renewal_at: self
                .last_successful_renewal_at
                .read()
                .await
                .as_ref()
                .map(|value| format_time(*value)),
            consecutive_failures: self.consecutive_failures.load(Ordering::Acquire),
            last_failure_kind: self.last_failure_kind.read().await.clone(),
        })
    }

    pub async fn validate(&self) -> Result<(), StorePrimaryLeaseError> {
        self.validate_inner(None).await
    }

    pub async fn validate_at(&self, now: DateTime<Utc>) -> Result<(), StorePrimaryLeaseError> {
        self.validate_inner(Some(canonical_time(now)?)).await
    }

    async fn validate_inner(
        &self,
        now: Option<DateTime<Utc>>,
    ) -> Result<(), StorePrimaryLeaseError> {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT owner_id, epoch, expires_at FROM store_primary_leases
                 WHERE name = $1",
                vec![STORE_PRIMARY_LEASE_NAME.into()],
            ))
            .await?
            .ok_or(StorePrimaryLeaseError::Missing)?;
        let stored = stored_lease(&row)?;
        let now = now.unwrap_or_else(Utc::now);
        validate_token(&stored, &self.owner_id, self.epoch, now)?;
        if self.renewal_failed.load(Ordering::Acquire) {
            return Err(StorePrimaryLeaseError::RenewalFailed);
        }
        Ok(())
    }

    pub fn spawn_renewal(&self, shutdown: Arc<AtomicBool>) {
        let lease = self.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(StdDuration::from_secs(STORE_PRIMARY_RENEWAL_SECONDS));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                if let Err(error) = renew_with_retry(&lease, &shutdown).await {
                    request_shutdown_after_renewal_failure(&lease.renewal_failed, &shutdown);
                    tracing::error!(error = %error, "Store Primary lease renewal failed; lease marked lost");
                    break;
                }
            }
        });
    }
}

fn request_shutdown_after_renewal_failure(lease_lost: &AtomicBool, shutdown: &AtomicBool) {
    lease_lost.store(true, Ordering::Release);
    shutdown.store(true, Ordering::Release);
}

pub async fn wait_for_background_shutdown(shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        tokio::time::sleep(StdDuration::from_millis(BACKGROUND_SHUTDOWN_POLL_MS)).await;
    }
}

async fn renew_with_retry(
    lease: &StorePrimaryLease,
    shutdown: &AtomicBool,
) -> Result<(), StorePrimaryLeaseError> {
    tokio::time::timeout(
        StdDuration::from_millis(STORE_PRIMARY_RENEWAL_ROUND_TIMEOUT_MS),
        renew_attempts(lease, shutdown),
    )
    .await
    .unwrap_or(Err(StorePrimaryLeaseError::RenewalFailed))
}

async fn renew_attempts(
    lease: &StorePrimaryLease,
    shutdown: &AtomicBool,
) -> Result<(), StorePrimaryLeaseError> {
    let mut last_error = None;
    for (attempt, delay_ms) in std::iter::once(0)
        .chain(STORE_PRIMARY_RETRY_DELAYS_MS)
        .enumerate()
    {
        if shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        if delay_ms != 0 {
            tokio::time::sleep(StdDuration::from_millis(delay_ms)).await;
        }
        if lease.remaining_ttl().await <= Duration::seconds(STORE_PRIMARY_RENEWAL_SAFETY_SECONDS) {
            return Err(StorePrimaryLeaseError::Expired);
        }
        match lease.renew().await {
            Ok(()) => return Ok(()),
            Err(error) if is_fencing_loss(&error) => return Err(error),
            Err(error) => {
                tracing::warn!(error = %error, attempt, delay_ms, "Store Primary renewal transient failure; retrying");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or(StorePrimaryLeaseError::RenewalFailed))
}

async fn acquire_locked<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    owner_id: &str,
    now: Option<DateTime<Utc>>,
    for_update: bool,
) -> Result<i64, StorePrimaryLeaseError> {
    let row = connection
        .query_one(db.stmt(
            lease_select_sql(for_update),
            vec![STORE_PRIMARY_LEASE_NAME.into()],
        ))
        .await?;
    let now = now.unwrap_or_else(Utc::now);
    let expires_at = now
        .checked_add_signed(Duration::seconds(STORE_PRIMARY_LEASE_SECONDS))
        .ok_or_else(|| StorePrimaryLeaseError::Storage("lease expiry overflow".to_string()))?;
    let expires_at = format_time(expires_at);
    let updated_at = format_time(now);

    let Some(row) = row else {
        let result = connection
            .execute(db.stmt(
                "INSERT INTO store_primary_leases
                    (name, owner_id, epoch, expires_at, updated_at)
                 VALUES ($1, $2, 1, $3, $4)",
                vec![
                    STORE_PRIMARY_LEASE_NAME.into(),
                    owner_id.into(),
                    expires_at.into(),
                    updated_at.into(),
                ],
            ))
            .await;
        return match result {
            Ok(result) if result.rows_affected() == 1 => Ok(1),
            Ok(_) => Err(StorePrimaryLeaseError::Unavailable),
            Err(error) if is_unique_conflict(&error) => Err(StorePrimaryLeaseError::Unavailable),
            Err(error) => Err(error.into()),
        };
    };

    let stored = stored_lease(&row)?;
    let epoch = if stored.owner_id == owner_id {
        stored.epoch
    } else if stored.expires_at <= now {
        stored
            .epoch
            .checked_add(1)
            .ok_or(StorePrimaryLeaseError::EpochOverflow)?
    } else {
        return Err(StorePrimaryLeaseError::Unavailable);
    };
    let result = connection
        .execute(db.stmt(
            "UPDATE store_primary_leases
             SET owner_id = $2, epoch = $3, expires_at = $4, updated_at = $5
             WHERE name = $1 AND owner_id = $6 AND epoch = $7",
            vec![
                STORE_PRIMARY_LEASE_NAME.into(),
                owner_id.into(),
                epoch.into(),
                expires_at.into(),
                updated_at.into(),
                stored.owner_id.into(),
                stored.epoch.into(),
            ],
        ))
        .await?;
    if result.rows_affected() != 1 {
        return Err(StorePrimaryLeaseError::Unavailable);
    }
    Ok(epoch)
}

async fn renew_locked<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    owner_id: &str,
    epoch: i64,
    now: Option<DateTime<Utc>>,
    for_update: bool,
) -> Result<(), StorePrimaryLeaseError> {
    let row = connection
        .query_one(db.stmt(
            lease_select_sql(for_update),
            vec![STORE_PRIMARY_LEASE_NAME.into()],
        ))
        .await?
        .ok_or(StorePrimaryLeaseError::Missing)?;
    let now = now.unwrap_or_else(Utc::now);
    let stored = stored_lease(&row)?;
    validate_token(&stored, owner_id, epoch, now)?;
    let expires_at = now
        .checked_add_signed(Duration::seconds(STORE_PRIMARY_LEASE_SECONDS))
        .ok_or_else(|| StorePrimaryLeaseError::Storage("lease expiry overflow".to_string()))?;
    let result = connection
        .execute(db.stmt(
            "UPDATE store_primary_leases
             SET expires_at = $4, updated_at = $5
             WHERE name = $1 AND owner_id = $2 AND epoch = $3",
            vec![
                STORE_PRIMARY_LEASE_NAME.into(),
                owner_id.into(),
                epoch.into(),
                format_time(expires_at).into(),
                format_time(now).into(),
            ],
        ))
        .await?;
    if result.rows_affected() != 1 {
        return Err(StorePrimaryLeaseError::Unavailable);
    }
    Ok(())
}

fn lease_select_sql(for_update: bool) -> &'static str {
    if for_update {
        "SELECT owner_id, epoch, expires_at FROM store_primary_leases
         WHERE name = $1 FOR UPDATE"
    } else {
        "SELECT owner_id, epoch, expires_at FROM store_primary_leases
         WHERE name = $1"
    }
}

fn stored_lease(row: &QueryResult) -> Result<StoredLease, StorePrimaryLeaseError> {
    let expires_at: String = row.try_get("", "expires_at")?;
    Ok(StoredLease {
        owner_id: row.try_get("", "owner_id")?,
        epoch: row.try_get("", "epoch")?,
        expires_at: DateTime::parse_from_rfc3339(&expires_at)
            .map_err(|error| StorePrimaryLeaseError::Storage(error.to_string()))?
            .with_timezone(&Utc),
    })
}

fn validate_token(
    stored: &StoredLease,
    owner_id: &str,
    epoch: i64,
    now: DateTime<Utc>,
) -> Result<(), StorePrimaryLeaseError> {
    if stored.epoch != epoch {
        return Err(StorePrimaryLeaseError::EpochMismatch);
    }
    if stored.owner_id != owner_id {
        return Err(StorePrimaryLeaseError::OwnerMismatch);
    }
    if stored.expires_at <= now {
        return Err(StorePrimaryLeaseError::Expired);
    }
    Ok(())
}

fn is_fencing_loss(error: &StorePrimaryLeaseError) -> bool {
    matches!(
        error,
        StorePrimaryLeaseError::Unavailable
            | StorePrimaryLeaseError::Missing
            | StorePrimaryLeaseError::OwnerMismatch
            | StorePrimaryLeaseError::EpochMismatch
            | StorePrimaryLeaseError::Expired
            | StorePrimaryLeaseError::RenewalFailed
            | StorePrimaryLeaseError::EpochOverflow
            | StorePrimaryLeaseError::InvalidOwner
    )
}

fn failure_kind(error: &StorePrimaryLeaseError) -> String {
    match error {
        StorePrimaryLeaseError::Storage(_) => "storage".to_string(),
        StorePrimaryLeaseError::Unavailable => "unavailable".to_string(),
        StorePrimaryLeaseError::Missing => "missing".to_string(),
        StorePrimaryLeaseError::OwnerMismatch => "owner_mismatch".to_string(),
        StorePrimaryLeaseError::EpochMismatch => "epoch_mismatch".to_string(),
        StorePrimaryLeaseError::Expired => "expired".to_string(),
        StorePrimaryLeaseError::RenewalFailed => "renewal_failed".to_string(),
        StorePrimaryLeaseError::InvalidOwner => "invalid_owner".to_string(),
        StorePrimaryLeaseError::EpochOverflow => "epoch_overflow".to_string(),
    }
}

async fn finish_transaction<T>(
    transaction: crate::db::WriteTransaction,
    outcome: Result<T, StorePrimaryLeaseError>,
) -> Result<T, StorePrimaryLeaseError> {
    match outcome {
        Ok(value) => {
            transaction.commit().await?;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().await?;
            Err(error)
        }
    }
}

fn canonical_time(value: DateTime<Utc>) -> Result<DateTime<Utc>, StorePrimaryLeaseError> {
    DateTime::from_timestamp_micros(value.timestamp_micros())
        .ok_or_else(|| StorePrimaryLeaseError::Storage("lease time is invalid".to_string()))
}

fn format_time(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn is_unique_conflict(error: &DbErr) -> bool {
    matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration as StdDuration;

    use sea_orm_migration::MigratorTrait;

    use super::{
        StorePrimaryLease, StorePrimaryLeaseError, lease_select_sql, renew_with_retry,
        request_shutdown_after_renewal_failure, wait_for_background_shutdown,
    };
    use crate::db::DbPool;
    use crate::migration::Migrator;

    #[test]
    fn postgres_mutations_lock_the_lease_row() {
        assert!(lease_select_sql(true).ends_with("FOR UPDATE"));
        assert!(!lease_select_sql(false).contains("FOR UPDATE"));
    }

    #[test]
    fn terminal_renewal_failure_requests_application_shutdown() {
        let lease_lost = AtomicBool::new(false);
        let shutdown = AtomicBool::new(false);

        request_shutdown_after_renewal_failure(&lease_lost, &shutdown);

        assert!(lease_lost.load(Ordering::Acquire));
        assert!(shutdown.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn background_shutdown_waiter_observes_an_existing_request() {
        let shutdown = Arc::new(AtomicBool::new(true));

        tokio::time::timeout(
            std::time::Duration::from_millis(250),
            wait_for_background_shutdown(shutdown),
        )
        .await
        .expect("shutdown waiter must return within the specified bound");
    }

    #[tokio::test]
    async fn renewal_round_times_out_while_the_sqlite_writer_is_stalled() {
        let db = DbPool::connect("sqlite::memory:").await.expect("database");
        Migrator::up(&*db.write().await, None)
            .await
            .expect("migrations");
        let lease = StorePrimaryLease::acquire(db.clone(), "owner-a")
            .await
            .expect("lease");
        let _blocked_writer = db.begin_write().await.expect("blocking transaction");
        let shutdown = AtomicBool::new(false);

        let result = tokio::time::timeout(
            StdDuration::from_millis(2_500),
            renew_with_retry(&lease, &shutdown),
        )
        .await
        .expect("renewal round must have its own timeout");

        assert_eq!(result.unwrap_err(), StorePrimaryLeaseError::RenewalFailed);
    }
}
