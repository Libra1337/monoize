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
    #[error("insufficient commission balance")]
    InsufficientBalance,
    #[error("sales storage failure: {0}")]
    Storage(String),
}

impl From<SalesError> for SalesStoreError {
    fn from(error: SalesError) -> Self {
        match error {
            SalesError::DiscountAboveRate => Self::DiscountAboveRate,
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

fn parse_minor(value: &str) -> Result<i128, SalesStoreError> {
    let parsed: i128 = value.parse().map_err(|_| SalesStoreError::InvalidInput)?;
    if parsed < 0 || parsed.to_string() != value {
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
    pub commission_balance_minor: String,
    pub enabled: bool,
    pub created_at: String,
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
                        a.commission_balance_fen, a.enabled, a.created_at
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
        })
    }

    pub async fn list_agents(&self) -> Result<Vec<SalesAgent>, SalesStoreError> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT a.user_id, u.username, a.code, a.discount_bp,
                        a.commission_balance_fen, a.enabled, a.created_at
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
    pub async fn update_agent(
        &self,
        user_id: &str,
        discount_bp: i64,
        enabled: bool,
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
                "UPDATE sales_agents SET discount_bp = $2, enabled = $3, updated_at = $4
                 WHERE user_id = $1",
                vec![
                    user_id.into(),
                    discount_bp.into(),
                    i64::from(enabled).into(),
                    now.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        self.agent_for_user(user_id)
            .await?
            .ok_or(SalesStoreError::NotAgent)
    }

    /// SC-6.2 aggregates over non-reversed entries.
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
        if row_string(&order, "user_id")? != buyer_user_id
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
        let balance = parse_minor(&row_string(&agent, "commission_balance_fen")?)?;
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
            let balance = parse_minor(&row_string(&agent, "commission_balance_fen")?)?;
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
    let balance = parse_minor(&row_string(&row, "commission_balance_fen")?)?;
    set_agent_balance(db, conn, agent_user_id, balance + delta_minor, now).await
}

async fn set_agent_balance<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    agent_user_id: &str,
    balance_minor: i128,
    now: DateTime<Utc>,
) -> Result<(), SalesStoreError> {
    // SC-3.4 clamps at zero: a reversal after the agent already withdrew must not drive the
    // balance negative, and the reversed entry keeps the record of the shortfall.
    let clamped = balance_minor.max(0);
    conn.execute(db.stmt(
        "UPDATE sales_agents SET commission_balance_fen = $2, updated_at = $3 WHERE user_id = $1",
        vec![
            agent_user_id.into(),
            clamped.to_string().into(),
            timestamp(now).into(),
        ],
    ))
    .await
    .map_err(storage)?;
    Ok(())
}

/// Reads the order face value in Coin minor units from a frozen quote (SC-3.3).
///
/// The face value is the balance quote's `actual_received_minor`, not `payment_minor`, so a
/// discounted order still accrues on what the buyer received.
pub fn face_value_minor(quote_json: &str) -> Result<i128, SalesStoreError> {
    let quote: serde_json::Value =
        serde_json::from_str(quote_json).map_err(|_| SalesStoreError::InvalidInput)?;
    let received = quote
        .get("product")
        .and_then(|product| product.get("balance"))
        .and_then(|balance| balance.get("actual_received_minor"))
        .and_then(serde_json::Value::as_str)
        .ok_or(SalesStoreError::InvalidInput)?;
    parse_minor(received)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_value_reads_the_received_amount_not_the_paid_amount() {
        let quote = serde_json::json!({
            "version": 2,
            "product": {
                "kind": "balance",
                "balance": {
                    "recharge_minor": "9900",
                    "bonus_minor": "0",
                    "actual_received_minor": "10000"
                }
            }
        })
        .to_string();
        assert_eq!(face_value_minor(&quote), Ok(10_000));
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
}
