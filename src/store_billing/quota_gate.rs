use crate::db::DbPool;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DbErr, QueryResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const QUOTA_COMPATIBILITY_ID: &str = "store-plan-quota-v1";
const QUOTA_SCHEMA_VERSION: i64 = 1;
const QUOTA_SCHEMA_MIGRATIONS: [&str; 2] = [
    "m20260827_000049_store_billing",
    "m20260827_000051_store_payment_core",
];
const QUOTA_COMPATIBILITY_MANIFEST: &[u8] = b"compatibility_id=store-plan-quota-v1\nschema_version=1\nrequired_migrations=m20260827_000049_store_billing,m20260827_000051_store_payment_core\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateSlot {
    Current,
    Next,
}

impl GateSlot {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Next => "next",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaGateState {
    Pending,
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaEnvironment {
    pub compatibility_id: String,
    pub schema_version: i64,
    pub sqlite_version: String,
    pub journal_mode: String,
    pub busy_timeout_ms: i64,
    pub page_size: i64,
    pub synchronous: String,
    pub filesystem_id: String,
    pub quota_manifest_digest: String,
}

impl QuotaEnvironment {
    pub fn compatibility_fingerprint(&self) -> Result<String, QuotaGateError> {
        if self.compatibility_id.trim().is_empty()
            || self.schema_version <= 0
            || self.sqlite_version.trim().is_empty()
            || (self.journal_mode != "wal"
                && !(self.journal_mode == "memory" && self.filesystem_id.starts_with("memory:")))
            || self.busy_timeout_ms != 5_000
            || self.page_size <= 0
            || self.synchronous.trim().is_empty()
            || self.filesystem_id.trim().is_empty()
            || self.quota_manifest_digest.trim().is_empty()
        {
            return Err(QuotaGateError::InvalidManifest);
        }
        let canonical = serde_json::to_vec(self).map_err(QuotaGateError::Serialize)?;
        Ok(hex_digest(Sha256::digest(canonical)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaManifest {
    pub environment: QuotaEnvironment,
    pub application_version: String,
    pub drill_result_digest: String,
    pub measured_at: DateTime<Utc>,
    pub imported_by: String,
    pub compatibility_fingerprint: String,
}

impl QuotaManifest {
    pub fn passed(
        environment: QuotaEnvironment,
        application_version: impl Into<String>,
        drill_result_digest: impl Into<String>,
        measured_at: DateTime<Utc>,
        imported_by: impl Into<String>,
    ) -> Result<Self, QuotaGateError> {
        let application_version = application_version.into();
        let drill_result_digest = drill_result_digest.into();
        let imported_by = imported_by.into();
        if application_version.trim().is_empty()
            || drill_result_digest.trim().is_empty()
            || imported_by.trim().is_empty()
        {
            return Err(QuotaGateError::InvalidManifest);
        }
        let compatibility_fingerprint = environment.compatibility_fingerprint()?;
        Ok(Self {
            environment,
            application_version,
            drill_result_digest,
            measured_at,
            imported_by,
            compatibility_fingerprint,
        })
    }
}

#[derive(Debug, Error)]
pub enum QuotaGateError {
    #[error("quota manifest is invalid")]
    InvalidManifest,
    #[error("quota Gate manifest conflicts with the expected fingerprint")]
    FingerprintConflict,
    #[error("quota Gate storage failed: {0}")]
    Storage(String),
    #[error("quota Gate manifest serialization failed: {0}")]
    Serialize(serde_json::Error),
}

impl From<DbErr> for QuotaGateError {
    fn from(value: DbErr) -> Self {
        storage(value)
    }
}

#[derive(Debug, Clone)]
pub struct QuotaGateStore {
    db: DbPool,
}

impl QuotaGateStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn import_manifest(
        &self,
        slot: GateSlot,
        manifest: QuotaManifest,
    ) -> Result<(), QuotaGateError> {
        let write = self.db.write().await;
        self.import_manifest_on(&*write, slot, manifest).await
    }

    pub async fn import_matching_manifest(
        &self,
        slot: GateSlot,
        manifest: QuotaManifest,
    ) -> Result<(), QuotaGateError> {
        if !self.db.is_sqlite() {
            return Err(QuotaGateError::Storage(
                "offline quota Gate import requires SQLite".to_string(),
            ));
        }
        let db = self.db.clone();
        self.db
            .with_sqlite_quota_probe(move |connection| {
                Box::pin(async move {
                    let store = QuotaGateStore::new(db);
                    let live = store.live_environment_on(connection).await?;
                    if live != manifest.environment {
                        return Err(QuotaGateError::FingerprintConflict);
                    }
                    store.import_manifest_on(connection, slot, manifest).await
                })
            })
            .await
    }

    async fn import_manifest_on<C: ConnectionTrait>(
        &self,
        connection: &C,
        slot: GateSlot,
        manifest: QuotaManifest,
    ) -> Result<(), QuotaGateError> {
        let expected = manifest.environment.compatibility_fingerprint()?;
        if expected != manifest.compatibility_fingerprint {
            return Err(QuotaGateError::FingerprintConflict);
        }
        let json = serde_json::to_string(&manifest).map_err(QuotaGateError::Serialize)?;
        connection
            .execute(self.db.stmt(
                "INSERT INTO store_quota_gates
                    (backend, slot, state, compatibility_fingerprint, manifest_json,
                     tested_at, failure_reason, updated_at)
                 VALUES ('sqlite', $1, 'passed', $2, $3, $4, NULL, $4)
                 ON CONFLICT (backend, slot) DO UPDATE SET
                    state = excluded.state,
                    compatibility_fingerprint = excluded.compatibility_fingerprint,
                    manifest_json = excluded.manifest_json,
                    tested_at = excluded.tested_at,
                    failure_reason = NULL,
                    updated_at = excluded.updated_at",
                vec![
                    slot.as_str().into(),
                    manifest.compatibility_fingerprint.into(),
                    json.into(),
                    manifest.measured_at.to_rfc3339().into(),
                ],
            ))
            .await
            .map_err(storage)?;
        Ok(())
    }

    pub async fn record_failure(
        &self,
        slot: GateSlot,
        environment: QuotaEnvironment,
        failure_digest: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<(), QuotaGateError> {
        let fingerprint = environment.compatibility_fingerprint()?;
        let failure_digest = failure_digest.into();
        if failure_digest.trim().is_empty() {
            return Err(QuotaGateError::InvalidManifest);
        }
        let manifest_json =
            serde_json::to_string(&environment).map_err(QuotaGateError::Serialize)?;
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "INSERT INTO store_quota_gates
                    (backend, slot, state, compatibility_fingerprint, manifest_json,
                     tested_at, failure_reason, updated_at)
                 VALUES ('sqlite', $1, 'failed', $2, $3, $4, $5, $4)
                 ON CONFLICT (backend, slot) DO UPDATE SET
                    state = excluded.state,
                    compatibility_fingerprint = excluded.compatibility_fingerprint,
                    manifest_json = excluded.manifest_json,
                    tested_at = excluded.tested_at,
                    failure_reason = excluded.failure_reason,
                    updated_at = excluded.updated_at",
                vec![
                    slot.as_str().into(),
                    fingerprint.into(),
                    manifest_json.into(),
                    now.to_rfc3339().into(),
                    failure_digest.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        Ok(())
    }

    pub async fn effective_state(
        &self,
        environment: &QuotaEnvironment,
    ) -> Result<QuotaGateState, QuotaGateError> {
        let expected = environment.compatibility_fingerprint()?;
        let Some(row) = self.load_slot(GateSlot::Current).await? else {
            return Ok(QuotaGateState::Pending);
        };
        if row.compatibility_fingerprint != expected {
            return Ok(QuotaGateState::Pending);
        }
        Ok(row.state)
    }

    pub async fn plan_features_enabled(&self) -> Result<bool, QuotaGateError> {
        if !self.db.is_sqlite() {
            return Ok(true);
        }
        let db = self.db.clone();
        self.db
            .with_sqlite_quota_probe(move |connection| {
                Box::pin(async move {
                    QuotaGateStore::new(db)
                        .plan_features_enabled_on(connection)
                        .await
                })
            })
            .await
    }

    pub async fn live_environment(&self) -> Result<QuotaEnvironment, QuotaGateError> {
        if !self.db.is_sqlite() {
            return Err(QuotaGateError::Storage(
                "live quota environment requires SQLite".to_string(),
            ));
        }
        let db = self.db.clone();
        self.db
            .with_sqlite_quota_probe(move |connection| {
                Box::pin(async move {
                    QuotaGateStore::new(db)
                        .live_environment_on(connection)
                        .await
                })
            })
            .await
    }

    pub(crate) async fn plan_features_enabled_on<C: ConnectionTrait>(
        &self,
        connection: &C,
    ) -> Result<bool, QuotaGateError> {
        let Some(row) = self.load_slot_on(connection, GateSlot::Current).await? else {
            return Ok(false);
        };
        if row.state != QuotaGateState::Passed {
            return Ok(false);
        }
        let manifest: QuotaManifest =
            serde_json::from_str(&row.manifest_json).map_err(QuotaGateError::Serialize)?;
        let environment_fingerprint = manifest.environment.compatibility_fingerprint()?;
        let live = match self.live_environment_on(connection).await {
            Ok(live) => live,
            Err(_) => return Ok(false),
        };
        let live_fingerprint = live.compatibility_fingerprint()?;
        Ok(!row.compatibility_fingerprint.is_empty()
            && manifest.compatibility_fingerprint == environment_fingerprint
            && row.compatibility_fingerprint == environment_fingerprint
            && live == manifest.environment
            && live_fingerprint == environment_fingerprint)
    }

    pub async fn promote_next(&self, expected_fingerprint: &str) -> Result<(), QuotaGateError> {
        let live_environment = if self.db.is_sqlite() {
            let environment = self.live_environment().await?;
            let live_fingerprint = environment.compatibility_fingerprint()?;
            if live_fingerprint != expected_fingerprint {
                return Err(QuotaGateError::FingerprintConflict);
            }
            Some(environment)
        } else {
            None
        };
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        let row = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT state, compatibility_fingerprint, manifest_json, tested_at
                     FROM store_quota_gates
                     WHERE backend = 'sqlite' AND slot = 'next'{lock}"
                ),
                vec![],
            ))
            .await
            .map_err(storage)?
            .ok_or(QuotaGateError::FingerprintConflict)?;
        let state = row_string(&row, "state")?;
        let fingerprint = row_string(&row, "compatibility_fingerprint")?;
        if state != "passed" || fingerprint != expected_fingerprint {
            return Err(QuotaGateError::FingerprintConflict);
        }
        let manifest_json = row_string(&row, "manifest_json")?;
        if let Some(live_environment) = live_environment {
            let manifest: QuotaManifest =
                serde_json::from_str(&manifest_json).map_err(QuotaGateError::Serialize)?;
            if manifest.environment != live_environment {
                return Err(QuotaGateError::FingerprintConflict);
            }
        }
        let tested_at = row_optional_string(&row, "tested_at")?;
        let updated_at = Utc::now().to_rfc3339();
        tx.execute(self.db.stmt(
            "INSERT INTO store_quota_gates
                (backend, slot, state, compatibility_fingerprint, manifest_json,
                 tested_at, failure_reason, updated_at)
             VALUES ('sqlite', 'current', 'passed', $1, $2, $3, NULL, $4)
             ON CONFLICT (backend, slot) DO UPDATE SET
                state = excluded.state,
                compatibility_fingerprint = excluded.compatibility_fingerprint,
                manifest_json = excluded.manifest_json,
                tested_at = excluded.tested_at,
                failure_reason = NULL,
                updated_at = excluded.updated_at",
            vec![
                fingerprint.into(),
                manifest_json.into(),
                tested_at.into(),
                updated_at.clone().into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_quota_gates
             SET state = 'pending', compatibility_fingerprint = '', manifest_json = '{}',
                 tested_at = NULL, failure_reason = NULL, updated_at = $1
             WHERE backend = 'sqlite' AND slot = 'next'",
            vec![updated_at.into()],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    pub async fn current_manifest(&self) -> Result<Option<QuotaManifest>, QuotaGateError> {
        let Some(row) = self.load_slot(GateSlot::Current).await? else {
            return Ok(None);
        };
        if row.state != QuotaGateState::Passed {
            return Ok(None);
        }
        serde_json::from_str(&row.manifest_json)
            .map(Some)
            .map_err(QuotaGateError::Serialize)
    }

    async fn live_environment_on<C: ConnectionTrait>(
        &self,
        connection: &C,
    ) -> Result<QuotaEnvironment, QuotaGateError> {
        let migration_count = connection
            .query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM seaql_migrations
                 WHERE version IN ($1, $2)",
                vec![
                    QUOTA_SCHEMA_MIGRATIONS[0].into(),
                    QUOTA_SCHEMA_MIGRATIONS[1].into(),
                ],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| QuotaGateError::Storage("quota schema query returned no row".into()))?
            .try_get::<i64>("", "value")
            .map_err(storage)?;
        if migration_count != QUOTA_SCHEMA_MIGRATIONS.len() as i64 {
            return Err(QuotaGateError::InvalidManifest);
        }

        let sqlite_version = scalar_string(
            &self.db,
            connection,
            "SELECT sqlite_version() AS value",
            "value",
        )
        .await?;
        let journal_mode =
            scalar_string(&self.db, connection, "PRAGMA journal_mode", "journal_mode")
                .await?
                .to_ascii_lowercase();
        let busy_timeout_ms =
            scalar_i64(&self.db, connection, "PRAGMA busy_timeout", "timeout").await?;
        let page_size = scalar_i64(&self.db, connection, "PRAGMA page_size", "page_size").await?;
        let synchronous =
            match scalar_i64(&self.db, connection, "PRAGMA synchronous", "synchronous").await? {
                0 => "off",
                1 => "normal",
                2 => "full",
                3 => "extra",
                _ => return Err(QuotaGateError::InvalidManifest),
            }
            .to_string();
        let filesystem_id = self
            .db
            .sqlite_filesystem_id()
            .ok_or(QuotaGateError::InvalidManifest)?
            .to_string();
        let environment = QuotaEnvironment {
            compatibility_id: QUOTA_COMPATIBILITY_ID.to_string(),
            schema_version: QUOTA_SCHEMA_VERSION,
            sqlite_version,
            journal_mode,
            busy_timeout_ms,
            page_size,
            synchronous,
            filesystem_id,
            quota_manifest_digest: hex_digest(Sha256::digest(QUOTA_COMPATIBILITY_MANIFEST)),
        };
        environment.compatibility_fingerprint()?;
        Ok(environment)
    }

    async fn load_slot(&self, slot: GateSlot) -> Result<Option<StoredGate>, QuotaGateError> {
        self.load_slot_on(self.db.read(), slot).await
    }

    async fn load_slot_on<C: ConnectionTrait>(
        &self,
        connection: &C,
        slot: GateSlot,
    ) -> Result<Option<StoredGate>, QuotaGateError> {
        connection
            .query_one(self.db.stmt(
                "SELECT state, compatibility_fingerprint, manifest_json
                 FROM store_quota_gates WHERE backend = 'sqlite' AND slot = $1",
                vec![slot.as_str().into()],
            ))
            .await
            .map_err(storage)?
            .map(stored_gate)
            .transpose()
    }
}

async fn scalar_string<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    sql: &str,
    column: &str,
) -> Result<String, QuotaGateError> {
    connection
        .query_one(db.stmt(sql, vec![]))
        .await
        .map_err(storage)?
        .ok_or_else(|| QuotaGateError::Storage(format!("{sql} returned no row")))?
        .try_get("", column)
        .map_err(storage)
}

async fn scalar_i64<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    sql: &str,
    column: &str,
) -> Result<i64, QuotaGateError> {
    connection
        .query_one(db.stmt(sql, vec![]))
        .await
        .map_err(storage)?
        .ok_or_else(|| QuotaGateError::Storage(format!("{sql} returned no row")))?
        .try_get("", column)
        .map_err(storage)
}

struct StoredGate {
    state: QuotaGateState,
    compatibility_fingerprint: String,
    manifest_json: String,
}

fn stored_gate(row: QueryResult) -> Result<StoredGate, QuotaGateError> {
    let state = match row_string(&row, "state")?.as_str() {
        "pending" => QuotaGateState::Pending,
        "passed" => QuotaGateState::Passed,
        "failed" => QuotaGateState::Failed,
        _ => {
            return Err(QuotaGateError::Storage(
                "invalid quota Gate state".to_string(),
            ));
        }
    };
    Ok(StoredGate {
        state,
        compatibility_fingerprint: row_string(&row, "compatibility_fingerprint")?,
        manifest_json: row_string(&row, "manifest_json")?,
    })
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, QuotaGateError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_string(row: &QueryResult, column: &str) -> Result<Option<String>, QuotaGateError> {
    row.try_get("", column).map_err(storage)
}

fn storage(error: impl std::fmt::Display) -> QuotaGateError {
    QuotaGateError::Storage(error.to_string())
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
