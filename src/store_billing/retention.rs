use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, SecondsFormat, Timelike, Utc};
use sea_orm::{ConnectionTrait, DbErr, QueryResult, Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::availability::StorePrimaryLease;
use super::models::StorePrivacyRetention;
use crate::db::DbPool;

pub const RETENTION_BATCH_SIZE: i64 = 500;
const NETWORK_METADATA_RETENTION_DAYS: i64 = 90;
const RETENTION_SYSTEM_ACTOR: &str = "_monoize_retention_job";
const RETENTION_SCHEDULER_HOUR_UTC: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreRetentionRunState {
    Running,
    Succeeded,
    Failed,
}

impl StoreRetentionRunState {
    fn from_str(value: &str) -> Result<Self, StoreRetentionError> {
        match value {
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            _ => Err(storage("stored retention run state is invalid")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreRetentionDataClass {
    RawCallbackBodies,
    NetworkMetadata,
    FinancialRecords,
    RedemptionAudits,
    ExpiredReauthGrants,
}

impl StoreRetentionDataClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RawCallbackBodies => "raw_callback_bodies",
            Self::NetworkMetadata => "network_metadata",
            Self::FinancialRecords => "financial_records",
            Self::RedemptionAudits => "redemption_audits",
            Self::ExpiredReauthGrants => "expired_reauth_grants",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreRetentionCounts {
    pub raw_callback_bodies: u64,
    pub network_metadata: u64,
    pub financial_records: u64,
    pub redemption_audits: u64,
    pub expired_reauth_grants: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreRetentionRun {
    pub id: String,
    pub worker_owner_id: String,
    pub policy_version: String,
    pub counts: StoreRetentionCounts,
    pub oldest_remaining_at: Option<DateTime<Utc>>,
    pub state: StoreRetentionRunState,
    pub error_category: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreRetentionAlert {
    pub id: String,
    pub run_id: String,
    pub severity: String,
    pub consecutive_failures: u64,
    pub created_at: DateTime<Utc>,
    pub contained_at: Option<DateTime<Utc>>,
    pub containment_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreRetentionContainment {
    pub id: String,
    pub alert_id: String,
    pub actor_id: String,
    pub reason: String,
    pub evidence_digest: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreRetentionStatus {
    pub current_run_id: Option<String>,
    pub last_run_id: Option<String>,
    pub consecutive_failures: u64,
    pub checkout_paused: bool,
    pub active_alert: Option<StoreRetentionAlert>,
    pub latest_containment_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreLegalHold {
    pub id: String,
    pub data_class: StoreRetentionDataClass,
    pub identifiers: Vec<String>,
    pub reason: String,
    pub requesting_authority: String,
    pub requester_id: String,
    pub approver_id: String,
    pub approver_role: String,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub extends_hold_id: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateStoreLegalHoldInput {
    pub data_class: StoreRetentionDataClass,
    pub identifiers: Vec<String>,
    pub reason: String,
    pub requesting_authority: String,
    pub requester_id: String,
    pub approver_role: String,
    pub expires_at: DateTime<Utc>,
    pub extends_hold_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunStoreRetentionInput {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateStoreRetentionContainmentInput {
    pub reason: String,
    pub evidence_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreRetentionOverview {
    pub status: StoreRetentionStatus,
    pub runs: Vec<StoreRetentionRun>,
    pub holds: Vec<StoreLegalHold>,
    pub containments: Vec<StoreRetentionContainment>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreRetentionError {
    #[error("retention input is invalid")]
    InvalidInput,
    #[error("a retention run is already active")]
    RunActive,
    #[error("retention containment is unavailable")]
    ContainmentUnavailable,
    #[error("retention record was not found")]
    NotFound,
    #[error("retention storage failed: {0}")]
    Storage(String),
}

impl From<DbErr> for StoreRetentionError {
    fn from(error: DbErr) -> Self {
        Self::Storage(error.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct StoreRetention {
    db: DbPool,
    worker_owner_id: Arc<str>,
}

#[derive(Debug, Clone)]
pub struct RetentionRunActor {
    pub actor_id: String,
    pub actor_role: String,
    pub reason: String,
}

impl RetentionRunActor {
    pub fn scheduled() -> Self {
        Self {
            actor_id: RETENTION_SYSTEM_ACTOR.to_string(),
            actor_role: "system".to_string(),
            reason: "scheduled_retention".to_string(),
        }
    }
}

enum PolicyLookup {
    Current {
        version: String,
        retention: StorePrivacyRetention,
    },
    Invalid {
        version: String,
    },
    Unavailable,
}

impl PolicyLookup {
    fn version(&self) -> &str {
        match self {
            Self::Current { version, .. } | Self::Invalid { version } => version,
            Self::Unavailable => "unavailable",
        }
    }
}

impl StoreRetention {
    pub fn new(db: DbPool, worker_owner_id: impl Into<String>) -> Self {
        Self {
            db,
            worker_owner_id: worker_owner_id.into().into(),
        }
    }

    pub async fn run_now(
        &self,
        actor: RetentionRunActor,
    ) -> Result<StoreRetentionRun, StoreRetentionError> {
        self.run_at(Utc::now(), actor).await
    }

    pub async fn run_at(
        &self,
        now: DateTime<Utc>,
        actor: RetentionRunActor,
    ) -> Result<StoreRetentionRun, StoreRetentionError> {
        let now = canonical_time(now)?;
        validate_actor(&actor)?;
        let policy = self.current_policy(now).await?;
        let run_id = self.claim_run(now, policy.version()).await?;
        let outcome = match policy {
            PolicyLookup::Current { retention, .. } => {
                if !valid_retention(&retention) {
                    Err("privacy_policy_invalid")
                } else {
                    self.execute_success(&run_id, now, &retention, &actor)
                        .await
                        .map_err(|_| "storage")
                }
            }
            PolicyLookup::Invalid { .. } => Err("privacy_policy_invalid"),
            PolicyLookup::Unavailable => Err("privacy_policy_unavailable"),
        };
        if let Err(category) = outcome {
            self.finalize_failure(&run_id, now, category, &actor)
                .await?;
        }
        self.run_by_id(&run_id)
            .await?
            .ok_or_else(|| storage("completed retention run is missing"))
    }

    pub async fn overview(&self) -> Result<StoreRetentionOverview, StoreRetentionError> {
        let now = Utc::now();
        Ok(StoreRetentionOverview {
            status: self.status().await?,
            runs: self.list_runs(100).await?,
            holds: self.list_legal_holds(now, 100).await?,
            containments: self.list_containments(100).await?,
        })
    }

    pub async fn status(&self) -> Result<StoreRetentionStatus, StoreRetentionError> {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT current_run_id, last_run_id, consecutive_failures, checkout_paused,
                        active_alert_id, latest_containment_id, updated_at
                 FROM store_retention_state WHERE singleton_id = 1",
                vec![],
            ))
            .await?
            .ok_or_else(|| storage("retention state is missing"))?;
        let active_alert_id = optional_string(&row, "active_alert_id")?;
        Ok(StoreRetentionStatus {
            current_run_id: optional_string(&row, "current_run_id")?,
            last_run_id: optional_string(&row, "last_run_id")?,
            consecutive_failures: nonnegative_u64(&row, "consecutive_failures")?,
            checkout_paused: boolean(&row, "checkout_paused")?,
            active_alert: match active_alert_id {
                Some(id) => self.alert_by_id(&id).await?,
                None => None,
            },
            latest_containment_id: optional_string(&row, "latest_containment_id")?,
            updated_at: parse_time(&string(&row, "updated_at")?)?,
        })
    }

    pub async fn list_runs(
        &self,
        limit: u64,
    ) -> Result<Vec<StoreRetentionRun>, StoreRetentionError> {
        self.db
            .read()
            .query_all(self.db.stmt(
                "SELECT id, worker_owner_id, policy_version, counts_json,
                        oldest_remaining_at, state, error_category, started_at, completed_at
                 FROM store_retention_runs
                 ORDER BY started_at DESC, id DESC LIMIT $1",
                vec![(limit.min(100) as i64).into()],
            ))
            .await?
            .into_iter()
            .map(run_from_row)
            .collect()
    }

    pub async fn create_legal_hold(
        &self,
        mut input: CreateStoreLegalHoldInput,
        approver_id: &str,
        now: DateTime<Utc>,
    ) -> Result<StoreLegalHold, StoreRetentionError> {
        let now = canonical_time(now)?;
        validate_hold_input(&input, approver_id, now)?;
        input.identifiers.sort();
        if input.identifiers.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(StoreRetentionError::InvalidInput);
        }

        let tx = self.db.begin_write().await?;
        let outcome = async {
            if let Some(extended_id) = input.extends_hold_id.as_deref() {
                let previous = legal_hold_by_id(&self.db, &*tx, extended_id, now)
                    .await?
                    .ok_or(StoreRetentionError::InvalidInput)?;
                if previous.data_class != input.data_class
                    || previous.identifiers != input.identifiers
                    || input.expires_at <= previous.expires_at
                {
                    return Err(StoreRetentionError::InvalidInput);
                }
            }
            let id = Uuid::new_v4().to_string();
            let identifiers_json = serde_json::to_string(&input.identifiers).map_err(storage)?;
            tx.execute(self.db.stmt(
                "INSERT INTO store_legal_holds
                    (id, data_class, identifiers_json, reason, requesting_authority,
                     approver_id, starts_at, expires_at, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $7)",
                vec![
                    id.clone().into(),
                    input.data_class.as_str().into(),
                    identifiers_json.into(),
                    input.reason.clone().into(),
                    input.requesting_authority.clone().into(),
                    approver_id.into(),
                    timestamp(now).into(),
                    timestamp(input.expires_at).into(),
                ],
            ))
            .await?;
            tx.execute(self.db.stmt(
                "INSERT INTO store_legal_hold_approvals
                    (hold_id, requester_id, approver_role, extends_hold_id)
                 VALUES ($1, $2, $3, $4)",
                vec![
                    id.clone().into(),
                    input.requester_id.clone().into(),
                    input.approver_role.clone().into(),
                    input.extends_hold_id.clone().into(),
                ],
            ))
            .await?;
            for identifier in &input.identifiers {
                tx.execute(self.db.stmt(
                    "INSERT INTO store_legal_hold_items (hold_id, data_class, identifier)
                     VALUES ($1, $2, $3)",
                    vec![
                        id.clone().into(),
                        input.data_class.as_str().into(),
                        identifier.clone().into(),
                    ],
                ))
                .await?;
            }
            insert_access_audit(
                &self.db,
                &*tx,
                approver_id,
                &input.approver_role,
                "legal_hold_create",
                serde_json::json!({
                    "hold_id": id,
                    "data_class": input.data_class,
                    "identifiers": input.identifiers,
                    "requester_id": input.requester_id,
                    "requesting_authority": input.requesting_authority,
                    "extends_hold_id": input.extends_hold_id,
                }),
                &input.reason,
                "succeeded",
                now,
            )
            .await?;
            Ok(id)
        }
        .await;
        let id = finish_transaction(tx, outcome).await?;
        self.legal_hold(&id, now)
            .await?
            .ok_or_else(|| storage("created legal hold is missing"))
    }

    pub async fn list_legal_holds(
        &self,
        now: DateTime<Utc>,
        limit: u64,
    ) -> Result<Vec<StoreLegalHold>, StoreRetentionError> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                &format!(
                    "{} ORDER BY h.created_at DESC, h.id DESC LIMIT $1",
                    legal_hold_select()
                ),
                vec![(limit.min(100) as i64).into()],
            ))
            .await?;
        rows.into_iter()
            .map(|row| legal_hold_from_row(row, now))
            .collect()
    }

    pub async fn contain(
        &self,
        input: CreateStoreRetentionContainmentInput,
        actor_id: &str,
        now: DateTime<Utc>,
    ) -> Result<StoreRetentionContainment, StoreRetentionError> {
        let now = canonical_time(now)?;
        if !valid_nonblank_text(&input.reason, 2000)
            || !valid_digest(&input.evidence_digest)
            || !valid_identifier(actor_id)
        {
            return Err(StoreRetentionError::InvalidInput);
        }
        let tx = self.db.begin_write().await?;
        let outcome = async {
            let lock = if self.db.is_postgres() {
                " FOR UPDATE"
            } else {
                ""
            };
            let state = tx
                .query_one(self.db.stmt(
                    &format!(
                        "SELECT checkout_paused, active_alert_id
                         FROM store_retention_state WHERE singleton_id = 1{lock}"
                    ),
                    vec![],
                ))
                .await?
                .ok_or_else(|| storage("retention state is missing"))?;
            let paused = boolean(&state, "checkout_paused")?;
            let alert_id = optional_string(&state, "active_alert_id")?
                .filter(|_| paused)
                .ok_or(StoreRetentionError::ContainmentUnavailable)?;
            let id = Uuid::new_v4().to_string();
            tx.execute(self.db.stmt(
                "INSERT INTO store_retention_containments
                    (id, alert_id, actor_id, reason, evidence_digest, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                vec![
                    id.clone().into(),
                    alert_id.clone().into(),
                    actor_id.into(),
                    input.reason.clone().into(),
                    input.evidence_digest.clone().into(),
                    timestamp(now).into(),
                ],
            ))
            .await?;
            let changed = tx
                .execute(self.db.stmt(
                    "UPDATE store_retention_alerts
                     SET contained_at = $2, containment_id = $3
                     WHERE id = $1 AND contained_at IS NULL",
                    vec![
                        alert_id.clone().into(),
                        timestamp(now).into(),
                        id.clone().into(),
                    ],
                ))
                .await?;
            if changed.rows_affected() != 1 {
                return Err(StoreRetentionError::ContainmentUnavailable);
            }
            tx.execute(self.db.stmt(
                "UPDATE store_retention_state
                 SET checkout_paused = 0, active_alert_id = NULL,
                     latest_containment_id = $1, updated_at = $2
                 WHERE singleton_id = 1",
                vec![id.clone().into(), timestamp(now).into()],
            ))
            .await?;
            insert_access_audit(
                &self.db,
                &*tx,
                actor_id,
                "admin",
                "retention_containment",
                serde_json::json!({"alert_id": alert_id, "containment_id": id}),
                &input.reason,
                "succeeded",
                now,
            )
            .await?;
            Ok(StoreRetentionContainment {
                id,
                alert_id,
                actor_id: actor_id.to_string(),
                reason: input.reason.clone(),
                evidence_digest: input.evidence_digest.clone(),
                created_at: now,
            })
        }
        .await;
        finish_transaction(tx, outcome).await
    }

    async fn current_policy(
        &self,
        now: DateTime<Utc>,
    ) -> Result<PolicyLookup, StoreRetentionError> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT policy_version, retention_json, approved_at, next_review_at
                 FROM store_privacy_records WHERE accepted = 1
                 ORDER BY approved_at DESC, id DESC",
                vec![],
            ))
            .await?;
        for row in rows {
            let version = string(&row, "policy_version")?;
            let approved = match parse_time(&string(&row, "approved_at")?) {
                Ok(value) => value,
                Err(_) => return Ok(PolicyLookup::Invalid { version }),
            };
            let review = match parse_time(&string(&row, "next_review_at")?) {
                Ok(value) => value,
                Err(_) => return Ok(PolicyLookup::Invalid { version }),
            };
            if approved <= now && now < review {
                return match serde_json::from_str(&string(&row, "retention_json")?) {
                    Ok(retention) => Ok(PolicyLookup::Current { version, retention }),
                    Err(_) => Ok(PolicyLookup::Invalid { version }),
                };
            }
        }
        Ok(PolicyLookup::Unavailable)
    }

    async fn claim_run(
        &self,
        now: DateTime<Utc>,
        policy_version: &str,
    ) -> Result<String, StoreRetentionError> {
        let run_id = Uuid::new_v4().to_string();
        if self.db.is_sqlite() {
            let db = self.db.clone();
            let owner = self.worker_owner_id.to_string();
            let policy = policy_version.to_string();
            let run = run_id.clone();
            self.db
                .with_immediate_write(move |connection| {
                    Box::pin(async move {
                        claim_run_locked(&db, connection, &run, &owner, &policy, now, false).await
                    })
                })
                .await?;
        } else {
            let tx = self.db.begin_write().await?;
            let outcome = claim_run_locked(
                &self.db,
                &*tx,
                &run_id,
                &self.worker_owner_id,
                policy_version,
                now,
                true,
            )
            .await;
            finish_transaction(tx, outcome).await?;
        }
        Ok(run_id)
    }

    async fn execute_success(
        &self,
        run_id: &str,
        now: DateTime<Utc>,
        policy: &StorePrivacyRetention,
        actor: &RetentionRunActor,
    ) -> Result<(), StoreRetentionError> {
        let tx = self.db.begin_write().await?;
        let run_id = run_id.to_string();
        let worker_owner_id = self.worker_owner_id.to_string();
        let outcome = async {
            let counts = apply_retention(&self.db, &*tx, now, policy).await?;
            let oldest_remaining_at = oldest_remaining(&self.db, &*tx).await?;
            let counts_json = serde_json::to_string(&counts).map_err(storage)?;
            let changed = tx
                .execute(self.db.stmt(
                    "UPDATE store_retention_runs
                     SET counts_json = $3, oldest_remaining_at = $4, state = 'succeeded',
                         error_category = NULL, completed_at = $5
                     WHERE id = $1 AND worker_owner_id = $2 AND state = 'running'",
                    vec![
                        run_id.clone().into(),
                        worker_owner_id.clone().into(),
                        counts_json.into(),
                        oldest_remaining_at.map(timestamp).into(),
                        timestamp(now).into(),
                    ],
                ))
                .await?;
            if changed.rows_affected() != 1 {
                return Err(storage("retention worker claim was lost"));
            }
            tx.execute(self.db.stmt(
                "UPDATE store_retention_state
                 SET run_in_progress = 0, current_run_id = NULL, current_worker_owner_id = NULL,
                     last_run_id = $1, consecutive_failures = 0, updated_at = $2
                 WHERE singleton_id = 1 AND current_run_id = $1
                   AND current_worker_owner_id = $3",
                vec![
                    run_id.clone().into(),
                    timestamp(now).into(),
                    worker_owner_id.clone().into(),
                ],
            ))
            .await?;
            insert_access_audit(
                &self.db,
                &*tx,
                &actor.actor_id,
                &actor.actor_role,
                "retention_run",
                serde_json::json!({"run_id": run_id, "counts": counts}),
                &actor.reason,
                "succeeded",
                now,
            )
            .await?;
            Ok(())
        }
        .await;
        finish_transaction(tx, outcome).await
    }

    async fn finalize_failure(
        &self,
        run_id: &str,
        now: DateTime<Utc>,
        category: &str,
        actor: &RetentionRunActor,
    ) -> Result<(), StoreRetentionError> {
        let tx = self.db.begin_write().await?;
        let run_id = run_id.to_string();
        let worker_owner_id = self.worker_owner_id.to_string();
        let category = category.to_string();
        let outcome = async {
            let changed = tx
                .execute(self.db.stmt(
                    "UPDATE store_retention_runs
                     SET state = 'failed', error_category = $3, completed_at = $4
                     WHERE id = $1 AND worker_owner_id = $2 AND state = 'running'",
                    vec![
                        run_id.clone().into(),
                        worker_owner_id.clone().into(),
                        category.clone().into(),
                        timestamp(now).into(),
                    ],
                ))
                .await?;
            if changed.rows_affected() != 1 {
                return Err(storage("retention worker claim was lost"));
            }
            increment_failure_locked(&self.db, &*tx, &run_id, now).await?;
            tx.execute(self.db.stmt(
                "UPDATE store_retention_state
                 SET run_in_progress = 0, current_run_id = NULL, current_worker_owner_id = NULL,
                     last_run_id = $1, updated_at = $2
                 WHERE singleton_id = 1 AND current_run_id = $1
                   AND current_worker_owner_id = $3",
                vec![
                    run_id.clone().into(),
                    timestamp(now).into(),
                    worker_owner_id.clone().into(),
                ],
            ))
            .await?;
            insert_access_audit(
                &self.db,
                &*tx,
                &actor.actor_id,
                &actor.actor_role,
                "retention_run",
                serde_json::json!({"run_id": run_id, "counts": StoreRetentionCounts::default(), "error_category": category}),
                &actor.reason,
                "failed",
                now,
            )
            .await?;
            Ok(())
        }
        .await;
        finish_transaction(tx, outcome).await?;
        if self.status().await?.checkout_paused {
            tracing::error!(
                run_id,
                error_category = category,
                "critical Store retention alert paused checkout"
            );
        }
        Ok(())
    }

    async fn run_by_id(&self, id: &str) -> Result<Option<StoreRetentionRun>, StoreRetentionError> {
        self.db
            .read()
            .query_one(self.db.stmt(
                "SELECT id, worker_owner_id, policy_version, counts_json,
                        oldest_remaining_at, state, error_category, started_at, completed_at
                 FROM store_retention_runs WHERE id = $1",
                vec![id.into()],
            ))
            .await?
            .map(run_from_row)
            .transpose()
    }

    async fn alert_by_id(
        &self,
        id: &str,
    ) -> Result<Option<StoreRetentionAlert>, StoreRetentionError> {
        self.db
            .read()
            .query_one(self.db.stmt(
                "SELECT id, run_id, severity, consecutive_failures, created_at,
                        contained_at, containment_id
                 FROM store_retention_alerts WHERE id = $1",
                vec![id.into()],
            ))
            .await?
            .map(alert_from_row)
            .transpose()
    }

    async fn legal_hold(
        &self,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<StoreLegalHold>, StoreRetentionError> {
        legal_hold_by_id(&self.db, self.db.read(), id, now).await
    }

    async fn list_containments(
        &self,
        limit: u64,
    ) -> Result<Vec<StoreRetentionContainment>, StoreRetentionError> {
        self.db
            .read()
            .query_all(self.db.stmt(
                "SELECT id, alert_id, actor_id, reason, evidence_digest, created_at
                 FROM store_retention_containments
                 ORDER BY created_at DESC, id DESC LIMIT $1",
                vec![(limit.min(100) as i64).into()],
            ))
            .await?
            .into_iter()
            .map(containment_from_row)
            .collect()
    }
}

pub async fn retention_checkout_paused<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
) -> Result<bool, DbErr> {
    let lock = if db.is_postgres() { " FOR UPDATE" } else { "" };
    let row = connection
        .query_one(db.stmt(
            &format!(
                "SELECT checkout_paused FROM store_retention_state WHERE singleton_id = 1{lock}"
            ),
            vec![],
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("retention state is missing".to_string()))?;
    let paused: i32 = row.try_get("", "checkout_paused")?;
    match paused {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DbErr::Custom(
            "retention checkout pause value is invalid".to_string(),
        )),
    }
}

pub fn spawn_daily_retention_job(db: DbPool, lease: StorePrimaryLease, shutdown: Arc<AtomicBool>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(duration_until_next_retention_run(Utc::now())).await;
            if shutdown.load(Ordering::Acquire) {
                return;
            }
            if let Err(error) = lease.validate().await {
                tracing::error!(error = %error, "Store retention job lost the Primary lease");
                return;
            }
            let retention = StoreRetention::new(db.clone(), lease.owner_id().to_string());
            match retention.run_now(RetentionRunActor::scheduled()).await {
                Ok(run) if run.state == StoreRetentionRunState::Succeeded => {
                    tracing::info!(run_id = %run.id, "Store retention run completed");
                }
                Ok(run) => {
                    tracing::error!(
                        run_id = %run.id,
                        error_category = ?run.error_category,
                        "Store retention run failed"
                    );
                }
                Err(StoreRetentionError::RunActive) => {
                    tracing::warn!("Store retention run skipped because another run is active");
                }
                Err(error) => {
                    tracing::error!(error = %error, "Store retention scheduler failed");
                }
            }
        }
    });
}

fn duration_until_next_retention_run(now: DateTime<Utc>) -> StdDuration {
    let today = now
        .with_hour(RETENTION_SCHEDULER_HOUR_UTC)
        .and_then(|value| value.with_minute(0))
        .and_then(|value| value.with_second(0))
        .and_then(|value| value.with_nanosecond(0))
        .expect("03:00 UTC is valid");
    let next = if now < today {
        today
    } else {
        today + Duration::days(1)
    };
    (next - now)
        .to_std()
        .unwrap_or_else(|_| StdDuration::from_secs(1))
}

async fn claim_run_locked<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    run_id: &str,
    owner_id: &str,
    policy_version: &str,
    now: DateTime<Utc>,
    for_update: bool,
) -> Result<(), StoreRetentionError> {
    let lock = if for_update { " FOR UPDATE" } else { "" };
    let state = connection
        .query_one(db.stmt(
            &format!(
                "SELECT run_in_progress, current_run_id, current_worker_owner_id
                 FROM store_retention_state WHERE singleton_id = 1{lock}"
            ),
            vec![],
        ))
        .await?
        .ok_or_else(|| storage("retention state is missing"))?;
    if boolean(&state, "run_in_progress")? {
        let current_owner = optional_string(&state, "current_worker_owner_id")?
            .ok_or_else(|| storage("active retention owner is missing"))?;
        if current_owner == owner_id {
            return Err(StoreRetentionError::RunActive);
        }
        let interrupted_id = optional_string(&state, "current_run_id")?
            .ok_or_else(|| storage("active retention run ID is missing"))?;
        connection
            .execute(db.stmt(
                "UPDATE store_retention_runs
                 SET state = 'failed', error_category = 'interrupted', completed_at = $2
                 WHERE id = $1 AND state = 'running'",
                vec![interrupted_id.clone().into(), timestamp(now).into()],
            ))
            .await?;
        increment_failure_locked(db, connection, &interrupted_id, now).await?;
        insert_access_audit(
            db,
            connection,
            RETENTION_SYSTEM_ACTOR,
            "system",
            "retention_run",
            serde_json::json!({
                "run_id": interrupted_id,
                "counts": StoreRetentionCounts::default(),
                "error_category": "interrupted",
            }),
            "scheduled_retention",
            "failed",
            now,
        )
        .await?;
    }
    connection
        .execute(db.stmt(
            "INSERT INTO store_retention_runs
                (id, policy_version, counts_json, oldest_remaining_at, state,
                 error_category, started_at, completed_at, worker_owner_id)
             VALUES ($1, $2, $3, NULL, 'running', NULL, $4, NULL, $5)",
            vec![
                run_id.into(),
                policy_version.into(),
                serde_json::to_string(&StoreRetentionCounts::default())
                    .map_err(storage)?
                    .into(),
                timestamp(now).into(),
                owner_id.into(),
            ],
        ))
        .await?;
    connection
        .execute(db.stmt(
            "UPDATE store_retention_state
             SET run_in_progress = 1, current_run_id = $1,
                 current_worker_owner_id = $2, updated_at = $3
             WHERE singleton_id = 1",
            vec![run_id.into(), owner_id.into(), timestamp(now).into()],
        ))
        .await?;
    Ok(())
}

async fn increment_failure_locked<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<(), StoreRetentionError> {
    let row = connection
        .query_one(db.stmt(
            "SELECT consecutive_failures, checkout_paused
             FROM store_retention_state WHERE singleton_id = 1",
            vec![],
        ))
        .await?
        .ok_or_else(|| storage("retention state is missing"))?;
    let failures = nonnegative_u64(&row, "consecutive_failures")?
        .checked_add(1)
        .ok_or_else(|| storage("retention failure count overflow"))?;
    let paused = boolean(&row, "checkout_paused")?;
    let alert_id = if failures >= 3 && !paused {
        let alert_id = Uuid::new_v4().to_string();
        connection
            .execute(db.stmt(
                "INSERT INTO store_retention_alerts
                    (id, run_id, severity, consecutive_failures, created_at,
                     contained_at, containment_id)
                 VALUES ($1, $2, 'critical', $3, $4, NULL, NULL)",
                vec![
                    alert_id.clone().into(),
                    run_id.into(),
                    i64::try_from(failures)
                        .map_err(|_| storage("retention failure count overflow"))?
                        .into(),
                    timestamp(now).into(),
                ],
            ))
            .await?;
        Some(alert_id)
    } else {
        None
    };
    connection
        .execute(db.stmt(
            "UPDATE store_retention_state
             SET consecutive_failures = $1,
                 checkout_paused = CASE WHEN $2 IS NULL THEN checkout_paused ELSE 1 END,
                 active_alert_id = COALESCE($2, active_alert_id),
                 updated_at = $3
             WHERE singleton_id = 1",
            vec![
                i64::try_from(failures)
                    .map_err(|_| storage("retention failure count overflow"))?
                    .into(),
                alert_id.into(),
                timestamp(now).into(),
            ],
        ))
        .await?;
    Ok(())
}

async fn apply_retention<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    now: DateTime<Utc>,
    policy: &StorePrivacyRetention,
) -> Result<StoreRetentionCounts, StoreRetentionError> {
    Ok(StoreRetentionCounts {
        raw_callback_bodies: clear_raw_callbacks(db, connection, now).await?,
        network_metadata: clear_network_metadata(db, connection, now).await?,
        financial_records: delete_financial_records(
            db,
            connection,
            now,
            policy.financial_records_days,
        )
        .await?,
        redemption_audits: delete_redemption_audits(db, connection, now).await?,
        expired_reauth_grants: delete_reauth_grants(
            db,
            connection,
            now,
            policy.expired_reauth_grant_hours,
        )
        .await?,
    })
}

async fn clear_raw_callbacks<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    now: DateTime<Utc>,
) -> Result<u64, StoreRetentionError> {
    let cutoff = checked_sub(now, Duration::days(30))?;
    let ids = candidate_ids(
        db,
        connection,
        "SELECT e.id FROM store_provider_events e
         WHERE e.received_at <= $1 AND e.raw_ciphertext_base64 IS NOT NULL
           AND NOT EXISTS (
             SELECT 1 FROM store_legal_hold_items i
             JOIN store_legal_holds h ON h.id = i.hold_id
             WHERE i.data_class = 'raw_callback_bodies' AND i.identifier = e.id
               AND h.starts_at <= $2 AND h.expires_at > $2
           )
         ORDER BY e.received_at ASC, e.id ASC LIMIT $3",
        vec![
            timestamp(cutoff).into(),
            timestamp(now).into(),
            RETENTION_BATCH_SIZE.into(),
        ],
    )
    .await?;
    mutate_ids(
        db,
        connection,
        &ids,
        "UPDATE store_provider_events
         SET raw_format_version = NULL, raw_key_id = NULL,
             raw_nonce_base64 = NULL, raw_ciphertext_base64 = NULL
         WHERE id = $1 AND raw_ciphertext_base64 IS NOT NULL",
    )
    .await
}

async fn clear_network_metadata<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    now: DateTime<Utc>,
) -> Result<u64, StoreRetentionError> {
    let cutoff = checked_sub(now, Duration::days(90))?;
    let ids = candidate_ids(
        db,
        connection,
        "SELECT e.id FROM store_provider_events e
         WHERE e.received_at <= $1 AND (e.source_ip IS NOT NULL OR e.user_agent IS NOT NULL)
           AND NOT EXISTS (
             SELECT 1 FROM store_legal_hold_items i
             JOIN store_legal_holds h ON h.id = i.hold_id
             WHERE i.data_class = 'network_metadata' AND i.identifier = e.id
               AND h.starts_at <= $2 AND h.expires_at > $2
           )
         ORDER BY e.received_at ASC, e.id ASC LIMIT $3",
        vec![
            timestamp(cutoff).into(),
            timestamp(now).into(),
            RETENTION_BATCH_SIZE.into(),
        ],
    )
    .await?;
    mutate_ids(
        db,
        connection,
        &ids,
        "UPDATE store_provider_events
         SET source_ip = NULL, user_agent = NULL
         WHERE id = $1 AND (source_ip IS NOT NULL OR user_agent IS NOT NULL)",
    )
    .await
}

async fn delete_reauth_grants<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    now: DateTime<Utc>,
    hours: i64,
) -> Result<u64, StoreRetentionError> {
    let cutoff = checked_sub(now, Duration::hours(hours))?;
    let ids = candidate_ids(
        db,
        connection,
        "SELECT g.id FROM store_reauth_grants g
         WHERE g.expires_at <= $1
           AND NOT EXISTS (
             SELECT 1 FROM store_legal_hold_items i
             JOIN store_legal_holds h ON h.id = i.hold_id
             WHERE i.data_class = 'expired_reauth_grants' AND i.identifier = g.id
               AND h.starts_at <= $2 AND h.expires_at > $2
           )
         ORDER BY g.expires_at ASC, g.id ASC LIMIT $3",
        vec![
            timestamp(cutoff).into(),
            timestamp(now).into(),
            RETENTION_BATCH_SIZE.into(),
        ],
    )
    .await?;
    mutate_ids(
        db,
        connection,
        &ids,
        "DELETE FROM store_reauth_grants WHERE id = $1",
    )
    .await
}

async fn delete_redemption_audits<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    now: DateTime<Utc>,
) -> Result<u64, StoreRetentionError> {
    let cutoff = checked_sub(now, Duration::days(730))?;
    let ids = candidate_ids(
        db,
        connection,
        "SELECT a.id FROM store_access_audits a
         WHERE a.created_at <= $1
           AND a.action IN ('redemption_reveal', 'redemption_copy', 'redemption_export')
           AND NOT EXISTS (
             SELECT 1 FROM store_legal_hold_items i
             JOIN store_legal_holds h ON h.id = i.hold_id
             WHERE i.data_class = 'redemption_audits' AND i.identifier = a.id
               AND h.starts_at <= $2 AND h.expires_at > $2
           )
         ORDER BY a.created_at ASC, a.id ASC LIMIT $3",
        vec![
            timestamp(cutoff).into(),
            timestamp(now).into(),
            RETENTION_BATCH_SIZE.into(),
        ],
    )
    .await?;
    mutate_ids(
        db,
        connection,
        &ids,
        "DELETE FROM store_access_audits WHERE id = $1",
    )
    .await
}

async fn delete_financial_records<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    now: DateTime<Utc>,
    days: i64,
) -> Result<u64, StoreRetentionError> {
    let financial_cutoff = checked_sub(now, Duration::days(days))?;
    let provider_event_cutoff = checked_sub(
        now,
        Duration::days(days.max(NETWORK_METADATA_RETENTION_DAYS)),
    )?;
    const HOLD_EXCLUSION: &str = "NOT EXISTS (
             SELECT 1 FROM store_legal_hold_items i
             JOIN store_legal_holds h ON h.id = i.hold_id
             WHERE i.data_class = 'financial_records' AND i.identifier = r.id
               AND h.starts_at <= $3 AND h.expires_at > $3
           )";
    let sql = format!(
        "SELECT id, source_table FROM (
             SELECT r.id, r.created_at AS retention_at, 'store_orders' AS source_table
             FROM store_orders r
             WHERE r.created_at <= $1
               AND r.payment_state IN ('closed', 'refunded')
               AND {HOLD_EXCLUSION}
             UNION ALL
             SELECT r.id, r.received_at, 'store_provider_events'
             FROM store_provider_events r
             WHERE r.received_at <= $2
               AND {HOLD_EXCLUSION}
             UNION ALL
             SELECT r.id, r.created_at, 'billing_ledger'
             FROM billing_ledger r
             WHERE r.created_at <= $1
               AND {HOLD_EXCLUSION}
             UNION ALL
             SELECT r.id, r.created_at, 'store_refunds'
             FROM store_refunds r
             WHERE r.created_at <= $1
               AND {HOLD_EXCLUSION}
             UNION ALL
             SELECT r.id, r.created_at, 'store_order_recovery_claims'
             FROM store_order_recovery_claims r
             WHERE r.created_at <= $1
               AND {HOLD_EXCLUSION}
             UNION ALL
             SELECT r.id, r.imported_at, 'store_settlement_reports'
             FROM store_settlement_reports r
             WHERE r.imported_at <= $1
               AND {HOLD_EXCLUSION}
             UNION ALL
             SELECT r.id, r.created_at, 'store_access_audits'
             FROM store_access_audits r
             WHERE r.created_at <= $1
               AND r.action NOT IN ('redemption_reveal', 'redemption_copy', 'redemption_export')
               AND {HOLD_EXCLUSION}
         ) candidates
         ORDER BY retention_at ASC, id ASC
         LIMIT $4"
    );
    let candidates = financial_candidate_rows(
        db,
        connection,
        &sql,
        vec![
            timestamp(financial_cutoff).into(),
            timestamp(provider_event_cutoff).into(),
            timestamp(now).into(),
            RETENTION_BATCH_SIZE.into(),
        ],
    )
    .await?;
    let mut deleted = 0_u64;
    for (id, table) in candidates {
        if delete_financial_root(db, connection, &table, &id, now).await? {
            deleted += 1;
        }
    }
    Ok(deleted)
}

