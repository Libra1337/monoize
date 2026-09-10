//! Persistence for the sales commission subsystem (`spec/sales-commission.spec.md`).
//!
//! Amounts crossing this boundary are Coin minor units as canonical integer strings, matching
//! the money convention of `store-billing.spec.md` SB-0.7.

use crate::db::DbPool;
use crate::store_billing::sales::{
    DEFAULT_COMMISSION_RATE_BP, MAX_COMMISSION_RATE_BP, MIN_WITHDRAWAL_MINOR, SalesError,
    claim_commission, generate_code, normalize_code,
};
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Settings key holding the commission rate in basis points (SC-1.1).
const COMMISSION_RATE_KEY: &str = "store.sales_commission_rate_bp";

/// SC-4.7 claim rate limit.
const CLAIM_ATTEMPTS_PER_MINUTE: i64 = 10;
const CLAIM_FAILURES_BEFORE_COOLDOWN: i64 = 5;
const CLAIM_FAILURE_WINDOW_MINUTES: i64 = 15;
const CLAIM_COOLDOWN_MINUTES: i64 = 30;

/// How many code collisions to tolerate before giving up (SC-7.2).
const CODE_GENERATION_ATTEMPTS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SalesStoreError {
    #[error("sales input is invalid")]
    InvalidInput,
    #[error("sales code is invalid")]
    CodeInvalid,
    #[error("an agent may not use their own code")]
    SelfReferral,
    #[error("discount exceeds the commission rate")]
    DiscountAboveRate,
    #[error("order amount is below the sales-code minimum")]
    AmountTooSmall,
    #[error("the caller is not a sales agent")]
    NotAgent,
    #[error("the order and user do not correspond")]
    ClaimMismatch,
    #[error("this order already has commission")]
    ClaimAlreadyCredited,
    #[error("claim rate limit exceeded")]
    ClaimRateLimited,
    #[error("a withdrawal is already pending")]
    WithdrawalPending,
    #[error("withdrawal is not pending")]
    WithdrawalNotPending,
    #[error("withdrawal was not found")]
    WithdrawalNotFound,
    #[error("insufficient commission balance")]
    InsufficientBalance,
    #[error("sales storage failure: {0}")]
    Storage(String),
}

impl From<SalesError> for SalesStoreError {
    fn from(error: SalesError) -> Self {
        match error {
            SalesError::DiscountAboveRate => Self::DiscountAboveRate,
            SalesError::AmountTooSmall => Self::AmountTooSmall,
            SalesError::RateOutOfRange | SalesError::InvalidAmount => Self::InvalidInput,
        }
    }
}

fn storage(error: impl std::fmt::Display) -> SalesStoreError {
    SalesStoreError::Storage(error.to_string())
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, SalesStoreError> {
    row.try_get("", column).map_err(storage)
}

fn row_i64(row: &QueryResult, column: &str) -> Result<i64, SalesStoreError> {
    row.try_get("", column).map_err(storage)
}

/// Parses a canonical nonnegative integer amount.
fn parse_minor(value: &str) -> Result<i128, SalesStoreError> {
    let parsed = parse_signed_minor(value)?;
    if parsed < 0 {
        return Err(SalesStoreError::InvalidInput);
    }
    Ok(parsed)
}

/// Parses a canonical integer amount that may be negative (SC-3.4c).
///
/// A commission balance goes negative when a refund reverses an accrual the agent already
/// withdrew. That debt is carried until later commission repays it, so the balance column is
/// signed while every individual amount remains nonnegative.
fn parse_signed_minor(value: &str) -> Result<i128, SalesStoreError> {
    let parsed: i128 = value.parse().map_err(|_| SalesStoreError::InvalidInput)?;
    if parsed.to_string() != value {
        return Err(SalesStoreError::InvalidInput);
    }
    Ok(parsed)
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

/// One agent as the Sales surface sees itself (SC-6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesAgent {
    pub user_id: String,
    pub username: String,
    pub code: String,
    pub discount_bp: i64,
    /// Withdrawable now; the same value as `settlement.available_minor` (SC-6.9).
    pub commission_balance_minor: String,
    pub enabled: bool,
    pub created_at: String,
    pub settlement: SalesSettlement,
}

/// What an agent has earned and where it currently sits (SC-6.9).
///
/// The four satisfy `accrued = available + pending_withdrawal + withdrawn`, which lets a
/// reader check them against each other rather than trusting a single balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesSettlement {
    /// Sum of commission over entries that were not reversed.
    pub accrued_minor: String,
    /// The current balance: earned, not yet requested.
    pub available_minor: String,
    /// Requested and awaiting a decision; already deducted from the balance.
    pub pending_withdrawal_minor: String,
    /// Paid out.
    pub withdrawn_minor: String,
}

/// One accrual (SC-D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesCommissionEntry {
    pub id: String,
    pub order_number: String,
    pub base_minor: String,
    pub commission_minor: String,
    pub discount_bp: i64,
    pub commission_rate_bp: i64,
    pub origin: String,
    pub reversed_at: Option<String>,
    pub created_at: String,
}

/// One time window of the SC-6.2 aggregates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesWindow {
    pub sales_minor: String,
    pub commission_minor: String,
    pub order_count: i64,
}

/// One accrual as the Admin sees it (SC-7.6), carrying the identities SC-6.3 withholds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminSalesEntry {
    pub id: String,
    pub agent_user_id: String,
    pub agent_username: String,
    pub order_number: String,
    pub buyer_user_id: String,
    pub base_minor: String,
    pub commission_minor: String,
    pub discount_bp: i64,
    pub commission_rate_bp: i64,
    pub origin: String,
    pub reversed_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalesWithdrawal {
    pub id: String,
    pub agent_user_id: String,
    pub agent_username: String,
    pub amount_minor: String,
    pub state: String,
    pub requested_at: String,
    pub decided_at: Option<String>,
    pub decision_note: Option<String>,
}

/// What the Admin receives once, at agent creation (SC-7.2a).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreatedSalesAgent {
    pub agent: SalesAgent,
    /// Returned exactly once; only the hash is stored.
    pub password: String,
}

/// A resolved code, ready to price an order (SC-2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSalesCode {
    pub agent_user_id: String,
    pub code: String,
    pub discount_bp: i64,
    pub commission_rate_bp: i64,
}

#[derive(Clone)]
pub struct SalesStore {
    db: DbPool,
}

