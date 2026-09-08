use super::models::{PlanQuota, WindowKind};
use super::quota_gate::QuotaGateStore;
use crate::db::DbPool;
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use chrono_tz::Asia::Shanghai;
use sea_orm::{ConnectionTrait, DbErr, QueryResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const NANO_USD_PER_CENT: i128 = 10_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementGenerationInput {
    pub expected_generation: Option<i64>,
    pub user_id: String,
    pub product_id: String,
    pub product_name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub rate_numerator: String,
    pub rate_denominator: String,
    pub group_ids: Vec<String>,
    pub quotas: Vec<PlanQuota>,
    pub source_kind: String,
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitlementGeneration {
    pub id: String,
    pub user_id: String,
    pub generation: i64,
    pub product_id: String,
    pub product_name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub rate_numerator: String,
    pub rate_denominator: String,
    pub group_ids: Vec<String>,
    pub quotas: Vec<PlanQuota>,
    pub source_kind: String,
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaReservationInput {
    pub user_id: String,
    pub request_id: String,
    pub maximum_nano_usd: i128,
    pub pricing_revision: String,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFundingInput {
    pub user_id: String,
    pub request_id: String,
    pub effective_groups: Vec<String>,
    pub maximum_nano_usd: Option<i128>,
    pub pricing_revision: String,
    pub now: DateTime<Utc>,
    pub replica: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanFundingAdmission {
    Balance,
    Plan(QuotaReservation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaTerminalState {
    Reserved,
    Settled,
    Released,
    Violated,
}

impl QuotaTerminalState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Settled => "settled",
            Self::Released => "released",
            Self::Violated => "violated",
        }
    }

    fn parse(value: &str) -> Result<Self, QuotaError> {
        match value {
            "reserved" => Ok(Self::Reserved),
            "settled" => Ok(Self::Settled),
            "released" => Ok(Self::Released),
            "violated" => Ok(Self::Violated),
            _ => Err(QuotaError::Storage(
                "invalid quota reservation state".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaReservation {
    pub id: String,
    pub request_id: String,
    pub user_id: String,
    pub entitlement_id: String,
    pub generation: i64,
    pub maximum_nano_usd: i128,
    pub reserved_fen_cny: i128,
    pub rate_numerator: String,
    pub rate_denominator: String,
    pub pricing_revision: String,
    pub actual_nano_usd: Option<i128>,
    pub actual_fen_cny: Option<i128>,
    pub state: QuotaTerminalState,
    pub admitted_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
    pub bucket_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum QuotaError {
    #[error("{0}")]
    Code(&'static str),
    #[error("quota storage failed: {0}")]
    Storage(String),
}

impl QuotaError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Code(code) => code,
            Self::Storage(_) => "quota_storage_error",
        }
    }
}

impl From<DbErr> for QuotaError {
    fn from(value: DbErr) -> Self {
        Self::Storage(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct QuotaStore {
    db: DbPool,
}

impl QuotaStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn replace_entitlement(
        &self,
        input: EntitlementGenerationInput,
    ) -> Result<EntitlementGeneration, QuotaError> {
        validate_generation_input(&input)?;
        if self.db.is_sqlite() {
            let db = self.db.clone();
            self.db
                .with_immediate_write(move |connection| {
                    Box::pin(async move { replace_entitlement_tx(&db, connection, input).await })
                })
                .await
        } else {
            let tx = self.db.begin_write().await?;
            let result = replace_entitlement_tx(&self.db, &*tx, input).await;
            match result {
                Ok(value) => {
                    tx.commit().await?;
                    Ok(value)
                }
                Err(error) => {
                    tx.rollback().await?;
                    Err(error)
                }
            }
        }
    }

    pub async fn current_entitlement(
        &self,
        user_id: &str,
    ) -> Result<Option<EntitlementGeneration>, QuotaError> {
        self.db
            .read()
            .query_one(self.db.stmt(
                "SELECT g.id, g.user_id, g.generation, g.product_id, g.product_name,
                        g.starts_at, g.ends_at, g.rate_numerator, g.rate_denominator,
                        g.group_ids, g.quota_json, g.source_kind, g.source_id
                 FROM store_plan_entitlement_current p
                 JOIN store_plan_entitlement_generations g ON g.id = p.entitlement_id
                 WHERE p.user_id = $1",
                vec![user_id.into()],
            ))
            .await
            .map_err(storage)?
            .map(generation_from_row)
            .transpose()
    }

    pub async fn reserve(
        &self,
        input: QuotaReservationInput,
    ) -> Result<QuotaReservation, QuotaError> {
        self.reserve_bound(input, None).await
    }

    async fn reserve_bound(
        &self,
        input: QuotaReservationInput,
        expected_entitlement: Option<(String, i64)>,
    ) -> Result<QuotaReservation, QuotaError> {
        if input.user_id.trim().is_empty()
            || input.request_id.trim().is_empty()
            || input.pricing_revision.trim().is_empty()
            || input.maximum_nano_usd <= 0
        {
            return Err(QuotaError::Code("plan_request_unbounded"));
        }
        if self.db.is_sqlite() {
            let db = self.db.clone();
            self.db
                .with_immediate_write(move |connection| {
                    Box::pin(async move {
                        reserve_tx(&db, connection, input, expected_entitlement).await
                    })
                })
                .await
        } else {
            let tx = self.db.begin_write().await?;
            let result = reserve_tx(&self.db, &*tx, input, expected_entitlement).await;
            match result {
                Ok(value) => {
                    tx.commit().await?;
                    Ok(value)
                }
                Err(error) => {
                    tx.rollback().await?;
                    Err(error)
                }
            }
        }
    }

    pub async fn admit_funding(
        &self,
        input: PlanFundingInput,
    ) -> Result<PlanFundingAdmission, QuotaError> {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT g.id, g.user_id, g.generation, g.product_id, g.product_name,
                        g.starts_at, g.ends_at, g.rate_numerator, g.rate_denominator,
                        g.group_ids, g.quota_json, g.source_kind, g.source_id,
                        l.suspended_at, l.revoked_at
                 FROM store_plan_entitlement_current p
                 JOIN store_plan_entitlement_generations g ON g.id = p.entitlement_id
                 JOIN store_plan_entitlement_lifecycle l ON l.entitlement_id = g.id
                 WHERE p.user_id = $1",
                vec![input.user_id.clone().into()],
            ))
            .await
            .map_err(storage)?;
        let Some(row) = row else {
            return Ok(PlanFundingAdmission::Balance);
        };
        let suspended_at = parse_optional_timestamp(row_optional_string(&row, "suspended_at")?)?;
        let revoked_at = parse_optional_timestamp(row_optional_string(&row, "revoked_at")?)?;
        let entitlement = generation_from_row(row)?;
        if input.now < entitlement.starts_at
            || input.now >= entitlement.ends_at
            || suspended_at.is_some()
            || revoked_at.is_some()
        {
            return Ok(PlanFundingAdmission::Balance);
        }
        if !entitlement.group_ids.is_empty()
            && !entitlement.group_ids.iter().any(|group| {
                input
                    .effective_groups
                    .iter()
                    .any(|effective| effective == group)
            })
        {
            return Ok(PlanFundingAdmission::Balance);
        }
        if input.replica {
            return Err(QuotaError::Code("plan_admission_token_required"));
        }
        let maximum_nano_usd = input
            .maximum_nano_usd
            .filter(|value| *value > 0)
            .ok_or(QuotaError::Code("plan_request_unbounded"))?;
        self.reserve_bound(
            QuotaReservationInput {
                user_id: input.user_id,
                request_id: input.request_id,
                maximum_nano_usd,
                pricing_revision: input.pricing_revision,
                now: input.now,
            },
            Some((entitlement.id, entitlement.generation)),
        )
        .await
        .map(PlanFundingAdmission::Plan)
    }

    pub async fn settle(
        &self,
        reservation_id: &str,
        actual_nano_usd: i128,
        now: DateTime<Utc>,
    ) -> Result<QuotaReservation, QuotaError> {
        if reservation_id.trim().is_empty() || actual_nano_usd < 0 {
            return Err(QuotaError::Code("quota_terminal_conflict"));
        }
        self.terminal(reservation_id, Some(actual_nano_usd), now)
            .await
    }

    pub async fn release(
        &self,
        reservation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<QuotaReservation, QuotaError> {
        if reservation_id.trim().is_empty() {
            return Err(QuotaError::Code("quota_terminal_conflict"));
        }
        self.terminal(reservation_id, None, now).await
    }

    pub async fn settle_request_if_reserved(
        &self,
        request_id: &str,
        actual_nano_usd: i128,
        now: DateTime<Utc>,
    ) -> Result<Option<QuotaReservation>, QuotaError> {
        let Some(id) = self.reservation_id_for_request(request_id).await? else {
            return Ok(None);
        };
        self.settle(&id, actual_nano_usd, now).await.map(Some)
    }

    pub async fn release_request_if_reserved(
        &self,
        request_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<QuotaReservation>, QuotaError> {
        let Some(id) = self.reservation_id_for_request(request_id).await? else {
            return Ok(None);
        };
        self.release(&id, now).await.map(Some)
    }

    pub async fn reservation_for_request(
        &self,
        request_id: &str,
    ) -> Result<Option<QuotaReservation>, QuotaError> {
        if request_id.is_empty() {
            return Ok(None);
        }
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT id, request_id, user_id, entitlement_id, generation,
                        maximum_nano_usd, reserved_fen_cny, rate_numerator,
                        rate_denominator, pricing_revision, actual_nano_usd,
                        actual_fen_cny, state, admitted_at, terminal_at
                 FROM store_quota_reservations WHERE request_id = $1",
                vec![request_id.into()],
            ))
            .await
            .map_err(storage)?;
        match row {
            Some(row) => reservation_from_row(&self.db, self.db.read(), row)
                .await
                .map(Some),
            None => Ok(None),
        }
    }

    async fn reservation_id_for_request(
        &self,
        request_id: &str,
    ) -> Result<Option<String>, QuotaError> {
        if request_id.is_empty() {
            return Ok(None);
        }
        self.db
            .read()
            .query_one(self.db.stmt(
                "SELECT id FROM store_quota_reservations WHERE request_id = $1",
                vec![request_id.into()],
            ))
            .await
            .map_err(storage)?
            .map(|row| row_string(&row, "id"))
            .transpose()
    }

    async fn terminal(
        &self,
        reservation_id: &str,
        actual_nano_usd: Option<i128>,
        now: DateTime<Utc>,
    ) -> Result<QuotaReservation, QuotaError> {
        if self.db.is_sqlite() {
            let db = self.db.clone();
            let reservation_id = reservation_id.to_string();
            self.db
                .with_immediate_write(move |connection| {
                    Box::pin(async move {
                        terminal_tx(&db, connection, &reservation_id, actual_nano_usd, now).await
                    })
                })
                .await
        } else {
            let tx = self.db.begin_write().await?;
            let result = terminal_tx(&self.db, &*tx, reservation_id, actual_nano_usd, now).await;
            match result {
                Ok(value) => {
                    tx.commit().await?;
                    Ok(value)
                }
                Err(error) => {
                    tx.rollback().await?;
                    Err(error)
                }
            }
        }
    }
}

pub(crate) async fn replace_entitlement_tx<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    input: EntitlementGenerationInput,
) -> Result<EntitlementGeneration, QuotaError> {
    require_passed_gate(db, connection).await?;
    let lock = if db.is_postgres() { " FOR UPDATE" } else { "" };
    connection
        .query_one(db.stmt(
            &format!("SELECT id FROM users WHERE id = $1{lock}"),
            vec![input.user_id.clone().into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(QuotaError::Code("plan_entitlement_user_not_found"))?;

    if let Some(existing) = connection
        .query_one(db.stmt(
            &format!(
                "SELECT id, user_id, generation, product_id, product_name, starts_at, ends_at,
                        rate_numerator, rate_denominator, group_ids, quota_json,
                        source_kind, source_id
                 FROM store_plan_entitlement_generations
                 WHERE source_kind = $1 AND source_id = $2{lock}"
            ),
            vec![
                input.source_kind.clone().into(),
                input.source_id.clone().into(),
            ],
        ))
        .await
        .map_err(storage)?
    {
        let existing = generation_from_row(existing)?;
        if generation_matches_input(&existing, &input) {
            return Ok(existing);
        }
        return Err(QuotaError::Code("entitlement_source_conflict"));
    }

    let current = connection
        .query_one(db.stmt(
            &format!(
                "SELECT entitlement_id, generation FROM store_plan_entitlement_current
                 WHERE user_id = $1{lock}"
            ),
            vec![input.user_id.clone().into()],
        ))
        .await
        .map_err(storage)?;
    let current_generation = current
        .as_ref()
        .map(|row| row_i64(row, "generation"))
        .transpose()?;
    if current_generation != input.expected_generation {
        return Err(QuotaError::Code("entitlement_generation_conflict"));
    }
    let generation = current_generation
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(QuotaError::Code("entitlement_generation_conflict"))?;
    let id = Uuid::new_v4().to_string();
    let group_json = serde_json::to_string(&input.group_ids).map_err(serialize)?;
    let quota_json = serde_json::to_string(&input.quotas).map_err(serialize)?;
    let created_at = Utc::now().to_rfc3339();
    connection
        .execute(db.stmt(
            "INSERT INTO store_plan_entitlement_generations
                (id, user_id, generation, product_id, product_name, starts_at, ends_at,
                 rate_numerator, rate_denominator, group_ids, quota_json,
                 source_kind, source_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            vec![
                id.clone().into(),
                input.user_id.clone().into(),
                generation.into(),
                input.product_id.clone().into(),
                input.product_name.clone().into(),
                timestamp(input.starts_at).into(),
                timestamp(input.ends_at).into(),
                input.rate_numerator.clone().into(),
                input.rate_denominator.clone().into(),
                group_json.into(),
                quota_json.into(),
                input.source_kind.clone().into(),
                input.source_id.clone().into(),
                created_at.clone().into(),
            ],
        ))
        .await
        .map_err(storage)?;
    connection
        .execute(db.stmt(
            "INSERT INTO store_plan_entitlement_lifecycle
                (entitlement_id, suspended_at, suspension_reason, revoked_at,
                 revocation_reason, updated_at)
             VALUES ($1, NULL, NULL, NULL, NULL, $2)",
            vec![id.clone().into(), created_at.clone().into()],
        ))
        .await
        .map_err(storage)?;
    if let Some(expected) = current_generation {
        let changed = connection
            .execute(db.stmt(
                "UPDATE store_plan_entitlement_current
                 SET entitlement_id = $2, generation = $3, updated_at = $4
                 WHERE user_id = $1 AND generation = $5",
                vec![
                    input.user_id.clone().into(),
                    id.clone().into(),
                    generation.into(),
                    created_at.into(),
                    expected.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if changed.rows_affected() != 1 {
            return Err(QuotaError::Code("entitlement_generation_conflict"));
        }
    } else {
        connection
            .execute(db.stmt(
                "INSERT INTO store_plan_entitlement_current
                    (user_id, entitlement_id, generation, updated_at)
                 VALUES ($1, $2, $3, $4)",
                vec![
                    input.user_id.clone().into(),
                    id.clone().into(),
                    generation.into(),
                    created_at.into(),
                ],
            ))
            .await
            .map_err(storage)?;
    }
    Ok(EntitlementGeneration {
        id,
        user_id: input.user_id,
        generation,
        product_id: input.product_id,
        product_name: input.product_name,
        starts_at: input.starts_at,
        ends_at: input.ends_at,
        rate_numerator: input.rate_numerator,
        rate_denominator: input.rate_denominator,
        group_ids: input.group_ids,
        quotas: input.quotas,
        source_kind: input.source_kind,
        source_id: input.source_id,
    })
}

pub(crate) async fn reserve_tx<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    input: QuotaReservationInput,
    expected_entitlement: Option<(String, i64)>,
) -> Result<QuotaReservation, QuotaError> {
    let lock = if db.is_postgres() { " FOR UPDATE" } else { "" };
    let current = connection
        .query_one(db.stmt(
            &format!(
                "SELECT entitlement_id, generation FROM store_plan_entitlement_current
                 WHERE user_id = $1{lock}"
            ),
            vec![input.user_id.clone().into()],
        ))
        .await
        .map_err(storage)?;
    let current_binding = current
        .as_ref()
        .map(|row| {
            Ok::<(String, i64), QuotaError>((
                row_string(row, "entitlement_id")?,
                row_i64(row, "generation")?,
            ))
        })
        .transpose()?;
    if expected_entitlement.is_some() && current_binding != expected_entitlement {
        return Err(QuotaError::Code("plan_entitlement_inactive"));
    }
    if let Some(row) = connection
        .query_one(db.stmt(
            &format!(
                "SELECT id, request_id, user_id, entitlement_id, generation,
                        maximum_nano_usd, reserved_fen_cny, rate_numerator,
                        rate_denominator, pricing_revision, actual_nano_usd,
                        actual_fen_cny, state, admitted_at, terminal_at
                 FROM store_quota_reservations WHERE request_id = $1{lock}"
            ),
            vec![input.request_id.clone().into()],
        ))
        .await
        .map_err(storage)?
    {
        let existing = reservation_from_row(db, connection, row).await?;
        if existing.user_id == input.user_id
            && existing.maximum_nano_usd == input.maximum_nano_usd
            && existing.pricing_revision == input.pricing_revision
            && current_binding.as_ref()
                == Some(&(existing.entitlement_id.clone(), existing.generation))
        {
            return Ok(existing);
        }
        return Err(QuotaError::Code("quota_idempotency_conflict"));
    }

    require_passed_gate(db, connection).await?;
    let hold = count(
        db,
        connection,
        "SELECT COUNT(*) AS value FROM store_balance_holds WHERE user_id = $1 AND active = 1",
        vec![input.user_id.clone().into()],
    )
    .await?;
    if hold != 0 {
        return Err(QuotaError::Code("plan_payment_hold"));
    }
    let blocked = count(
        db,
        connection,
        "SELECT COUNT(*) AS value FROM store_quota_admission_blocks
         WHERE user_id = $1 AND cleared_at IS NULL",
        vec![input.user_id.clone().into()],
    )
    .await?;
    if blocked != 0 {
        return Err(QuotaError::Code("plan_quota_violation_blocked"));
    }
    let row = connection
        .query_one(db.stmt(
            &format!(
                "SELECT g.id, g.user_id, g.generation, g.product_id, g.product_name,
                        g.starts_at, g.ends_at, g.rate_numerator, g.rate_denominator,
                        g.group_ids, g.quota_json, g.source_kind, g.source_id,
                        l.suspended_at, l.revoked_at
                 FROM store_plan_entitlement_current p
                 JOIN store_plan_entitlement_generations g ON g.id = p.entitlement_id
                 JOIN store_plan_entitlement_lifecycle l ON l.entitlement_id = g.id
                 WHERE p.user_id = $1{lock}"
            ),
            vec![input.user_id.clone().into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(QuotaError::Code("plan_entitlement_inactive"))?;
    if row_optional_string(&row, "suspended_at")?.is_some()
        || row_optional_string(&row, "revoked_at")?.is_some()
    {
        return Err(QuotaError::Code("plan_entitlement_inactive"));
    }
    let entitlement = generation_from_row(row)?;
    if input.now < entitlement.starts_at || input.now >= entitlement.ends_at {
        return Err(QuotaError::Code("plan_entitlement_inactive"));
    }
    let rate_numerator = parse_canonical(&entitlement.rate_numerator)?;
    let rate_denominator = parse_canonical(&entitlement.rate_denominator)?;
    let reserved_fen = ceil_product_ratio(
        input.maximum_nano_usd,
        rate_numerator,
        rate_denominator
            .checked_mul(NANO_USD_PER_CENT)
            .ok_or(QuotaError::Code("quota_amount_overflow"))?,
    )?;

    let mut bindings = Vec::with_capacity(entitlement.quotas.len());
    for rule in &entitlement.quotas {
        let window = quota_window(rule.window_kind, rule.window_seconds, input.now)?;
        bindings.push(BucketBinding {
            rule: rule.clone(),
            id: bucket_id(&entitlement.id, entitlement.generation, rule, &window),
            window,
        });
    }
    bindings.sort_by(|left, right| {
        timestamp(left.window.end)
            .cmp(&timestamp(right.window.end))
            .then_with(|| left.rule.id.as_bytes().cmp(right.rule.id.as_bytes()))
            .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
    });

    for binding in &bindings {
        connection
            .execute(db.stmt(
                "INSERT INTO store_quota_buckets
                    (id, entitlement_id, generation, quota_rule_id, window_kind,
                     window_start, window_end, settled_fen_cny, reserved_fen_cny,
                     quota_fen_cny, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, '0', '0', $8, $9)
                 ON CONFLICT (id) DO NOTHING",
                vec![
                    binding.id.clone().into(),
                    entitlement.id.clone().into(),
                    entitlement.generation.into(),
                    binding.rule.id.clone().into(),
                    binding.rule.window_kind.as_str().into(),
                    timestamp(binding.window.start).into(),
                    timestamp(binding.window.end).into(),
                    binding.rule.quota_fen_cny.clone().into(),
                    timestamp(input.now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
    }

    let rows = connection
        .query_all(db.stmt(
            active_bucket_lock_sql(db.is_postgres()),
            vec![
                entitlement.id.clone().into(),
                entitlement.generation.into(),
                timestamp(input.now).into(),
            ],
        ))
        .await
        .map_err(storage)?;
    let mut used_by_rule = std::collections::BTreeMap::<String, i128>::new();
    let mut reserved_by_bucket = std::collections::BTreeMap::<String, i128>::new();
    for row in rows {
        let id = row_string(&row, "id")?;
        let rule_id = row_string(&row, "quota_rule_id")?;
        let settled = parse_canonical(&row_string(&row, "settled_fen_cny")?)?;
        let reserved = parse_canonical(&row_string(&row, "reserved_fen_cny")?)?;
        let used = used_by_rule.entry(rule_id).or_default();
        *used = used
            .checked_add(settled)
            .and_then(|value| value.checked_add(reserved))
            .ok_or(QuotaError::Code("quota_amount_overflow"))?;
        reserved_by_bucket.insert(id, reserved);
    }

    for binding in &bindings {
        let used = used_by_rule.get(&binding.rule.id).copied().unwrap_or(0);
        let quota = parse_canonical(&binding.rule.quota_fen_cny)?;
        if used
            .checked_add(reserved_fen)
            .ok_or(QuotaError::Code("quota_amount_overflow"))?
            > quota
        {
            return Err(QuotaError::Code("plan_quota_exhausted"));
        }
        let target_reserved = reserved_by_bucket
            .get(&binding.id)
            .copied()
            .ok_or_else(|| QuotaError::Storage("quota bucket disappeared".to_string()))?
            .checked_add(reserved_fen)
            .ok_or(QuotaError::Code("quota_amount_overflow"))?;
        connection
            .execute(db.stmt(
                "UPDATE store_quota_buckets
                 SET reserved_fen_cny = $2, updated_at = $3 WHERE id = $1",
                vec![
                    binding.id.clone().into(),
                    target_reserved.to_string().into(),
                    timestamp(input.now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
    }

    let id = Uuid::new_v4().to_string();
    connection
        .execute(db.stmt(
            "INSERT INTO store_quota_reservations
                (id, request_id, entitlement_id, generation, user_id,
                 maximum_nano_usd, reserved_fen_cny, rate_numerator, rate_denominator,
                 pricing_revision, actual_nano_usd, actual_fen_cny, state,
                 admitted_at, terminal_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     NULL, NULL, 'reserved', $11, NULL, $11)",
            vec![
                id.clone().into(),
                input.request_id.clone().into(),
                entitlement.id.clone().into(),
                entitlement.generation.into(),
                input.user_id.clone().into(),
                input.maximum_nano_usd.to_string().into(),
                reserved_fen.to_string().into(),
                entitlement.rate_numerator.clone().into(),
                entitlement.rate_denominator.clone().into(),
                input.pricing_revision.clone().into(),
                timestamp(input.now).into(),
            ],
        ))
        .await
        .map_err(storage)?;
    for binding in &bindings {
        connection
            .execute(db.stmt(
                "INSERT INTO store_quota_reservation_buckets
                    (reservation_id, bucket_id, reserved_fen_cny)
                 VALUES ($1, $2, $3)",
                vec![
                    id.clone().into(),
                    binding.id.clone().into(),
                    reserved_fen.to_string().into(),
                ],
            ))
            .await
            .map_err(storage)?;
    }
    Ok(QuotaReservation {
        id,
        request_id: input.request_id,
        user_id: input.user_id,
        entitlement_id: entitlement.id,
        generation: entitlement.generation,
        maximum_nano_usd: input.maximum_nano_usd,
        reserved_fen_cny: reserved_fen,
        rate_numerator: entitlement.rate_numerator,
        rate_denominator: entitlement.rate_denominator,
        pricing_revision: input.pricing_revision,
        actual_nano_usd: None,
        actual_fen_cny: None,
        state: QuotaTerminalState::Reserved,
        admitted_at: input.now,
        terminal_at: None,
        bucket_count: bindings.len(),
    })
}

pub(crate) async fn terminal_tx<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    reservation_id: &str,
    actual_nano_usd: Option<i128>,
    now: DateTime<Utc>,
) -> Result<QuotaReservation, QuotaError> {
    let lock = if db.is_postgres() { " FOR UPDATE" } else { "" };
    let row = connection
        .query_one(db.stmt(
            &format!(
                "SELECT id, request_id, user_id, entitlement_id, generation,
                        maximum_nano_usd, reserved_fen_cny, rate_numerator,
                        rate_denominator, pricing_revision, actual_nano_usd,
                        actual_fen_cny, state, admitted_at, terminal_at
                 FROM store_quota_reservations WHERE id = $1{lock}"
            ),
            vec![reservation_id.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(QuotaError::Code("quota_reservation_not_found"))?;
    let existing = reservation_from_row(db, connection, row).await?;
    if existing.state != QuotaTerminalState::Reserved {
        let same = match (existing.state, actual_nano_usd) {
            (QuotaTerminalState::Released, None) => true,
            (QuotaTerminalState::Settled | QuotaTerminalState::Violated, Some(actual)) => {
                existing.actual_nano_usd == Some(actual)
            }
            _ => false,
        };
        return if same {
            Ok(existing)
        } else {
            Err(QuotaError::Code("quota_terminal_conflict"))
        };
    }

    let actual_fen = actual_nano_usd
        .map(|actual| {
            round_product_ratio(
                actual,
                parse_canonical(&existing.rate_numerator)?,
                parse_canonical(&existing.rate_denominator)?
                    .checked_mul(NANO_USD_PER_CENT)
                    .ok_or(QuotaError::Code("quota_amount_overflow"))?,
            )
        })
        .transpose()?;
    let links = connection
        .query_all(db.stmt(
            reservation_bucket_lock_sql(db.is_postgres()),
            vec![reservation_id.into()],
        ))
        .await
        .map_err(storage)?;
    if links.len() != existing.bucket_count {
        return Err(QuotaError::Storage(
            "quota reservation bucket links changed".to_string(),
        ));
    }
    for link in links {
        let bucket_id = row_string(&link, "bucket_id")?;
        let linked_reserved = parse_canonical(&row_string(&link, "linked_reserved_fen_cny")?)?;
        let reserved = parse_canonical(&row_string(&link, "reserved_fen_cny")?)?
            .checked_sub(linked_reserved)
            .ok_or_else(|| {
                QuotaError::Storage("quota reservation conservation failed".to_string())
            })?;
        let settled = parse_canonical(&row_string(&link, "settled_fen_cny")?)?
            .checked_add(actual_fen.unwrap_or(0))
            .ok_or(QuotaError::Code("quota_amount_overflow"))?;
        connection
            .execute(db.stmt(
                "UPDATE store_quota_buckets
                 SET settled_fen_cny = $2, reserved_fen_cny = $3, updated_at = $4
                 WHERE id = $1",
                vec![
                    bucket_id.into(),
                    settled.to_string().into(),
                    reserved.to_string().into(),
                    timestamp(now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
    }

    let violated = actual_fen.is_some_and(|actual| actual > existing.reserved_fen_cny);
    let state = if actual_nano_usd.is_none() {
        QuotaTerminalState::Released
    } else if violated {
        QuotaTerminalState::Violated
    } else {
        QuotaTerminalState::Settled
    };
    connection
        .execute(db.stmt(
            "UPDATE store_quota_reservations
             SET actual_nano_usd = $2, actual_fen_cny = $3, state = $4,
                 terminal_at = $5, updated_at = $5
             WHERE id = $1 AND state = 'reserved'",
            vec![
                reservation_id.into(),
                actual_nano_usd.map(|value| value.to_string()).into(),
                actual_fen.map(|value| value.to_string()).into(),
                state.as_str().into(),
                timestamp(now).into(),
            ],
        ))
        .await
        .map_err(storage)?;
    if violated {
        let violation_id = Uuid::new_v4().to_string();
        connection
            .execute(db.stmt(
                "INSERT INTO store_quota_violations
                    (id, reservation_id, request_id, user_id, entitlement_id, generation,
                     reserved_fen_cny, actual_fen_cny, severity, detected_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'critical', $9)
                 ON CONFLICT (reservation_id) DO NOTHING",
                vec![
                    violation_id.clone().into(),
                    reservation_id.into(),
                    existing.request_id.clone().into(),
                    existing.user_id.clone().into(),
                    existing.entitlement_id.clone().into(),
                    existing.generation.into(),
                    existing.reserved_fen_cny.to_string().into(),
                    actual_fen.unwrap().to_string().into(),
                    timestamp(now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        connection
            .execute(db.stmt(
                "INSERT INTO store_quota_admission_blocks
                    (user_id, violation_id, entitlement_id, generation, reason,
                     blocked_at, cleared_at)
                 VALUES ($1, $2, $3, $4, 'above_reserve', $5, NULL)
                 ON CONFLICT (user_id) DO UPDATE SET
                    violation_id = excluded.violation_id,
                    entitlement_id = excluded.entitlement_id,
                    generation = excluded.generation,
                    reason = excluded.reason,
                    blocked_at = excluded.blocked_at,
                    cleared_at = NULL",
                vec![
                    existing.user_id.clone().into(),
                    violation_id.into(),
                    existing.entitlement_id.clone().into(),
                    existing.generation.into(),
                    timestamp(now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
    }
    Ok(QuotaReservation {
        actual_nano_usd,
        actual_fen_cny: actual_fen,
        state,
        terminal_at: Some(now),
        ..existing
    })
}

pub fn quota_window(
    kind: WindowKind,
    window_seconds: i64,
    now: DateTime<Utc>,
) -> Result<QuotaWindow, QuotaError> {
    if is_rolling(kind) {
        let expected = match kind {
            WindowKind::FiveHours => 18_000,
            WindowKind::TwelveHours => 43_200,
            WindowKind::Custom => window_seconds,
            _ => unreachable!(),
        };
        if window_seconds != expected
            || window_seconds <= 0
            || (kind == WindowKind::Custom
                && (window_seconds > 31_536_000 || window_seconds % 3_600 != 0))
        {
            return Err(QuotaError::Code("invalid_quota_window"));
        }
        return Ok(QuotaWindow {
            start: now,
            end: now
                .checked_add_signed(Duration::seconds(window_seconds))
                .ok_or(QuotaError::Code("invalid_quota_window"))?,
        });
    }

    let local = now.with_timezone(&Shanghai);
    let date = local.date_naive();
    let start_date = match kind {
        WindowKind::Day => date,
        WindowKind::Week => date - Duration::days(date.weekday().num_days_from_monday().into()),
        WindowKind::Month => date
            .with_day(1)
            .ok_or(QuotaError::Code("invalid_quota_window"))?,
        _ => return Err(QuotaError::Code("invalid_quota_window")),
    };
    let end_date = match kind {
        WindowKind::Day => start_date + Duration::days(1),
        WindowKind::Week => start_date + Duration::days(7),
        WindowKind::Month => {
            let (year, month) = if start_date.month() == 12 {
                (start_date.year() + 1, 1)
            } else {
                (start_date.year(), start_date.month() + 1)
            };
            chrono::NaiveDate::from_ymd_opt(year, month, 1)
                .ok_or(QuotaError::Code("invalid_quota_window"))?
        }
        _ => unreachable!(),
    };
    let local_start = Shanghai
        .from_local_datetime(&start_date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .ok_or(QuotaError::Code("invalid_quota_window"))?;
    let local_end = Shanghai
        .from_local_datetime(&end_date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .ok_or(QuotaError::Code("invalid_quota_window"))?;
    Ok(QuotaWindow {
        start: local_start.with_timezone(&Utc),
        end: local_end.with_timezone(&Utc),
    })
}

struct BucketBinding {
    rule: PlanQuota,
    id: String,
    window: QuotaWindow,
}

fn active_bucket_lock_sql(postgres: bool) -> &'static str {
    if postgres {
        concat!(
            "SELECT id, quota_rule_id, window_end, settled_fen_cny, reserved_fen_cny\n",
            "FROM store_quota_buckets\n",
            "WHERE entitlement_id = $1 AND generation = $2 AND window_end > $3\n",
            "ORDER BY window_end ASC, quota_rule_id COLLATE \"C\" ASC, ",
            "id COLLATE \"C\" ASC\n",
            "FOR UPDATE"
        )
    } else {
        concat!(
            "SELECT id, quota_rule_id, window_end, settled_fen_cny, reserved_fen_cny\n",
            "FROM store_quota_buckets\n",
            "WHERE entitlement_id = $1 AND generation = $2 AND window_end > $3\n",
            "ORDER BY window_end ASC, quota_rule_id COLLATE BINARY ASC, ",
            "id COLLATE BINARY ASC"
        )
    }
}

fn reservation_bucket_lock_sql(postgres: bool) -> &'static str {
    if postgres {
        concat!(
            "SELECT b.id AS bucket_id, l.reserved_fen_cny AS linked_reserved_fen_cny,\n",
            "b.settled_fen_cny, b.reserved_fen_cny\n",
            "FROM store_quota_reservation_buckets l\n",
            "JOIN store_quota_buckets b ON b.id = l.bucket_id\n",
            "WHERE l.reservation_id = $1\n",
            "ORDER BY b.window_end ASC, b.quota_rule_id COLLATE \"C\" ASC, ",
            "b.id COLLATE \"C\" ASC\n",
            "FOR UPDATE"
        )
    } else {
        concat!(
            "SELECT b.id AS bucket_id, l.reserved_fen_cny AS linked_reserved_fen_cny,\n",
            "b.settled_fen_cny, b.reserved_fen_cny\n",
            "FROM store_quota_reservation_buckets l\n",
            "JOIN store_quota_buckets b ON b.id = l.bucket_id\n",
            "WHERE l.reservation_id = $1\n",
            "ORDER BY b.window_end ASC, b.quota_rule_id COLLATE BINARY ASC, ",
            "b.id COLLATE BINARY ASC"
        )
    }
}

fn validate_generation_input(input: &EntitlementGenerationInput) -> Result<(), QuotaError> {
    if input.user_id.trim().is_empty()
        || input.product_id.trim().is_empty()
        || input.product_name.trim().is_empty()
        || input.product_name.trim().len() > 100
        || input.starts_at >= input.ends_at
        || !matches!(input.source_kind.as_str(), "order" | "redemption")
        || input.source_id.trim().is_empty()
        || input.quotas.is_empty()
    {
        return Err(QuotaError::Code("invalid_entitlement"));
    }
    let numerator = parse_canonical(&input.rate_numerator)?;
    let denominator = parse_canonical(&input.rate_denominator)?;
    if numerator == 0 || denominator == 0 || gcd(numerator, denominator) != 1 {
        return Err(QuotaError::Code("invalid_entitlement"));
    }
    let mut windows = std::collections::BTreeSet::new();
    for quota in &input.quotas {
        if quota.id.trim().is_empty()
            || parse_canonical(&quota.quota_fen_cny)? == 0
            || !windows.insert(quota.window_seconds)
        {
            return Err(QuotaError::Code("invalid_entitlement"));
        }
        quota_window(quota.window_kind, quota.window_seconds, input.starts_at)?;
    }
    Ok(())
}

fn generation_matches_input(
    generation: &EntitlementGeneration,
    input: &EntitlementGenerationInput,
) -> bool {
    generation.user_id == input.user_id
        && generation.product_id == input.product_id
        && generation.product_name == input.product_name
        && generation.starts_at == input.starts_at
        && generation.ends_at == input.ends_at
        && generation.rate_numerator == input.rate_numerator
        && generation.rate_denominator == input.rate_denominator
        && generation.group_ids == input.group_ids
        && generation.quotas == input.quotas
        && generation.source_kind == input.source_kind
        && generation.source_id == input.source_id
}

async fn require_passed_gate<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
) -> Result<(), QuotaError> {
    if db.is_postgres() {
        return Ok(());
    }
    let enabled = QuotaGateStore::new(db.clone())
        .plan_features_enabled_on(connection)
        .await
        .map_err(|error| QuotaError::Storage(error.to_string()))?;
    if !enabled {
        return Err(QuotaError::Code("quota_gate_unavailable"));
    }
    Ok(())
}

async fn reservation_from_row<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    row: QueryResult,
) -> Result<QuotaReservation, QuotaError> {
    let id = row_string(&row, "id")?;
    let bucket_count = count(
        db,
        connection,
        "SELECT COUNT(*) AS value FROM store_quota_reservation_buckets
         WHERE reservation_id = $1",
        vec![id.clone().into()],
    )
    .await? as usize;
    Ok(QuotaReservation {
        id,
        request_id: row_string(&row, "request_id")?,
        user_id: row_string(&row, "user_id")?,
        entitlement_id: row_string(&row, "entitlement_id")?,
        generation: row_i64(&row, "generation")?,
        maximum_nano_usd: parse_canonical(&row_string(&row, "maximum_nano_usd")?)?,
        reserved_fen_cny: parse_canonical(&row_string(&row, "reserved_fen_cny")?)?,
        rate_numerator: row_string(&row, "rate_numerator")?,
        rate_denominator: row_string(&row, "rate_denominator")?,
        pricing_revision: row_string(&row, "pricing_revision")?,
        actual_nano_usd: row_optional_string(&row, "actual_nano_usd")?
            .map(|value| parse_canonical(&value))
            .transpose()?,
        actual_fen_cny: row_optional_string(&row, "actual_fen_cny")?
            .map(|value| parse_canonical(&value))
            .transpose()?,
        state: QuotaTerminalState::parse(&row_string(&row, "state")?)?,
        admitted_at: parse_timestamp(&row_string(&row, "admitted_at")?)?,
        terminal_at: row_optional_string(&row, "terminal_at")?
            .map(|value| parse_timestamp(&value))
            .transpose()?,
        bucket_count,
    })
}

fn generation_from_row(row: QueryResult) -> Result<EntitlementGeneration, QuotaError> {
    Ok(EntitlementGeneration {
        id: row_string(&row, "id")?,
        user_id: row_string(&row, "user_id")?,
        generation: row_i64(&row, "generation")?,
        product_id: row_string(&row, "product_id")?,
        product_name: row_string(&row, "product_name")?,
        starts_at: parse_timestamp(&row_string(&row, "starts_at")?)?,
        ends_at: parse_timestamp(&row_string(&row, "ends_at")?)?,
        rate_numerator: row_string(&row, "rate_numerator")?,
        rate_denominator: row_string(&row, "rate_denominator")?,
        group_ids: serde_json::from_str(&row_string(&row, "group_ids")?).map_err(serialize)?,
        quotas: serde_json::from_str(&row_string(&row, "quota_json")?).map_err(serialize)?,
        source_kind: row_string(&row, "source_kind")?,
        source_id: row_string(&row, "source_id")?,
    })
}

fn bucket_id(
    entitlement_id: &str,
    generation: i64,
    rule: &PlanQuota,
    window: &QuotaWindow,
) -> String {
    let payload = format!(
        "{entitlement_id}\0{generation}\0{}\0{}\0{}",
        rule.id,
        timestamp(window.start),
        timestamp(window.end)
    );
    let digest = Sha256::digest(payload.as_bytes());
    let mut id = String::from("quota-bucket-");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(id, "{byte:02x}");
    }
    id
}

fn is_rolling(kind: WindowKind) -> bool {
    matches!(
        kind,
        WindowKind::FiveHours | WindowKind::TwelveHours | WindowKind::Custom
    )
}

fn parse_canonical(value: &str) -> Result<i128, QuotaError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(QuotaError::Code("quota_amount_overflow"));
    }
    value
        .parse()
        .map_err(|_| QuotaError::Code("quota_amount_overflow"))
}

fn ceil_product_ratio(
    value: i128,
    multiplier: i128,
    denominator: i128,
) -> Result<i128, QuotaError> {
    let product = value
        .checked_mul(multiplier)
        .ok_or(QuotaError::Code("quota_amount_overflow"))?;
    let quotient = product / denominator;
    let remainder = product % denominator;
    quotient
        .checked_add(i128::from(remainder != 0))
        .ok_or(QuotaError::Code("quota_amount_overflow"))
}

fn round_product_ratio(
    value: i128,
    multiplier: i128,
    denominator: i128,
) -> Result<i128, QuotaError> {
    let product = value
        .checked_mul(multiplier)
        .ok_or(QuotaError::Code("quota_amount_overflow"))?;
    let quotient = product / denominator;
    let remainder = product % denominator;
    let doubled = remainder
        .checked_mul(2)
        .ok_or(QuotaError::Code("quota_amount_overflow"))?;
    quotient
        .checked_add(i128::from(doubled >= denominator))
        .ok_or(QuotaError::Code("quota_amount_overflow"))
}

fn gcd(mut left: i128, mut right: i128) -> i128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

async fn count<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<i64, QuotaError> {
    connection
        .query_one(db.stmt(sql, values))
        .await
        .map_err(storage)?
        .ok_or_else(|| QuotaError::Storage("count returned no row".to_string()))?
        .try_get("", "value")
        .map_err(storage)
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, QuotaError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_string(row: &QueryResult, column: &str) -> Result<Option<String>, QuotaError> {
    row.try_get("", column).map_err(storage)
}

fn row_i64(row: &QueryResult, column: &str) -> Result<i64, QuotaError> {
    row.try_get("", column).map_err(storage)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, QuotaError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| QuotaError::Storage(error.to_string()))
}

fn parse_optional_timestamp(value: Option<String>) -> Result<Option<DateTime<Utc>>, QuotaError> {
    value.map(|value| parse_timestamp(&value)).transpose()
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn storage(error: impl std::fmt::Display) -> QuotaError {
    QuotaError::Storage(error.to_string())
}

fn serialize(error: serde_json::Error) -> QuotaError {
    QuotaError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{active_bucket_lock_sql, reservation_bucket_lock_sql};

    #[test]
    fn postgres_bucket_lock_sql_uses_the_required_byte_order_and_row_lock() {
        assert_eq!(
            active_bucket_lock_sql(true),
            "SELECT id, quota_rule_id, window_end, settled_fen_cny, reserved_fen_cny\n\
             FROM store_quota_buckets\n\
             WHERE entitlement_id = $1 AND generation = $2 AND window_end > $3\n\
             ORDER BY window_end ASC, quota_rule_id COLLATE \"C\" ASC, id COLLATE \"C\" ASC\n\
             FOR UPDATE"
        );
    }

    #[test]
    fn sqlite_bucket_lock_sql_keeps_the_same_order_without_for_update() {
        assert_eq!(
            active_bucket_lock_sql(false),
            "SELECT id, quota_rule_id, window_end, settled_fen_cny, reserved_fen_cny\n\
             FROM store_quota_buckets\n\
             WHERE entitlement_id = $1 AND generation = $2 AND window_end > $3\n\
             ORDER BY window_end ASC, quota_rule_id COLLATE BINARY ASC, id COLLATE BINARY ASC"
        );
    }

    #[test]
    fn postgres_terminal_bucket_lock_uses_the_admission_lock_order() {
        assert_eq!(
            reservation_bucket_lock_sql(true),
            "SELECT b.id AS bucket_id, l.reserved_fen_cny AS linked_reserved_fen_cny,\n\
                    b.settled_fen_cny, b.reserved_fen_cny\n\
             FROM store_quota_reservation_buckets l\n\
             JOIN store_quota_buckets b ON b.id = l.bucket_id\n\
             WHERE l.reservation_id = $1\n\
             ORDER BY b.window_end ASC, b.quota_rule_id COLLATE \"C\" ASC, b.id COLLATE \"C\" ASC\n\
             FOR UPDATE"
        );
    }
}