async fn delete_financial_root<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    table: &str,
    id: &str,
    now: DateTime<Utc>,
) -> Result<bool, StoreRetentionError> {
    match table {
        "store_orders" => {
            delete_order_children(db, connection, id, now).await?;
        }
        "store_provider_events" => {
            connection
                .execute(db.stmt(
                    "DELETE FROM store_order_event_applications
                     WHERE provider_event_row_id = $1",
                    vec![id.into()],
                ))
                .await?;
        }
        "store_refunds" => {
            connection
                .execute(db.stmt(
                    "DELETE FROM store_refund_query_retries WHERE refund_id = $1",
                    vec![id.into()],
                ))
                .await?;
        }
        "store_settlement_reports" => {
            connection
                .execute(db.stmt(
                    "DELETE FROM store_settlement_lines
                     WHERE report_id = $1 AND NOT EXISTS (
                       SELECT 1 FROM store_legal_hold_items i
                       JOIN store_legal_holds h ON h.id = i.hold_id
                       WHERE i.data_class = 'financial_records'
                         AND i.identifier = store_settlement_lines.id
                         AND h.starts_at <= $2 AND h.expires_at > $2
                     )",
                    vec![id.into(), timestamp(now).into()],
                ))
                .await?;
            let remaining = count(
                db,
                connection,
                "SELECT COUNT(*) AS value FROM store_settlement_lines WHERE report_id = $1",
                vec![id.into()],
            )
            .await?;
            if remaining != 0 {
                return Ok(false);
            }
        }
        _ => {}
    }
    let result = connection
        .execute(db.stmt(
            &format!("DELETE FROM {table} WHERE id = $1"),
            vec![id.into()],
        ))
        .await?;
    Ok(result.rows_affected() == 1)
}