impl SalesStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// Reads the configured commission rate, falling back to the SC-1.1 default.
    pub async fn commission_rate_bp(&self) -> Result<i64, SalesStoreError> {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT value FROM system_settings WHERE key = $1",
                vec![COMMISSION_RATE_KEY.into()],
            ))
            .await
            .map_err(storage)?;
        let Some(row) = row else {
            return Ok(DEFAULT_COMMISSION_RATE_BP);
        };
        let parsed: i64 = row_string(&row, "value")?
            .parse()
            .map_err(|_| SalesStoreError::InvalidInput)?;
        if !(0..=MAX_COMMISSION_RATE_BP).contains(&parsed) {
            return Err(SalesStoreError::InvalidInput);
        }
        Ok(parsed)
    }

    /// SC-7.5: the new rate may not fall below any enabled agent's discount, which would
    /// otherwise make that agent's commission negative on the next sale.
    pub async fn set_commission_rate_bp(&self, rate_bp: i64) -> Result<i64, SalesStoreError> {
        if !(0..=MAX_COMMISSION_RATE_BP).contains(&rate_bp) {
            return Err(SalesStoreError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(storage)?;
        let conflicting = tx
            .query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM sales_agents
                 WHERE enabled = 1 AND discount_bp > $1",
                vec![rate_bp.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| storage("count returned no row"))?;
        if row_i64(&conflicting, "value")? > 0 {
            return Err(SalesStoreError::DiscountAboveRate);
        }
        let now = timestamp(Utc::now());
        tx.execute(self.db.stmt(
            "INSERT INTO system_settings (key, value, updated_at) VALUES ($1, $2, $3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                                            updated_at = excluded.updated_at",
            vec![COMMISSION_RATE_KEY.into(), rate_bp.to_string().into(), now.into()],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(rate_bp)
    }

    /// SC-2.2: resolves a submitted code to an enabled agent.
    pub async fn resolve_code(&self, input: &str) -> Result<ResolvedSalesCode, SalesStoreError> {
        let normalized = normalize_code(input).ok_or(SalesStoreError::CodeInvalid)?;
        let rate_bp = self.commission_rate_bp().await?;
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT user_id, code, discount_bp FROM sales_agents
                 WHERE code = $1 AND enabled = 1",
                vec![normalized.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(SalesStoreError::CodeInvalid)?;
        let discount_bp = row_i64(&row, "discount_bp")?;
        Ok(ResolvedSalesCode {
            agent_user_id: row_string(&row, "user_id")?,
            code: row_string(&row, "code")?,
            // A discount recorded above the current rate would price the order below cost.
            // Clamping silently would hide it, so the order is refused instead.
            discount_bp: if discount_bp > rate_bp {
                return Err(SalesStoreError::DiscountAboveRate);
            } else {
                discount_bp
            },
            commission_rate_bp: rate_bp,
        })
    }

    pub async fn agent_for_user(
        &self,
        user_id: &str,
    ) -> Result<Option<SalesAgent>, SalesStoreError> {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT a.user_id, u.username, a.code, a.discount_bp,
                        a.commission_balance_fen, a.enabled, a.created_at,
                        (SELECT COALESCE(SUM(CAST(e.commission_fen AS INTEGER)), 0)
                         FROM sales_commission_entries e
                         WHERE e.agent_user_id = a.user_id AND e.reversed_at IS NULL)
                        AS accrued_fen,
                        (SELECT COALESCE(SUM(CAST(w.amount_fen AS INTEGER)), 0)
                         FROM sales_withdrawals w
                         WHERE w.agent_user_id = a.user_id AND w.state = 'requested')
                        AS pending_withdrawal_fen,
                        (SELECT COALESCE(SUM(CAST(w.amount_fen AS INTEGER)), 0)
                         FROM sales_withdrawals w
                         WHERE w.agent_user_id = a.user_id AND w.state = 'paid')
                        AS withdrawn_fen
                 FROM sales_agents a JOIN users u ON u.id = a.user_id
                 WHERE a.user_id = $1",
                vec![user_id.into()],
            ))
            .await
            .map_err(storage)?;
        row.map(|row| self.row_to_agent(&row)).transpose()
    }

    fn row_to_agent(&self, row: &QueryResult) -> Result<SalesAgent, SalesStoreError> {
        Ok(SalesAgent {
            user_id: row_string(row, "user_id")?,
            username: row_string(row, "username")?,
            code: row_string(row, "code")?,
            discount_bp: row_i64(row, "discount_bp")?,
            commission_balance_minor: row_string(row, "commission_balance_fen")?,
            enabled: row_i64(row, "enabled")? == 1,
            created_at: row_string(row, "created_at")?,
            settlement: SalesSettlement {
                accrued_minor: row_i64(row, "accrued_fen")?.to_string(),
                available_minor: row_string(row, "commission_balance_fen")?,
                pending_withdrawal_minor: row_i64(row, "pending_withdrawal_fen")?.to_string(),
                withdrawn_minor: row_i64(row, "withdrawn_fen")?.to_string(),
            },
        })
    }

    pub async fn list_agents(&self) -> Result<Vec<SalesAgent>, SalesStoreError> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                // The three settlement figures are correlated subqueries rather than a
                // second round of per-agent queries, so the roster stays one statement.
                "SELECT a.user_id, u.username, a.code, a.discount_bp,
                        a.commission_balance_fen, a.enabled, a.created_at,
                        (SELECT COALESCE(SUM(CAST(e.commission_fen AS INTEGER)), 0)
                         FROM sales_commission_entries e
                         WHERE e.agent_user_id = a.user_id AND e.reversed_at IS NULL)
                        AS accrued_fen,
                        (SELECT COALESCE(SUM(CAST(w.amount_fen AS INTEGER)), 0)
                         FROM sales_withdrawals w
                         WHERE w.agent_user_id = a.user_id AND w.state = 'requested')
                        AS pending_withdrawal_fen,
                        (SELECT COALESCE(SUM(CAST(w.amount_fen AS INTEGER)), 0)
                         FROM sales_withdrawals w
                         WHERE w.agent_user_id = a.user_id AND w.state = 'paid')
                        AS withdrawn_fen
                 FROM sales_agents a JOIN users u ON u.id = a.user_id
                 ORDER BY a.created_at DESC, a.user_id ASC
                 LIMIT 200",
                vec![],
            ))
            .await
            .map_err(storage)?;
        rows.iter().map(|row| self.row_to_agent(row)).collect()
    }

    /// Generates a code that is free on both unique indexes (SC-7.2).
    ///
    /// The username equals the code, so a collision on either index must retry. Checking both
    /// before insert keeps the caller from having to distinguish which index rejected it.
    pub async fn available_code(&self) -> Result<String, SalesStoreError> {
        for _ in 0..CODE_GENERATION_ATTEMPTS {
            let candidate = generate_code();
            let taken = self
                .db
                .read()
                .query_one(self.db.stmt(
                    "SELECT COUNT(*) AS value FROM sales_agents WHERE code = $1
                     UNION ALL SELECT COUNT(*) FROM users WHERE username = $1",
                    vec![candidate.clone().into()],
                ))
                .await
                .map_err(storage)?;
            let collides = match taken {
                Some(row) => row_i64(&row, "value")? > 0,
                None => false,
            };
            if !collides {
                return Ok(candidate);
            }
        }
        Err(storage("exhausted sales code generation attempts"))
    }

    /// Inserts the agent row for an already-created user (SC-7.2).
    pub async fn insert_agent(
        &self,
        user_id: &str,
        code: &str,
        discount_bp: i64,
    ) -> Result<SalesAgent, SalesStoreError> {
        let rate_bp = self.commission_rate_bp().await?;
        if !(0..=rate_bp).contains(&discount_bp) {
            return Err(SalesStoreError::DiscountAboveRate);
        }
        let now = timestamp(Utc::now());
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "INSERT INTO sales_agents
                    (user_id, code, discount_bp, commission_balance_fen, enabled,
                     created_at, updated_at)
                 VALUES ($1, $2, $3, '0', 1, $4, $4)",
                vec![user_id.into(), code.into(), discount_bp.into(), now.into()],
            ))
            .await
            .map_err(storage)?;
        self.agent_for_user(user_id)
            .await?
            .ok_or_else(|| storage("agent row missing after insert"))
    }

    /// SC-7.3.
    /// Sets an agent's enabled state (SC-7.3).
    ///
    /// The discount is deliberately not settable here: it is the agent's own commission
    /// being given away, so only the agent may change it (SC-1.2a).
    pub async fn set_agent_enabled(
        &self,
        user_id: &str,
        enabled: bool,
    ) -> Result<SalesAgent, SalesStoreError> {
        let now = timestamp(Utc::now());
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "UPDATE sales_agents SET enabled = $2, updated_at = $3
                 WHERE user_id = $1",
                vec![user_id.into(), i64::from(enabled).into(), now.into()],
            ))
            .await
            .map_err(storage)?;
        self.agent_for_user(user_id)
            .await?
            .ok_or(SalesStoreError::NotAgent)
    }

    /// SC-6.2 aggregates over non-reversed entries.
    /// Sets an agent's own discount (SC-1.2a).
    ///
    /// Bounded by the current commission rate because the discount is funded from the
    /// commission; a larger discount would make the agent's own commission negative.
    pub async fn set_own_discount(
        &self,
        user_id: &str,
        discount_bp: i64,
    ) -> Result<SalesAgent, SalesStoreError> {
        let rate_bp = self.commission_rate_bp().await?;
        if !(0..=rate_bp).contains(&discount_bp) {
            return Err(SalesStoreError::DiscountAboveRate);
        }
        let now = timestamp(Utc::now());
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "UPDATE sales_agents SET discount_bp = $2, updated_at = $3
                 WHERE user_id = $1",
                vec![user_id.into(), discount_bp.into(), now.into()],
            ))
            .await
            .map_err(storage)?;
        self.agent_for_user(user_id)
            .await?
            .ok_or(SalesStoreError::NotAgent)
    }

    pub async fn windows(
        &self,
        agent_user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(SalesWindow, SalesWindow, SalesWindow), SalesStoreError> {
        let today = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
            .ok_or_else(|| storage("could not derive the start of day"))?;
        let mut windows = Vec::with_capacity(3);
        for since in [today, now - Duration::days(7), now - Duration::days(30)] {
            let row = self
                .db
                .read()
                .query_one(self.db.stmt(
                    "SELECT COUNT(*) AS order_count,
                            COALESCE(SUM(CAST(base_fen AS INTEGER)), 0) AS sales_minor,
                            COALESCE(SUM(CAST(commission_fen AS INTEGER)), 0) AS commission_minor
                     FROM sales_commission_entries
                     WHERE agent_user_id = $1 AND reversed_at IS NULL AND created_at >= $2",
                    vec![agent_user_id.into(), timestamp(since).into()],
                ))
                .await
                .map_err(storage)?
                .ok_or_else(|| storage("aggregate returned no row"))?;
            windows.push(SalesWindow {
                sales_minor: row_i64(&row, "sales_minor")?.to_string(),
                commission_minor: row_i64(&row, "commission_minor")?.to_string(),
                order_count: row_i64(&row, "order_count")?,
            });
        }
        let mut drain = windows.into_iter();
        Ok((
            drain.next().expect("today"),
            drain.next().expect("7d"),
            drain.next().expect("30d"),
        ))
    }

    /// SC-6.3. Deliberately omits buyer identity beyond what the agent already supplies.
    pub async fn list_entries(
        &self,
        agent_user_id: &str,
    ) -> Result<Vec<SalesCommissionEntry>, SalesStoreError> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT id, order_number, base_fen, commission_fen, discount_bp,
                        commission_rate_bp, origin, reversed_at, created_at
                 FROM sales_commission_entries WHERE agent_user_id = $1
                 ORDER BY created_at DESC, id ASC LIMIT 100",
                vec![agent_user_id.into()],
            ))
            .await
            .map_err(storage)?;
        rows.iter()
            .map(|row| {
                Ok(SalesCommissionEntry {
                    id: row_string(row, "id")?,
                    order_number: row_string(row, "order_number")?,
                    base_minor: row_string(row, "base_fen")?,
                    commission_minor: row_string(row, "commission_fen")?,
                    discount_bp: row_i64(row, "discount_bp")?,
                    commission_rate_bp: row_i64(row, "commission_rate_bp")?,
                    origin: row_string(row, "origin")?,
                    reversed_at: row.try_get("", "reversed_at").ok(),
                    created_at: row_string(row, "created_at")?,
                })
            })
            .collect()
    }

    /// SC-7.6: entries across every agent, optionally filtered to one.
    pub async fn list_entries_admin(
        &self,
        agent_user_id: Option<&str>,
    ) -> Result<Vec<AdminSalesEntry>, SalesStoreError> {
        let (filter, values) = match agent_user_id {
            Some(id) => ("WHERE e.agent_user_id = $1", vec![id.into()]),
            None => ("", vec![]),
        };
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                &format!(
                    "SELECT e.id, e.agent_user_id, u.username AS agent_username,
                            e.order_number, e.buyer_user_id, e.base_fen, e.commission_fen,
                            e.discount_bp, e.commission_rate_bp, e.origin, e.reversed_at,
                            e.created_at
                     FROM sales_commission_entries e
                     JOIN users u ON u.id = e.agent_user_id
                     {filter}
                     ORDER BY e.created_at DESC, e.id ASC LIMIT 100"
                ),
                values,
            ))
            .await
            .map_err(storage)?;
        rows.iter()
            .map(|row| {
                Ok(AdminSalesEntry {
                    id: row_string(row, "id")?,
                    agent_user_id: row_string(row, "agent_user_id")?,
                    agent_username: row_string(row, "agent_username")?,
                    order_number: row_string(row, "order_number")?,
                    buyer_user_id: row_string(row, "buyer_user_id")?,
                    base_minor: row_string(row, "base_fen")?,
                    commission_minor: row_string(row, "commission_fen")?,
                    discount_bp: row_i64(row, "discount_bp")?,
                    commission_rate_bp: row_i64(row, "commission_rate_bp")?,
                    origin: row_string(row, "origin")?,
                    reversed_at: row.try_get("", "reversed_at").ok(),
                    created_at: row_string(row, "created_at")?,
                })
            })
            .collect()
    }

    /// SC-4: a retroactive claim.
    ///
    /// The single success per order is enforced by the unique index on `order_id`, not by
    /// counting attempts, so a failure leaves the order claimable (SC-4.6).
    pub async fn claim_order(
        &self,
        agent_user_id: &str,
        order_number: &str,
        buyer_user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<SalesCommissionEntry, SalesStoreError> {
        self.check_claim_rate(agent_user_id, now).await?;
        let rate_bp = self.commission_rate_bp().await?;

        let outcome = self
            .claim_order_inner(agent_user_id, order_number, buyer_user_id, rate_bp, now)
            .await;
        // SC-4.3 and SC-4.4 both record an attempt; only the flag differs.
        self.record_claim_attempt(agent_user_id, outcome.is_ok(), now)
            .await?;
        outcome
    }

    /// Credits an agent for a past order on an Admin's behalf (SC-7.7).
    ///
    /// The eligibility rules are identical to an agent's own claim and are evaluated against
    /// the named agent. The SC-4.7 rate limit and the attempt record are deliberately absent:
    /// that limit stops an agent from enumerating buyer identities through the error channel,
    /// and an Admin can already read any order directly.
    pub async fn claim_order_for_agent(
        &self,
        agent_user_id: &str,
        order_number: &str,
        buyer_user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<SalesCommissionEntry, SalesStoreError> {
        // A code that no longer exists has nobody to credit, so the agent must be real.
        self.agent_for_user(agent_user_id)
            .await?
            .ok_or(SalesStoreError::NotAgent)?;
        let rate_bp = self.commission_rate_bp().await?;
        self.claim_order_inner(agent_user_id, order_number, buyer_user_id, rate_bp, now)
            .await
    }

    async fn claim_order_inner(
        &self,
        agent_user_id: &str,
        order_number: &str,
        buyer_user_id: &str,
        rate_bp: i64,
        now: DateTime<Utc>,
    ) -> Result<SalesCommissionEntry, SalesStoreError> {
        if agent_user_id == buyer_user_id {
            return Err(SalesStoreError::SelfReferral);
        }
        let tx = self.db.begin_write().await.map_err(storage)?;
        // SC-4.2a: the submitted identifier may be a user ID or a username. An operator
        // reading a support ticket has the username, not the UUID, and requiring the UUID
        // made the form unusable for the case it exists to serve.
        let buyer_user_id = &resolve_buyer_identifier(&self.db, &*tx, buyer_user_id).await?;
        let order = tx
            .query_one(self.db.stmt(
                "SELECT id, user_id, payment_state, fulfillment_state, payment_currency,
                        quote_json
                 FROM store_orders WHERE order_number = $1",
                vec![order_number.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(SalesStoreError::ClaimMismatch)?;

        // A mismatched buyer and a missing order return the same error, so a claim cannot be
        // used to learn whether an order number exists.
        if &row_string(&order, "user_id")? != buyer_user_id
            || row_string(&order, "payment_state")? != "paid"
            || row_string(&order, "fulfillment_state")? != "fulfilled"
            || row_string(&order, "payment_currency")? != "CNY"
        {
            return Err(SalesStoreError::ClaimMismatch);
        }
        let order_id = row_string(&order, "id")?;
        let base_minor = face_value_minor(&row_string(&order, "quote_json")?)?;
        let commission_minor = claim_commission(base_minor, rate_bp)?;

        let existing = tx
            .query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM sales_commission_entries WHERE order_id = $1",
                vec![order_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| storage("count returned no row"))?;
        if row_i64(&existing, "value")? > 0 {
            return Err(SalesStoreError::ClaimAlreadyCredited);
        }

        let entry = SalesCommissionEntry {
            id: Uuid::new_v4().to_string(),
            order_number: order_number.to_string(),
            base_minor: base_minor.to_string(),
            commission_minor: commission_minor.to_string(),
            discount_bp: 0,
            commission_rate_bp: rate_bp,
            origin: "claim".to_string(),
            reversed_at: None,
            created_at: timestamp(now),
        };
        tx.execute(self.db.stmt(
            "INSERT INTO sales_commission_entries
                (id, agent_user_id, order_id, order_number, buyer_user_id, base_fen,
                 commission_fen, discount_bp, commission_rate_bp, origin, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, 'claim', $9)",
            vec![
                entry.id.clone().into(),
                agent_user_id.into(),
                order_id.into(),
                order_number.into(),
                buyer_user_id.into(),
                entry.base_minor.clone().into(),
                entry.commission_minor.clone().into(),
                rate_bp.into(),
                entry.created_at.clone().into(),
            ],
        ))
        .await
        .map_err(storage)?;
        credit_agent(&self.db, &*tx, agent_user_id, commission_minor, now).await?;
        tx.commit().await.map_err(storage)?;
        Ok(entry)
    }

    async fn check_claim_rate(
        &self,
        agent_user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(), SalesStoreError> {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT
                     SUM(CASE WHEN attempted_at >= $2 THEN 1 ELSE 0 END) AS recent,
                     SUM(CASE WHEN attempted_at >= $3 AND succeeded = 0 THEN 1 ELSE 0 END)
                         AS failures
                 FROM sales_claim_attempts WHERE agent_user_id = $1",
                vec![
                    agent_user_id.into(),
                    timestamp(now - Duration::minutes(1)).into(),
                    timestamp(now - Duration::minutes(CLAIM_FAILURE_WINDOW_MINUTES)).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        let Some(row) = row else { return Ok(()) };
        let recent: i64 = row.try_get("", "recent").unwrap_or(0);
        let failures: i64 = row.try_get("", "failures").unwrap_or(0);
        if recent >= CLAIM_ATTEMPTS_PER_MINUTE || failures >= CLAIM_FAILURES_BEFORE_COOLDOWN {
            return Err(SalesStoreError::ClaimRateLimited);
        }
        Ok(())
    }

    async fn record_claim_attempt(
        &self,
        agent_user_id: &str,
        succeeded: bool,
        now: DateTime<Utc>,
    ) -> Result<(), SalesStoreError> {
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "INSERT INTO sales_claim_attempts (id, agent_user_id, succeeded, attempted_at)
                 VALUES ($1, $2, $3, $4)",
                vec![
                    Uuid::new_v4().to_string().into(),
                    agent_user_id.into(),
                    i64::from(succeeded).into(),
                    timestamp(now).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// SC-5.1 and SC-5.2: the amount leaves the balance when the request is filed.
    pub async fn request_withdrawal(
        &self,
        agent_user_id: &str,
        amount_minor: i128,
        now: DateTime<Utc>,
    ) -> Result<SalesWithdrawal, SalesStoreError> {
        if amount_minor < MIN_WITHDRAWAL_MINOR {
            return Err(SalesStoreError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(storage)?;
        let pending = tx
            .query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM sales_withdrawals
                 WHERE agent_user_id = $1 AND state = 'requested'",
                vec![agent_user_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| storage("count returned no row"))?;
        if row_i64(&pending, "value")? > 0 {
            return Err(SalesStoreError::WithdrawalPending);
        }

        let agent = tx
            .query_one(self.db.stmt(
                "SELECT commission_balance_fen FROM sales_agents WHERE user_id = $1",
                vec![agent_user_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(SalesStoreError::NotAgent)?;
        // SC-5.1a: a negative balance is a debt, so no positive amount satisfies the cap.
        let balance = parse_signed_minor(&row_string(&agent, "commission_balance_fen")?)?;
        if amount_minor > balance {
            return Err(SalesStoreError::InsufficientBalance);
        }

        let id = Uuid::new_v4().to_string();
        let requested_at = timestamp(now);
        tx.execute(self.db.stmt(
            "INSERT INTO sales_withdrawals
                (id, agent_user_id, amount_fen, state, requested_at)
             VALUES ($1, $2, $3, 'requested', $4)",
            vec![
                id.clone().into(),
                agent_user_id.into(),
                amount_minor.to_string().into(),
                requested_at.clone().into(),
            ],
        ))
        .await
        .map_err(storage)?;
        set_agent_balance(&self.db, &*tx, agent_user_id, balance - amount_minor, now)
            .await?;
        tx.commit().await.map_err(storage)?;

        Ok(SalesWithdrawal {
            id,
            agent_user_id: agent_user_id.to_string(),
            agent_username: String::new(),
            amount_minor: amount_minor.to_string(),
            state: "requested".to_string(),
            requested_at,
            decided_at: None,
            decision_note: None,
        })
    }

    /// SC-5.5 and SC-5.6. A `paid` decision records an out-of-band transfer and moves no
    /// money; a `rejected` decision returns the held amount.
    pub async fn decide_withdrawal(
        &self,
        withdrawal_id: &str,
        admin_user_id: &str,
        paid: bool,
        decision_note: &str,
        now: DateTime<Utc>,
    ) -> Result<SalesWithdrawal, SalesStoreError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let row = tx
            .query_one(self.db.stmt(
                "SELECT id, agent_user_id, amount_fen, state, requested_at
                 FROM sales_withdrawals WHERE id = $1",
                vec![withdrawal_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(SalesStoreError::WithdrawalNotPending)?;
        if row_string(&row, "state")? != "requested" {
            return Err(SalesStoreError::WithdrawalNotPending);
        }
        let agent_user_id = row_string(&row, "agent_user_id")?;
        let amount_minor = parse_minor(&row_string(&row, "amount_fen")?)?;
        let decided_at = timestamp(now);

        tx.execute(self.db.stmt(
            "UPDATE sales_withdrawals
             SET state = $2, decided_at = $3, decided_by = $4, decision_note = $5
             WHERE id = $1 AND state = 'requested'",
            vec![
                withdrawal_id.into(),
                if paid { "paid" } else { "rejected" }.into(),
                decided_at.clone().into(),
                admin_user_id.into(),
                decision_note.into(),
            ],
        ))
        .await
        .map_err(storage)?;

        if !paid {
            let agent = tx
                .query_one(self.db.stmt(
                    "SELECT commission_balance_fen FROM sales_agents WHERE user_id = $1",
                    vec![agent_user_id.clone().into()],
                ))
                .await
                .map_err(storage)?
                .ok_or(SalesStoreError::NotAgent)?;
            let balance = parse_signed_minor(&row_string(&agent, "commission_balance_fen")?)?;
            set_agent_balance(&self.db, &*tx, &agent_user_id, balance + amount_minor, now)
                .await?;
        }
        tx.commit().await.map_err(storage)?;

        Ok(SalesWithdrawal {
            id: withdrawal_id.to_string(),
            agent_user_id,
            agent_username: String::new(),
            amount_minor: amount_minor.to_string(),
            state: if paid { "paid" } else { "rejected" }.to_string(),
            requested_at: row_string(&row, "requested_at")?,
            decided_at: Some(decided_at),
            decision_note: Some(decision_note.to_string()),
        })
    }

    /// SC-5.1b: the agent cancels their own pending request.
    ///
    /// SC-5.2 holds the amount at request time and SC-5.3 admits one pending withdrawal, so
    /// without this an agent who typed the wrong amount is blocked until an Admin acts on a
    /// request neither party wants.
    pub async fn cancel_withdrawal(
        &self,
        withdrawal_id: &str,
        agent_user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<SalesWithdrawal, SalesStoreError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let row = tx
            .query_one(self.db.stmt(
                "SELECT id, agent_user_id, amount_fen, state, requested_at
                 FROM sales_withdrawals WHERE id = $1 AND agent_user_id = $2",
                vec![withdrawal_id.into(), agent_user_id.into()],
            ))
            .await
            .map_err(storage)?
            // A withdrawal belonging to another agent is indistinguishable from one that does
            // not exist, so an agent cannot probe for other agents' request identifiers.
            .ok_or(SalesStoreError::WithdrawalNotFound)?;
        if row_string(&row, "state")? != "requested" {
            return Err(SalesStoreError::WithdrawalNotPending);
        }
        let amount_minor = parse_minor(&row_string(&row, "amount_fen")?)?;
        let decided_at = timestamp(now);

        tx.execute(self.db.stmt(
            "UPDATE sales_withdrawals SET state = 'cancelled', decided_at = $2
             WHERE id = $1 AND state = 'requested'",
            vec![withdrawal_id.into(), decided_at.clone().into()],
        ))
        .await
        .map_err(storage)?;
        let agent = tx
            .query_one(self.db.stmt(
                "SELECT commission_balance_fen FROM sales_agents WHERE user_id = $1",
                vec![agent_user_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(SalesStoreError::NotAgent)?;
        let balance = parse_signed_minor(&row_string(&agent, "commission_balance_fen")?)?;
        set_agent_balance(&self.db, &*tx, agent_user_id, balance + amount_minor, now).await?;
        tx.commit().await.map_err(storage)?;

        Ok(SalesWithdrawal {
            id: withdrawal_id.to_string(),
            agent_user_id: agent_user_id.to_string(),
            agent_username: String::new(),
            amount_minor: amount_minor.to_string(),
            state: "cancelled".to_string(),
            requested_at: row_string(&row, "requested_at")?,
            decided_at: Some(decided_at),
            decision_note: None,
        })
    }

    pub async fn list_withdrawals(
        &self,
        agent_user_id: Option<&str>,
    ) -> Result<Vec<SalesWithdrawal>, SalesStoreError> {
        let (filter, values) = match agent_user_id {
            Some(id) => ("WHERE w.agent_user_id = $1", vec![id.into()]),
            None => ("", vec![]),
        };
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                &format!(
                    "SELECT w.id, w.agent_user_id, u.username AS agent_username, w.amount_fen,
                            w.state, w.requested_at, w.decided_at, w.decision_note
                     FROM sales_withdrawals w JOIN users u ON u.id = w.agent_user_id
                     {filter}
                     ORDER BY w.requested_at DESC, w.id ASC LIMIT 100"
                ),
                values,
            ))
            .await
            .map_err(storage)?;
        rows.iter()
            .map(|row| {
                Ok(SalesWithdrawal {
                    id: row_string(row, "id")?,
                    agent_user_id: row_string(row, "agent_user_id")?,
                    agent_username: row_string(row, "agent_username")?,
                    amount_minor: row_string(row, "amount_fen")?,
                    state: row_string(row, "state")?,
                    requested_at: row_string(row, "requested_at")?,
                    decided_at: row.try_get("", "decided_at").ok(),
                    decision_note: row.try_get("", "decision_note").ok(),
                })
            })
            .collect()
    }
}

/// Reads the commission rate inside a caller-owned transaction (SC-1.1).
///
/// Accrual needs the rate in the same transaction as the entry it freezes onto, so it cannot
/// use the pooled read connection.
pub async fn commission_rate_for_entry<C: ConnectionTrait, E>(
    db: &DbPool,
    conn: &C,
) -> Result<i64, E>
where
    E: From<SalesStoreError>,
{
    let row = conn
        .query_one(db.stmt(
            "SELECT value FROM system_settings WHERE key = $1",
            vec![COMMISSION_RATE_KEY.into()],
        ))
        .await
        .map_err(|error| E::from(storage(error)))?;
    let Some(row) = row else {
        return Ok(DEFAULT_COMMISSION_RATE_BP);
    };
    let value = row
        .try_get::<String>("", "value")
        .map_err(|error| E::from(storage(error)))?;
    let parsed: i64 = value
        .parse()
        .map_err(|_| E::from(SalesStoreError::InvalidInput))?;
    if !(0..=MAX_COMMISSION_RATE_BP).contains(&parsed) {
        return Err(E::from(SalesStoreError::InvalidInput));
    }
    Ok(parsed)
}

/// Reverses the commission entry of a refunded order, inside the refund transaction (SC-3.4).
///
/// Returns the reversed amount when an entry existed. An order without an entry is not an
/// error: most orders carry no sales code.
pub async fn reverse_commission_for_order<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    order_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<i128>, SalesStoreError> {
    let row = conn
        .query_one(db.stmt(
            "SELECT id, agent_user_id, commission_fen FROM sales_commission_entries
             WHERE order_id = $1 AND reversed_at IS NULL",
            vec![order_id.into()],
        ))
        .await
        .map_err(storage)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let entry_id = row_string(&row, "id")?;
    let agent_user_id = row_string(&row, "agent_user_id")?;
    let commission_minor = parse_minor(&row_string(&row, "commission_fen")?)?;

    // The entry keeps its original `commission_fen` and only gains `reversed_at`, so the
    // debt's origin stays auditable after later commission repays it (SC-3.4c).
    let changed = conn
        .execute(db.stmt(
            "UPDATE sales_commission_entries SET reversed_at = $2
             WHERE id = $1 AND reversed_at IS NULL",
            vec![entry_id.into(), timestamp(now).into()],
        ))
        .await
        .map_err(storage)?;
    if changed.rows_affected() != 1 {
        // Another transaction reversed it first; the balance was adjusted there.
        return Ok(None);
    }
    credit_agent(db, conn, &agent_user_id, -commission_minor, now).await?;
    Ok(Some(commission_minor))
}

/// Adds to an agent's balance inside a caller-owned transaction.
async fn credit_agent<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    agent_user_id: &str,
    delta_minor: i128,
    now: DateTime<Utc>,
) -> Result<(), SalesStoreError> {
    let row = conn
        .query_one(db.stmt(
            "SELECT commission_balance_fen FROM sales_agents WHERE user_id = $1",
            vec![agent_user_id.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(SalesStoreError::NotAgent)?;
    // SC-3.4b: an accrual adds to the balance whatever its sign, so a debt is repaid before
    // the agent can withdraw again.
    let balance = parse_signed_minor(&row_string(&row, "commission_balance_fen")?)?;
    set_agent_balance(db, conn, agent_user_id, balance + delta_minor, now).await
}

async fn set_agent_balance<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    agent_user_id: &str,
    balance_minor: i128,
    now: DateTime<Utc>,
) -> Result<(), SalesStoreError> {
    // SC-3.4a: the balance is not clamped. A reversal after the agent already withdrew leaves
    // a negative balance, which is a debt repaid by later commission (SC-3.4b). Clamping would
    // forgive it and pay the agent again from zero on the next sale.
    conn.execute(db.stmt(
        "UPDATE sales_agents SET commission_balance_fen = $2, updated_at = $3 WHERE user_id = $1",
        vec![
            agent_user_id.into(),
            balance_minor.to_string().into(),
            timestamp(now).into(),
        ],
    ))
    .await
    .map_err(storage)?;
    Ok(())
}

/// Resolves a submitted buyer identifier to a user ID (SC-4.2a).
///
/// Accepts either the ID itself or a username. An unknown identifier is returned unchanged so
/// the caller's own mismatch error surfaces, keeping "no such user" and "wrong user for this
/// order" indistinguishable.
async fn resolve_buyer_identifier<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    submitted: &str,
) -> Result<String, SalesStoreError> {
    let row = conn
        .query_one(db.stmt(
            "SELECT id FROM users WHERE id = $1 OR username = $1",
            vec![submitted.into()],
        ))
        .await
        .map_err(storage)?;
    Ok(match row {
        Some(row) => row_string(&row, "id")?,
        None => submitted.to_string(),
    })
}

/// Reads the order face value in Coin minor units from a frozen quote (SC-3.3).
///
/// The face value is the balance quote's `recharge_minor`: the amount the buyer owes before
/// any discount. It is deliberately neither of the two neighbouring amounts.
///
/// It is not `payment_minor`, because a discount lowers that and the commission is defined
/// against the undiscounted amount, which is what keeps platform revenue independent of the
/// discount (SC-1.4).
///
/// It is not `actual_received_minor`, because a bonus raises that above what the platform was
/// paid. A product selling 100 CNY of balance with a 20 CNY bonus takes 100 CNY; accruing on
/// 120 would pay the agent 6 CNY out of 100 received and cut platform revenue to 94.
pub fn face_value_minor(quote_json: &str) -> Result<i128, SalesStoreError> {
    let quote: serde_json::Value =
        serde_json::from_str(quote_json).map_err(|_| SalesStoreError::InvalidInput)?;
    let recharge = quote
        .get("product")
        .and_then(|product| product.get("balance"))
        .and_then(|balance| balance.get("recharge_minor"))
        .and_then(serde_json::Value::as_str)
        .ok_or(SalesStoreError::InvalidInput)?;
    parse_minor(recharge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm_migration::MigratorTrait;

    /// Builds a store with one agent holding `accrued` fen of commission across two entries.
    async fn store_with_agent(entries: &[i64]) -> (SalesStore, String) {
        let db = DbPool::connect("sqlite::memory:").await.expect("connect");
        {
            let write = db.write().await;
            crate::migration::Migrator::up(&*write, None)
                .await
                .expect("migrate");
            write
                .execute_unprepared(
                    "INSERT INTO users
                        (id, username, password_hash, role, created_at, updated_at, enabled,
                         balance_nano_usd, balance_unlimited, group_id)
                     SELECT 'agent-1', 'agent-1', 'x', 'user', '2026-09-09T00:00:00Z',
                            '2026-09-09T00:00:00Z', 1, '0', 0, id
                     FROM monoize_groups WHERE is_default = 1 LIMIT 1",
                )
                .await
                .expect("seed agent user");
        }
        let store = SalesStore::new(db.clone());
        store
            .insert_agent("agent-1", "AGENT001", 0)
            .await
            .expect("insert agent");

        let total: i64 = entries.iter().sum();
        for (index, amount) in entries.iter().enumerate() {
            db.write()
                .await
                .execute(db.stmt(
                    "INSERT INTO sales_commission_entries
                        (id, agent_user_id, order_id, order_number, buyer_user_id, base_fen,
                         commission_fen, discount_bp, commission_rate_bp, origin, created_at)
                     VALUES ($1, 'agent-1', $2, $3, 'buyer', '10000', $4, 0, 500, 'code',
                             '2026-09-09T00:00:00Z')",
                    vec![
                        format!("entry-{index}").into(),
                        format!("order-{index}").into(),
                        format!("LS-{index}").into(),
                        amount.to_string().into(),
                    ],
                ))
                .await
                .expect("insert entry");
        }
        db.write()
            .await
            .execute(db.stmt(
                "UPDATE sales_agents SET commission_balance_fen = $1 WHERE user_id = 'agent-1'",
                vec![total.to_string().into()],
            ))
            .await
            .expect("set balance");
        (store, "agent-1".to_string())
    }

    /// SC-6.9a: the four figures must add up, so a reader can check them against each other.
    ///
    /// The identity is the whole reason for reporting four numbers rather than a balance, and
    /// it only holds if a request deducts at request time and a rejection returns the amount.
    #[tokio::test]
    async fn settlement_figures_account_for_every_fen() {
        let (store, agent_id) = store_with_agent(&[300, 200]).await;
        let now = Utc::now();

        let fresh = store.agent_for_user(&agent_id).await.unwrap().unwrap();
        assert_eq!(fresh.settlement.accrued_minor, "500");
        assert_eq!(fresh.settlement.available_minor, "500");
        assert_eq!(fresh.settlement.pending_withdrawal_minor, "0");
        assert_eq!(fresh.settlement.withdrawn_minor, "0");

        // Requesting moves money from available to pending without changing what was accrued.
        let paid_request = store.request_withdrawal(&agent_id, 200, now).await.unwrap();
        let requested = store.agent_for_user(&agent_id).await.unwrap().unwrap();
        assert_eq!(requested.settlement.accrued_minor, "500");
        assert_eq!(requested.settlement.available_minor, "300");
        assert_eq!(requested.settlement.pending_withdrawal_minor, "200");
        assert_eq!(requested.settlement.withdrawn_minor, "0");

        // Paying it moves pending to withdrawn, again leaving accrued alone.
        store
            .decide_withdrawal(&paid_request.id, "admin", true, "", now)
            .await
            .unwrap();
        let settled = store.agent_for_user(&agent_id).await.unwrap().unwrap();
        assert_eq!(settled.settlement.accrued_minor, "500");
        assert_eq!(settled.settlement.available_minor, "300");
        assert_eq!(settled.settlement.pending_withdrawal_minor, "0");
        assert_eq!(settled.settlement.withdrawn_minor, "200");

        // A rejected request returns to available and counts toward none of the four.
        let rejected = store.request_withdrawal(&agent_id, 100, now).await.unwrap();
        store
            .decide_withdrawal(&rejected.id, "admin", false, "no", now)
            .await
            .unwrap();
        let after = store.agent_for_user(&agent_id).await.unwrap().unwrap();
        assert_eq!(after.settlement.accrued_minor, "500");
        assert_eq!(after.settlement.available_minor, "300");
        assert_eq!(after.settlement.pending_withdrawal_minor, "0");
        assert_eq!(after.settlement.withdrawn_minor, "200");

        for agent in [fresh, requested, settled, after] {
            let s = &agent.settlement;
            let sum: i64 = s.available_minor.parse::<i64>().unwrap()
                + s.pending_withdrawal_minor.parse::<i64>().unwrap()
                + s.withdrawn_minor.parse::<i64>().unwrap();
            assert_eq!(
                s.accrued_minor.parse::<i64>().unwrap(),
                sum,
                "accrued must equal available + pending + withdrawn: {s:?}"
            );
        }
    }

    /// A reversed entry earned nothing, so it must leave the accrued total.
    #[tokio::test]
    async fn a_reversed_entry_leaves_the_accrued_total() {
        let (store, agent_id) = store_with_agent(&[300, 200]).await;
        store
            .db
            .write()
            .await
            .execute(store.db.stmt(
                "UPDATE sales_commission_entries SET reversed_at = '2026-09-09T01:00:00Z'
                 WHERE id = 'entry-1'",
                vec![],
            ))
            .await
            .expect("reverse an entry");
        let agent = store.agent_for_user(&agent_id).await.unwrap().unwrap();
        assert_eq!(
            agent.settlement.accrued_minor, "300",
            "a refunded order must not count as earned"
        );
    }

    /// Admin and the agent must not disagree about the same agent (SC-6.10).
    #[tokio::test]
    async fn the_roster_reports_the_same_totals_as_the_agent_view() {
        let (store, agent_id) = store_with_agent(&[300, 200]).await;
        store
            .request_withdrawal(&agent_id, 150, Utc::now())
            .await
            .unwrap();
        let own = store.agent_for_user(&agent_id).await.unwrap().unwrap();
        let roster = store.list_agents().await.unwrap();
        let listed = roster
            .iter()
            .find(|agent| agent.user_id == agent_id)
            .expect("the agent is listed");
        assert_eq!(listed.settlement, own.settlement);
    }

    fn balance_quote(recharge: &str, bonus: &str, received: &str) -> String {
        serde_json::json!({
            "version": 2,
            "product": {
                "kind": "balance",
                "balance": {
                    "recharge_minor": recharge,
                    "bonus_minor": bonus,
                    "actual_received_minor": received,
                }
            }
        })
        .to_string()
    }

    /// SC-3.3: the face value is the undiscounted amount owed, so a discount does not shrink
    /// the commission base. That is what holds platform revenue at 95% under SC-1.4.
    #[test]
    fn face_value_ignores_the_discount() {
        // 100 CNY face value, 1% discount: the buyer pays 99 and receives 100.
        assert_eq!(
            face_value_minor(&balance_quote("10000", "0", "10000")),
            Ok(10_000)
        );
    }

    /// A bonus raises what the buyer receives above what the platform was paid. Accruing on
    /// the received amount would pay the agent out of money the platform never collected.
    #[test]
    fn face_value_excludes_a_bonus() {
        use crate::store_billing::sales::compute_amounts;

        // Sells 100 CNY of balance and grants 20 CNY: the platform is paid 100.
        let quote = balance_quote("10000", "2000", "12000");
        let base = face_value_minor(&quote).expect("face value");
        assert_eq!(base, 10_000, "the bonus must not enter the commission base");

        let amounts = compute_amounts(base, 500, 0).expect("amounts");
        assert_eq!(amounts.commission_minor, 500);
        assert_eq!(amounts.payment_minor - amounts.commission_minor, 9_500);

        // Had the bonus been included the agent would take 600 and leave the platform 9400.
        let inflated = compute_amounts(12_000, 500, 0).expect("amounts");
        assert_eq!(inflated.commission_minor, 600);
    }

    #[test]
    fn a_plan_quote_has_no_face_value() {
        let quote = serde_json::json!({
            "version": 2,
            "product": { "kind": "plan", "balance": null }
        })
        .to_string();
        assert_eq!(face_value_minor(&quote), Err(SalesStoreError::InvalidInput));
        assert_eq!(face_value_minor("not json"), Err(SalesStoreError::InvalidInput));
    }

    #[test]
    fn minor_amounts_must_be_canonical() {
        assert_eq!(parse_minor("10000"), Ok(10_000));
        assert_eq!(parse_minor("0"), Ok(0));
        assert_eq!(parse_minor("-1"), Err(SalesStoreError::InvalidInput));
        assert_eq!(parse_minor("010"), Err(SalesStoreError::InvalidInput));
        assert_eq!(parse_minor("1.5"), Err(SalesStoreError::InvalidInput));
        assert_eq!(parse_minor(""), Err(SalesStoreError::InvalidInput));
    }

    /// SC-3.4c: only the balance is signed. An individual amount stays nonnegative.
    #[test]
    fn only_the_balance_may_be_negative() {
        assert_eq!(parse_signed_minor("-500"), Ok(-500));
        assert_eq!(parse_signed_minor("0"), Ok(0));
        assert_eq!(parse_signed_minor("500"), Ok(500));
        assert_eq!(parse_signed_minor("-0"), Err(SalesStoreError::InvalidInput));
        assert_eq!(parse_signed_minor("--5"), Err(SalesStoreError::InvalidInput));
        assert_eq!(parse_signed_minor("+5"), Err(SalesStoreError::InvalidInput));
    }

    /// SC-1.7 and SC-2.7a: 1 CNY is both the smallest purchasable amount and the smallest
    /// face value a code may price, and its commission is exactly 0.05 CNY.
    #[test]
    fn the_minimum_recharge_earns_a_commission_in_whole_fen() {
        use crate::store_billing::sales::{MIN_CODED_ORDER_MINOR, SalesError, compute_amounts};

        assert_eq!(MIN_CODED_ORDER_MINOR, 100);
        let minimum = compute_amounts(MIN_CODED_ORDER_MINOR, 500, 0).expect("1 CNY");
        assert_eq!(minimum.commission_minor, 5);

        // A smaller face value is refused rather than accruing zero, so an agent never makes
        // a sale that credits nothing.
        assert_eq!(
            compute_amounts(99, 500, 0),
            Err(SalesError::AmountTooSmall)
        );
    }
}
