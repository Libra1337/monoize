use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, DbErr, QueryResult, SqlErr};
use thiserror::Error;

use crate::db::DbPool;

pub const STORE_PRIMARY_LEASE_NAME: &str = "store_primary";
pub const STORE_PRIMARY_LEASE_SECONDS: i64 = 15;
pub const STORE_PRIMARY_RENEWAL_SECONDS: u64 = 5;
const STORE_PRIMARY_RENEWAL_SAFETY_SECONDS: i64 = 5;
const STORE_PRIMARY_RETRY_DELAYS_MS: [u64; 3] = [100, 250, 500];

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
        }
        if outcome.as_ref().is_err_and(|error| is_fencing_loss(error)) {
            self.renewal_failed.store(true, Ordering::Release);
        }
        outcome
    }

    async fn remaining_ttl(&self) -> Duration {
        *self.expires_at.read().await - Utc::now()
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
                    lease.renewal_failed.store(true, Ordering::Release);
                    tracing::error!(error = %error, "Store Primary lease renewal failed; lease marked lost");
                    break;
                }
            }
        });
    }
}

async fn renew_with_retry(
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
    use super::lease_select_sql;

    #[test]
    fn postgres_mutations_lock_the_lease_row() {
        assert!(lease_select_sql(true).ends_with("FOR UPDATE"));
        assert!(!lease_select_sql(false).contains("FOR UPDATE"));
    }
}