async fn delete_order_children<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    order_id: &str,
    now: DateTime<Utc>,
) -> Result<(), StoreRetentionError> {
    let recovery_rows = connection
        .query_all(db.stmt(
            "SELECT id FROM store_order_reward_recoveries WHERE order_id = $1",
            vec![order_id.into()],
        ))
        .await?;
    for recovery in recovery_rows {
        let recovery_id = string(&recovery, "id")?;
        delete_unheld_by_parent(
            db,
            connection,
            "store_order_recovery_claims",
            "recovery_id",
            &recovery_id,
            now,
        )
        .await?;
    }
    for (table, parent_column) in [
        ("store_order_event_applications", "order_id"),
        ("store_refunds", "order_id"),
        ("store_order_reward_recoveries", "order_id"),
        ("store_payment_attempts", "order_id"),
    ] {
        delete_unheld_by_parent(db, connection, table, parent_column, order_id, now).await?;
    }
    Ok(())
}

async fn delete_unheld_by_parent<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    table: &str,
    parent_column: &str,
    parent_id: &str,
    now: DateTime<Utc>,
) -> Result<(), StoreRetentionError> {
    let identifier_column = if table == "store_order_event_applications" {
        "provider_event_row_id"
    } else {
        "id"
    };
    connection
        .execute(db.stmt(
            &format!(
                "DELETE FROM {table}
                 WHERE {parent_column} = $1 AND NOT EXISTS (
                   SELECT 1 FROM store_legal_hold_items i
                   JOIN store_legal_holds h ON h.id = i.hold_id
                   WHERE i.data_class = 'financial_records'
                     AND i.identifier = {table}.{identifier_column}
                     AND h.starts_at <= $2 AND h.expires_at > $2
                 )"
            ),
            vec![parent_id.into(), timestamp(now).into()],
        ))
        .await?;
    Ok(())
}

async fn oldest_remaining<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
) -> Result<Option<DateTime<Utc>>, StoreRetentionError> {
    let mut oldest = None;
    for (table, column, condition) in [
        ("store_provider_events", "received_at", "1 = 1"),
        ("store_reauth_grants", "expires_at", "1 = 1"),
        (
            "store_orders",
            "created_at",
            "payment_state IN ('closed', 'refunded')",
        ),
        ("billing_ledger", "created_at", "1 = 1"),
        ("store_refunds", "created_at", "1 = 1"),
        ("store_order_recovery_claims", "created_at", "1 = 1"),
        ("store_settlement_reports", "imported_at", "1 = 1"),
        ("store_access_audits", "created_at", "1 = 1"),
    ] {
        let row = connection
            .query_one(db.stmt(
                &format!(
                    "SELECT {column} AS retained_at FROM {table}
                     WHERE {condition} ORDER BY {column} ASC LIMIT 1"
                ),
                vec![],
            ))
            .await?;
        if let Some(row) = row {
            let value = parse_time(&string(&row, "retained_at")?)?;
            oldest = Some(oldest.map_or(value, |current: DateTime<Utc>| current.min(value)));
        }
    }
    Ok(oldest)
}

async fn candidate_ids<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    sql: &str,
    values: Vec<Value>,
) -> Result<Vec<String>, StoreRetentionError> {
    connection
        .query_all(db.stmt(sql, values))
        .await?
        .into_iter()
        .map(|row| string(&row, "id"))
        .collect()
}

async fn financial_candidate_rows<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    sql: &str,
    values: Vec<Value>,
) -> Result<Vec<(String, String)>, StoreRetentionError> {
    connection
        .query_all(db.stmt(sql, values))
        .await?
        .into_iter()
        .map(|row| Ok((string(&row, "id")?, string(&row, "source_table")?)))
        .collect()
}

async fn mutate_ids<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    ids: &[String],
    sql: &str,
) -> Result<u64, StoreRetentionError> {
    let mut changed = 0_u64;
    for id in ids {
        changed = changed
            .checked_add(
                connection
                    .execute(db.stmt(sql, vec![id.clone().into()]))
                    .await?
                    .rows_affected(),
            )
            .ok_or_else(|| storage("retention count overflow"))?;
    }
    Ok(changed)
}

async fn count<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    sql: &str,
    values: Vec<Value>,
) -> Result<i64, StoreRetentionError> {
    connection
        .query_one(db.stmt(sql, values))
        .await?
        .ok_or_else(|| storage("retention count query returned no row"))?
        .try_get("", "value")
        .map_err(Into::into)
}

async fn legal_hold_by_id<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    id: &str,
    now: DateTime<Utc>,
) -> Result<Option<StoreLegalHold>, StoreRetentionError> {
    connection
        .query_one(db.stmt(
            &format!("{} WHERE h.id = $1", legal_hold_select()),
            vec![id.into()],
        ))
        .await?
        .map(|row| legal_hold_from_row(row, now))
        .transpose()
}

fn legal_hold_select() -> &'static str {
    "SELECT h.id, h.data_class, h.identifiers_json, h.reason, h.requesting_authority,
            a.requester_id, h.approver_id, a.approver_role, h.starts_at, h.expires_at,
            h.created_at, a.extends_hold_id
     FROM store_legal_holds h
     JOIN store_legal_hold_approvals a ON a.hold_id = h.id"
}

fn legal_hold_from_row(
    row: QueryResult,
    now: DateTime<Utc>,
) -> Result<StoreLegalHold, StoreRetentionError> {
    let starts_at = parse_time(&string(&row, "starts_at")?)?;
    let expires_at = parse_time(&string(&row, "expires_at")?)?;
    let data_class = serde_json::from_value(serde_json::Value::String(string(&row, "data_class")?))
        .map_err(storage)?;
    Ok(StoreLegalHold {
        id: string(&row, "id")?,
        data_class,
        identifiers: serde_json::from_str(&string(&row, "identifiers_json")?).map_err(storage)?,
        reason: string(&row, "reason")?,
        requesting_authority: string(&row, "requesting_authority")?,
        requester_id: string(&row, "requester_id")?,
        approver_id: string(&row, "approver_id")?,
        approver_role: string(&row, "approver_role")?,
        starts_at,
        expires_at,
        created_at: parse_time(&string(&row, "created_at")?)?,
        extends_hold_id: optional_string(&row, "extends_hold_id")?,
        active: starts_at <= now && now < expires_at,
    })
}

fn run_from_row(row: QueryResult) -> Result<StoreRetentionRun, StoreRetentionError> {
    Ok(StoreRetentionRun {
        id: string(&row, "id")?,
        worker_owner_id: string(&row, "worker_owner_id")?,
        policy_version: string(&row, "policy_version")?,
        counts: serde_json::from_str(&string(&row, "counts_json")?).map_err(storage)?,
        oldest_remaining_at: optional_string(&row, "oldest_remaining_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
        state: StoreRetentionRunState::from_str(&string(&row, "state")?)?,
        error_category: optional_string(&row, "error_category")?,
        started_at: parse_time(&string(&row, "started_at")?)?,
        completed_at: optional_string(&row, "completed_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
    })
}

fn alert_from_row(row: QueryResult) -> Result<StoreRetentionAlert, StoreRetentionError> {
    Ok(StoreRetentionAlert {
        id: string(&row, "id")?,
        run_id: string(&row, "run_id")?,
        severity: string(&row, "severity")?,
        consecutive_failures: nonnegative_u64(&row, "consecutive_failures")?,
        created_at: parse_time(&string(&row, "created_at")?)?,
        contained_at: optional_string(&row, "contained_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
        containment_id: optional_string(&row, "containment_id")?,
    })
}

fn containment_from_row(
    row: QueryResult,
) -> Result<StoreRetentionContainment, StoreRetentionError> {
    Ok(StoreRetentionContainment {
        id: string(&row, "id")?,
        alert_id: string(&row, "alert_id")?,
        actor_id: string(&row, "actor_id")?,
        reason: string(&row, "reason")?,
        evidence_digest: string(&row, "evidence_digest")?,
        created_at: parse_time(&string(&row, "created_at")?)?,
    })
}

#[allow(clippy::too_many_arguments)]
async fn insert_access_audit<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    actor_id: &str,
    actor_role: &str,
    action: &str,
    scope: serde_json::Value,
    reason: &str,
    result: &str,
    now: DateTime<Utc>,
) -> Result<(), StoreRetentionError> {
    connection
        .execute(db.stmt(
            "INSERT INTO store_access_audits
                (id, actor_id, actor_role, action, scope_json, reason, result, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            vec![
                Uuid::new_v4().to_string().into(),
                actor_id.into(),
                actor_role.into(),
                action.into(),
                scope.to_string().into(),
                reason.into(),
                result.into(),
                timestamp(now).into(),
            ],
        ))
        .await?;
    Ok(())
}

async fn finish_transaction<T>(
    transaction: crate::db::WriteTransaction,
    outcome: Result<T, StoreRetentionError>,
) -> Result<T, StoreRetentionError> {
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

fn validate_actor(actor: &RetentionRunActor) -> Result<(), StoreRetentionError> {
    if !valid_identifier(&actor.actor_id)
        || !matches!(actor.actor_role.as_str(), "system" | "admin")
        || !valid_nonblank_text(&actor.reason, 2000)
    {
        return Err(StoreRetentionError::InvalidInput);
    }
    Ok(())
}

fn validate_hold_input(
    input: &CreateStoreLegalHoldInput,
    approver_id: &str,
    now: DateTime<Utc>,
) -> Result<(), StoreRetentionError> {
    if !(1..=100).contains(&input.identifiers.len())
        || input
            .identifiers
            .iter()
            .any(|value| !valid_identifier(value))
        || !valid_text(&input.reason, 2000)
        || !valid_text(&input.requesting_authority, 500)
        || !valid_identifier(&input.requester_id)
        || !valid_identifier(approver_id)
        || input.requester_id == approver_id
        || !matches!(input.approver_role.as_str(), "privacy" | "legal")
        || input.expires_at <= now
        || input
            .extends_hold_id
            .as_ref()
            .is_some_and(|value| !valid_identifier(value))
    {
        return Err(StoreRetentionError::InvalidInput);
    }
    Ok(())
}

fn valid_retention(value: &StorePrivacyRetention) -> bool {
    value.raw_callback_days == 30
        && value.network_metadata_days == 90
        && (1..=36_500).contains(&value.financial_records_days)
        && value.redemption_audit_days == 730
        && (1..=24).contains(&value.expired_reauth_grant_hours)
}

fn valid_text(value: &str, max_chars: usize) -> bool {
    !value.is_empty() && value.trim() == value && value.chars().count() <= max_chars
}

fn valid_nonblank_text(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max_chars
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && value.len() <= 255
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn checked_sub(
    value: DateTime<Utc>,
    duration: Duration,
) -> Result<DateTime<Utc>, StoreRetentionError> {
    value
        .checked_sub_signed(duration)
        .ok_or(StoreRetentionError::InvalidInput)
}

fn canonical_time(value: DateTime<Utc>) -> Result<DateTime<Utc>, StoreRetentionError> {
    DateTime::from_timestamp_micros(value.timestamp_micros())
        .ok_or(StoreRetentionError::InvalidInput)
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, StoreRetentionError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(storage)
}

fn string(row: &QueryResult, column: &str) -> Result<String, StoreRetentionError> {
    row.try_get("", column).map_err(Into::into)
}

fn optional_string(row: &QueryResult, column: &str) -> Result<Option<String>, StoreRetentionError> {
    row.try_get("", column).map_err(Into::into)
}

fn boolean(row: &QueryResult, column: &str) -> Result<bool, StoreRetentionError> {
    match row.try_get::<i32>("", column)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(storage(format!("{column} is not a Boolean"))),
    }
}

fn nonnegative_u64(row: &QueryResult, column: &str) -> Result<u64, StoreRetentionError> {
    let value = row.try_get::<i64>("", column)?;
    u64::try_from(value).map_err(|_| storage(format!("{column} is negative")))
}

fn storage(error: impl ToString) -> StoreRetentionError {
    StoreRetentionError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{StoreRetentionDataClass, duration_until_next_retention_run};

    #[test]
    fn scheduler_targets_the_next_three_am_utc() {
        let before = Utc.with_ymd_and_hms(2026, 8, 28, 2, 0, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 8, 28, 4, 0, 0).unwrap();
        assert_eq!(duration_until_next_retention_run(before).as_secs(), 3600);
        assert_eq!(
            duration_until_next_retention_run(after).as_secs(),
            23 * 3600
        );
    }

    #[test]
    fn data_class_serialization_is_stable() {
        assert_eq!(
            serde_json::to_string(&StoreRetentionDataClass::RawCallbackBodies).unwrap(),
            "\"raw_callback_bodies\""
        );
    }
}
