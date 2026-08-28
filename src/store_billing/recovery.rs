use crate::db::DbPool;
use chrono::{SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryClaimKind {
    Refund,
    Dispute,
    Chargeback,
}

impl RecoveryClaimKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Refund => "refund",
            Self::Dispute => "dispute",
            Self::Chargeback => "chargeback",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginRefundInput {
    pub order_id: String,
    pub requested_by_admin_id: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecoveryClaimInput {
    pub order_id: String,
    pub credential_version_id: String,
    pub provider_claim_id: String,
    pub provider_event_row_id: Option<String>,
    pub kind: RecoveryClaimKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundRecord {
    pub id: String,
    pub order_id: String,
    pub attempt_id: String,
    pub provider_refund_id: Option<String>,
    pub idempotency_key: String,
    pub state: String,
    pub amount_minor: String,
    pub currency: String,
    pub recovery_id: String,
    pub original_nano_usd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryClaim {
    pub id: String,
    pub recovery_id: String,
    pub order_id: String,
    pub credential_version_id: String,
    pub provider_claim_id: String,
    pub provider_event_row_id: Option<String>,
    pub kind: RecoveryClaimKind,
    pub amount_nano_usd: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecoveryError {
    #[error("invalid recovery input")]
    InvalidInput,
    #[error("recovery object was not found")]
    NotFound,
    #[error("order cannot enter economic recovery")]
    OrderNotRecoverable,
    #[error("available balance cannot cover the refund reserve")]
    InsufficientBalance,
    #[error("economic recovery state conflicts with this operation")]
    Conflict,
    #[error("economic recovery storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone)]
pub struct RecoveryStore {
    db: DbPool,
}

impl RecoveryStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn begin_refund(
        &self,
        input: BeginRefundInput,
    ) -> Result<RefundRecord, RecoveryError> {
        validate_refund_input(&input)?;
        let tx = self.db.begin_write().await.map_err(storage)?;
        if let Some(existing) = load_refund_by_key(&self.db, &*tx, &input.idempotency_key).await? {
            if existing.order_id != input.order_id {
                return Err(RecoveryError::Conflict);
            }
            tx.commit().await.map_err(storage)?;
            return Ok(existing);
        }

        let lock = lock_clause(&self.db);
        let order = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT o.id, o.user_id, o.product_kind, o.payment_state,
                            o.fulfillment_state, o.payment_minor, o.payment_currency,
                            o.state_revision, a.id AS attempt_id,
                            a.credential_version_id
                     FROM store_orders o
                     JOIN store_payment_attempts a ON a.order_id = o.id
                     WHERE o.id = $1 AND a.state = 'paid'
                       AND a.provider_transaction_id IS NOT NULL
                     ORDER BY a.paid_at DESC, a.id DESC LIMIT 1{lock}"
                ),
                vec![input.order_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(RecoveryError::OrderNotRecoverable)?;
        if row_string(&order, "payment_state")? != "paid"
            || (row_string(&order, "product_kind")? == "plan"
                && row_string(&order, "fulfillment_state")? == "fulfilled")
        {
            return Err(RecoveryError::OrderNotRecoverable);
        }

        let user_id = row_string(&order, "user_id")?;
        let original = if row_string(&order, "product_kind")? == "balance"
            && row_string(&order, "fulfillment_state")? == "fulfilled"
        {
            fulfilled_reward(&self.db, &*tx, &input.order_id).await?
        } else {
            0
        };
        let now = timestamp();
        let recovery_id =
            ensure_recovery(&self.db, &*tx, &input.order_id, original, &now, lock).await?;
        let recovery = load_recovery(&self.db, &*tx, &recovery_id, lock).await?;
        let reserved = parse_nonnegative(&row_string(&recovery, "reserved_nano_usd")?)?;
        let recovered = parse_nonnegative(&row_string(&recovery, "recovered_nano_usd")?)?;
        if recovered != 0 {
            return Err(RecoveryError::Conflict);
        }
        if original != 0 && reserved == 0 {
            let balance = lock_user_balance(&self.db, &*tx, &user_id, lock).await?;
            let original_delta = checked_i128(original)?;
            if balance < original_delta {
                return Err(RecoveryError::InsufficientBalance);
            }
            let ledger_key = format!("store:recovery:{recovery_id}:reserve");
            apply_balance_delta(
                &self.db,
                &*tx,
                &user_id,
                balance,
                -original_delta,
                "store_recovery_reserve",
                &ledger_key,
                &input.order_id,
                &recovery_id,
                &now,
            )
            .await?;
            tx.execute(self.db.stmt(
                "UPDATE store_order_reward_recoveries
                 SET reserved_nano_usd = $2, debit_ledger_key = $3,
                     state = 'reserved', updated_at = $4
                 WHERE id = $1 AND reserved_nano_usd = '0' AND recovered_nano_usd = '0'",
                vec![
                    recovery_id.clone().into(),
                    original.to_string().into(),
                    ledger_key.into(),
                    now.clone().into(),
                ],
            ))
            .await
            .map_err(storage)?;
        } else if original != 0 && reserved != original {
            return Err(RecoveryError::InsufficientBalance);
        }

        let refund_id = Uuid::new_v4().to_string();
        let claim_id = Uuid::new_v4().to_string();
        tx.execute(self.db.stmt(
            "INSERT INTO store_refunds
                (id, order_id, attempt_id, provider_refund_id, idempotency_key,
                 state, amount_minor, currency, requested_by_admin_id, created_at, updated_at)
             VALUES ($1, $2, $3, NULL, $4, 'created', $5, $6, $7, $8, $8)",
            vec![
                refund_id.clone().into(),
                input.order_id.clone().into(),
                row_string(&order, "attempt_id")?.into(),
                input.idempotency_key.clone().into(),
                row_string(&order, "payment_minor")?.into(),
                row_string(&order, "payment_currency")?.into(),
                input.requested_by_admin_id.into(),
                now.clone().into(),
            ],
        ))
        .await
        .map_err(conflict_or_storage)?;
        tx.execute(self.db.stmt(
            "INSERT INTO store_order_recovery_claims
                (id, recovery_id, credential_version_id, provider_claim_id,
                 provider_event_row_id, kind, amount_nano_usd, state, created_at)
             VALUES ($1, $2, $3, $4, NULL, 'refund', $5, 'open', $6)",
            vec![
                claim_id.into(),
                recovery_id.clone().into(),
                row_string(&order, "credential_version_id")?.into(),
                input.idempotency_key.clone().into(),
                original.to_string().into(),
                now.clone().into(),
            ],
        ))
        .await
        .map_err(conflict_or_storage)?;
        let changed = tx
            .execute(self.db.stmt(
                "UPDATE store_orders
                 SET payment_state = 'refund_pending', refund_pending_at = $2,
                     updated_at = $2, state_revision = state_revision + 1
                 WHERE id = $1 AND payment_state = 'paid' AND state_revision = $3",
                vec![
                    input.order_id.clone().into(),
                    now.into(),
                    row_i64(&order, "state_revision")?.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if changed.rows_affected() != 1 {
            return Err(RecoveryError::Conflict);
        }
        let result =
            load_refund_by_id(&self.db, &*tx, &refund_id)
                .await?
                .ok_or(RecoveryError::Storage(
                    "inserted refund is missing".to_string(),
                ))?;
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }

    pub async fn mark_refund_pending(
        &self,
        refund_id: &str,
        provider_refund_id: &str,
    ) -> Result<RefundRecord, RecoveryError> {
        if refund_id.is_empty() || provider_refund_id.trim().is_empty() {
            return Err(RecoveryError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(storage)?;
        let existing = load_refund_by_id(&self.db, &*tx, refund_id)
            .await?
            .ok_or(RecoveryError::NotFound)?;
        if existing.state == "pending" {
            if existing.provider_refund_id.as_deref() != Some(provider_refund_id) {
                return Err(RecoveryError::Conflict);
            }
            tx.commit().await.map_err(storage)?;
            return Ok(existing);
        }
        if existing.state != "created" {
            return Err(RecoveryError::Conflict);
        }
        tx.execute(self.db.stmt(
            "UPDATE store_refunds
             SET state = 'pending', provider_refund_id = $2, updated_at = $3
             WHERE id = $1 AND state = 'created'",
            vec![
                refund_id.into(),
                provider_refund_id.into(),
                timestamp().into(),
            ],
        ))
        .await
        .map_err(storage)?;
        let result = load_refund_by_id(&self.db, &*tx, refund_id)
            .await?
            .ok_or(RecoveryError::NotFound)?;
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }

    pub async fn reject_refund(&self, refund_id: &str) -> Result<(), RecoveryError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = lock_clause(&self.db);
        let row = load_refund_mutation(&self.db, &*tx, refund_id, lock).await?;
        let refund_state = row_string(&row, "refund_state")?;
        if refund_state == "failed" {
            tx.commit().await.map_err(storage)?;
            return Ok(());
        }
        if refund_state == "succeeded" {
            return Err(RecoveryError::Conflict);
        }
        let recovery_id = row_string(&row, "recovery_id")?;
        let claim_id = row_string(&row, "claim_id")?;
        let now = timestamp();
        tx.execute(self.db.stmt(
            "UPDATE store_order_recovery_claims
             SET state = 'resolved', resolved_at = $2
             WHERE id = $1 AND state = 'open'",
            vec![claim_id.clone().into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        let other_open = count_value(
            tx.query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM store_order_recovery_claims
                 WHERE recovery_id = $1 AND id <> $2 AND state = 'open'",
                vec![recovery_id.clone().into(), claim_id.into()],
            ))
            .await
            .map_err(storage)?,
        )?;
        let reserved = parse_nonnegative(&row_string(&row, "reserved_nano_usd")?)?;
        let recovered = parse_nonnegative(&row_string(&row, "recovered_nano_usd")?)?;
        if other_open == 0 && recovered == 0 && reserved != 0 {
            let user_id = row_string(&row, "user_id")?;
            let balance = lock_user_balance(&self.db, &*tx, &user_id, lock).await?;
            let reserved_delta = checked_i128(reserved)?;
            let release_key = format!("store:recovery:{recovery_id}:release");
            apply_balance_delta(
                &self.db,
                &*tx,
                &user_id,
                balance,
                reserved_delta,
                "store_recovery_release",
                &release_key,
                &row_string(&row, "order_id")?,
                &recovery_id,
                &now,
            )
            .await?;
            tx.execute(self.db.stmt(
                "UPDATE store_order_reward_recoveries
                 SET reserved_nano_usd = '0', release_ledger_key = $2,
                     state = 'released', updated_at = $3
                 WHERE id = $1 AND recovered_nano_usd = '0'",
                vec![recovery_id.into(), release_key.into(), now.clone().into()],
            ))
            .await
            .map_err(storage)?;
        }
        tx.execute(self.db.stmt(
            "UPDATE store_refunds
             SET state = 'failed', resolved_at = $2, updated_at = $2
             WHERE id = $1 AND state IN ('created', 'pending')",
            vec![refund_id.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_orders
             SET payment_state = 'paid', updated_at = $2,
                 state_revision = state_revision + 1
             WHERE id = $1 AND payment_state = 'refund_pending'",
            vec![row_string(&row, "order_id")?.into(), now.into()],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    pub async fn complete_refund(&self, refund_id: &str) -> Result<(), RecoveryError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let row = load_refund_mutation(&self.db, &*tx, refund_id, lock_clause(&self.db)).await?;
        let state = row_string(&row, "refund_state")?;
        if state == "succeeded" {
            tx.commit().await.map_err(storage)?;
            return Ok(());
        }
        if state == "failed" {
            return Err(RecoveryError::Conflict);
        }
        let original = parse_nonnegative(&row_string(&row, "original_nano_usd")?)?;
        let reserved = parse_nonnegative(&row_string(&row, "reserved_nano_usd")?)?;
        if original != 0 && reserved != original {
            return Err(RecoveryError::Conflict);
        }
        let now = timestamp();
        tx.execute(self.db.stmt(
            "UPDATE store_order_reward_recoveries
             SET reserved_nano_usd = '0', recovered_nano_usd = original_nano_usd,
                 state = 'recovered', updated_at = $2
             WHERE id = $1 AND recovered_nano_usd = '0'",
            vec![row_string(&row, "recovery_id")?.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_order_recovery_claims
             SET state = 'consumed', resolved_at = $2
             WHERE id = $1 AND state = 'open'",
            vec![row_string(&row, "claim_id")?.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_refunds
             SET state = 'succeeded', resolved_at = $2, updated_at = $2
             WHERE id = $1 AND state IN ('created', 'pending')",
            vec![refund_id.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        let changed = tx
            .execute(self.db.stmt(
                "UPDATE store_orders
                 SET payment_state = 'refunded', refunded_at = $2, updated_at = $2,
                     state_revision = state_revision + 1
                 WHERE id = $1 AND payment_state = 'refund_pending'",
                vec![row_string(&row, "order_id")?.into(), now.into()],
            ))
            .await
            .map_err(storage)?;
        if changed.rows_affected() != 1 {
            return Err(RecoveryError::Conflict);
        }
        tx.commit().await.map_err(storage)
    }

    pub async fn open_claim(
        &self,
        input: VerifiedRecoveryClaimInput,
    ) -> Result<RecoveryClaim, RecoveryError> {
        validate_claim_input(&input)?;
        if input.kind == RecoveryClaimKind::Refund {
            return Err(RecoveryError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(storage)?;
        if let Some(existing) = load_claim_by_provider(
            &self.db,
            &*tx,
            &input.credential_version_id,
            &input.provider_claim_id,
            input.kind,
        )
        .await?
        {
            if existing.order_id != input.order_id {
                return Err(RecoveryError::Conflict);
            }
            tx.commit().await.map_err(storage)?;
            return Ok(existing);
        }
        let lock = lock_clause(&self.db);
        let order = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT o.id, o.user_id, o.product_kind, o.payment_state,
                            o.fulfillment_state, o.state_revision
                     FROM store_orders o
                     WHERE o.id = $1 AND EXISTS (
                         SELECT 1 FROM store_payment_attempts a
                         WHERE a.order_id = o.id AND a.credential_version_id = $2
                           AND a.state = 'paid'
                     ){lock}"
                ),
                vec![
                    input.order_id.clone().into(),
                    input.credential_version_id.clone().into(),
                ],
            ))
            .await
            .map_err(storage)?
            .ok_or(RecoveryError::OrderNotRecoverable)?;
        if !matches!(
            row_string(&order, "payment_state")?.as_str(),
            "paid" | "refund_pending" | "refunded"
        ) {
            return Err(RecoveryError::OrderNotRecoverable);
        }
        let original = if row_string(&order, "product_kind")? == "balance"
            && row_string(&order, "fulfillment_state")? == "fulfilled"
        {
            fulfilled_reward(&self.db, &*tx, &input.order_id).await?
        } else {
            0
        };
        let now = timestamp();
        let recovery_id =
            ensure_recovery(&self.db, &*tx, &input.order_id, original, &now, lock).await?;
        let recovery = load_recovery(&self.db, &*tx, &recovery_id, lock).await?;
        let reserved = parse_nonnegative(&row_string(&recovery, "reserved_nano_usd")?)?;
        let recovered = parse_nonnegative(&row_string(&recovery, "recovered_nano_usd")?)?;
        if original != 0 && reserved == 0 && recovered == 0 {
            let user_id = row_string(&order, "user_id")?;
            let balance = lock_user_balance(&self.db, &*tx, &user_id, lock).await?;
            let available = balance.max(0) as u128;
            let reserve = original.min(available);
            if reserve != 0 {
                let ledger_key = format!("store:recovery:{recovery_id}:reserve");
                apply_balance_delta(
                    &self.db,
                    &*tx,
                    &user_id,
                    balance,
                    -(reserve as i128),
                    "store_recovery_reserve",
                    &ledger_key,
                    &input.order_id,
                    &recovery_id,
                    &now,
                )
                .await?;
                tx.execute(self.db.stmt(
                    "UPDATE store_order_reward_recoveries
                     SET reserved_nano_usd = $2, debit_ledger_key = $3,
                         state = 'reserved', updated_at = $4
                     WHERE id = $1 AND reserved_nano_usd = '0' AND recovered_nano_usd = '0'",
                    vec![
                        recovery_id.clone().into(),
                        reserve.to_string().into(),
                        ledger_key.into(),
                        now.clone().into(),
                    ],
                ))
                .await
                .map_err(storage)?;
            }
        }
        let claim_id = Uuid::new_v4().to_string();
        tx.execute(self.db.stmt(
            "INSERT INTO store_order_recovery_claims
                (id, recovery_id, credential_version_id, provider_claim_id,
                 provider_event_row_id, kind, amount_nano_usd, state, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'open', $8)",
            vec![
                claim_id.clone().into(),
                recovery_id.into(),
                input.credential_version_id.clone().into(),
                input.provider_claim_id.clone().into(),
                input.provider_event_row_id.clone().into(),
                input.kind.as_str().into(),
                original.to_string().into(),
                now.clone().into(),
            ],
        ))
        .await
        .map_err(conflict_or_storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_orders
             SET dispute_state = 'open', payment_hold = 1, updated_at = $2,
                 state_revision = state_revision + 1
             WHERE id = $1",
            vec![input.order_id.clone().into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "INSERT INTO store_balance_holds
                (user_id, active, reason, opened_at, cleared_at)
             VALUES ($1, 1, 'payment_dispute', $2, NULL)
             ON CONFLICT (user_id) DO UPDATE SET
                active = 1, reason = excluded.reason,
                opened_at = excluded.opened_at, cleared_at = NULL",
            vec![row_string(&order, "user_id")?.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_plan_entitlement_lifecycle
             SET suspended_at = $2, suspension_reason = 'payment_dispute', updated_at = $2
             WHERE entitlement_id IN (
                 SELECT id FROM store_plan_entitlement_generations
                 WHERE source_kind = 'order' AND source_id = $1
             ) AND suspended_at IS NULL AND revoked_at IS NULL",
            vec![input.order_id.into(), now.into()],
        ))
        .await
        .map_err(storage)?;
        let claim =
            load_claim_by_id(&self.db, &*tx, &claim_id)
                .await?
                .ok_or(RecoveryError::Storage(
                    "inserted recovery claim is missing".to_string(),
                ))?;
        tx.commit().await.map_err(storage)?;
        Ok(claim)
    }

    pub async fn win_claim(&self, claim_id: &str) -> Result<(), RecoveryError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = lock_clause(&self.db);
        let row = load_claim_mutation(&self.db, &*tx, claim_id, lock).await?;
        let state = row_string(&row, "claim_state")?;
        if state == "resolved" {
            tx.commit().await.map_err(storage)?;
            return Ok(());
        }
        if state == "consumed" {
            return Err(RecoveryError::Conflict);
        }
        let now = timestamp();
        tx.execute(self.db.stmt(
            "UPDATE store_order_recovery_claims
             SET state = 'resolved', resolved_at = $2
             WHERE id = $1 AND state = 'open'",
            vec![claim_id.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        let recovery_id = row_string(&row, "recovery_id")?;
        let other_open = count_value(
            tx.query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM store_order_recovery_claims
                 WHERE recovery_id = $1 AND id <> $2 AND state = 'open'",
                vec![recovery_id.clone().into(), claim_id.into()],
            ))
            .await
            .map_err(storage)?,
        )?;
        if other_open == 0 {
            let reserved = parse_nonnegative(&row_string(&row, "reserved_nano_usd")?)?;
            let recovered = parse_nonnegative(&row_string(&row, "recovered_nano_usd")?)?;
            let user_id = row_string(&row, "user_id")?;
            let mut final_balance = lock_user_balance(&self.db, &*tx, &user_id, lock).await?;
            if reserved != 0 && recovered == 0 {
                let release_key = format!("store:recovery:{recovery_id}:release");
                apply_balance_delta(
                    &self.db,
                    &*tx,
                    &user_id,
                    final_balance,
                    reserved as i128,
                    "store_recovery_release",
                    &release_key,
                    &row_string(&row, "order_id")?,
                    &recovery_id,
                    &now,
                )
                .await?;
                final_balance += reserved as i128;
                tx.execute(self.db.stmt(
                    "UPDATE store_order_reward_recoveries
                     SET reserved_nano_usd = '0', release_ledger_key = $2,
                         state = 'released', updated_at = $3
                     WHERE id = $1 AND recovered_nano_usd = '0'",
                    vec![recovery_id.into(), release_key.into(), now.clone().into()],
                ))
                .await
                .map_err(storage)?;
            }
            if final_balance >= 0 && recovered == 0 {
                tx.execute(self.db.stmt(
                    "UPDATE store_orders
                     SET dispute_state = 'won', payment_hold = 0, updated_at = $2,
                         state_revision = state_revision + 1
                     WHERE id = $1",
                    vec![row_string(&row, "order_id")?.into(), now.clone().into()],
                ))
                .await
                .map_err(storage)?;
                tx.execute(self.db.stmt(
                    "UPDATE store_balance_holds
                     SET active = 0, cleared_at = $2 WHERE user_id = $1",
                    vec![user_id.into(), now.clone().into()],
                ))
                .await
                .map_err(storage)?;
                tx.execute(self.db.stmt(
                    "UPDATE store_plan_entitlement_lifecycle
                     SET suspended_at = NULL, suspension_reason = NULL, updated_at = $2
                     WHERE entitlement_id IN (
                         SELECT id FROM store_plan_entitlement_generations
                         WHERE source_kind = 'order' AND source_id = $1 AND ends_at > $2
                     ) AND suspension_reason = 'payment_dispute' AND revoked_at IS NULL",
                    vec![row_string(&row, "order_id")?.into(), now.into()],
                ))
                .await
                .map_err(storage)?;
            }
        }
        tx.commit().await.map_err(storage)
    }

    pub async fn lose_claim(&self, claim_id: &str) -> Result<(), RecoveryError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = lock_clause(&self.db);
        let row = load_claim_mutation(&self.db, &*tx, claim_id, lock).await?;
        let state = row_string(&row, "claim_state")?;
        if state == "consumed" {
            tx.commit().await.map_err(storage)?;
            return Ok(());
        }
        if state == "resolved" {
            return Err(RecoveryError::Conflict);
        }
        let original = parse_nonnegative(&row_string(&row, "original_nano_usd")?)?;
        let reserved = parse_nonnegative(&row_string(&row, "reserved_nano_usd")?)?;
        let recovered = parse_nonnegative(&row_string(&row, "recovered_nano_usd")?)?;
        let remaining = original
            .checked_sub(reserved)
            .and_then(|value| value.checked_sub(recovered))
            .ok_or(RecoveryError::Conflict)?;
        let now = timestamp();
        if remaining != 0 {
            let user_id = row_string(&row, "user_id")?;
            let balance = lock_user_balance(&self.db, &*tx, &user_id, lock).await?;
            let loss_key = format!("store:recovery:{}:loss", row_string(&row, "recovery_id")?);
            apply_balance_delta(
                &self.db,
                &*tx,
                &user_id,
                balance,
                -(remaining as i128),
                "store_recovery_loss",
                &loss_key,
                &row_string(&row, "order_id")?,
                &row_string(&row, "recovery_id")?,
                &now,
            )
            .await?;
        }
        tx.execute(self.db.stmt(
            "UPDATE store_order_reward_recoveries
             SET reserved_nano_usd = '0', recovered_nano_usd = original_nano_usd,
                 state = 'recovered', updated_at = $2
             WHERE id = $1",
            vec![row_string(&row, "recovery_id")?.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_order_recovery_claims
             SET state = 'consumed', resolved_at = $2
             WHERE id = $1 AND state = 'open'",
            vec![claim_id.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_orders
             SET dispute_state = 'lost', payment_hold = 1, updated_at = $2,
                 state_revision = state_revision + 1
             WHERE id = $1",
            vec![row_string(&row, "order_id")?.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "INSERT INTO store_balance_holds
                (user_id, active, reason, opened_at, cleared_at)
             VALUES ($1, 1, 'payment_loss', $2, NULL)
             ON CONFLICT (user_id) DO UPDATE SET
                active = 1, reason = excluded.reason,
                opened_at = excluded.opened_at, cleared_at = NULL",
            vec![row_string(&row, "user_id")?.into(), now.clone().into()],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "UPDATE store_plan_entitlement_lifecycle
             SET revoked_at = $2, revocation_reason = 'payment_loss', updated_at = $2
             WHERE entitlement_id IN (
                 SELECT id FROM store_plan_entitlement_generations
                 WHERE source_kind = 'order' AND source_id = $1
             ) AND revoked_at IS NULL",
            vec![row_string(&row, "order_id")?.into(), now.into()],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }
}

async fn ensure_recovery<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    order_id: &str,
    original: u128,
    now: &str,
    lock: &str,
) -> Result<String, RecoveryError> {
    if let Some(row) = conn
        .query_one(db.stmt(
            &format!(
                "SELECT id, original_nano_usd FROM store_order_reward_recoveries
                 WHERE order_id = $1{lock}"
            ),
            vec![order_id.into()],
        ))
        .await
        .map_err(storage)?
    {
        if parse_nonnegative(&row_string(&row, "original_nano_usd")?)? != original {
            return Err(RecoveryError::Conflict);
        }
        return row_string(&row, "id");
    }
    let id = Uuid::new_v4().to_string();
    conn.execute(db.stmt(
        "INSERT INTO store_order_reward_recoveries
            (id, order_id, original_nano_usd, reserved_nano_usd,
             recovered_nano_usd, state, created_at, updated_at)
         VALUES ($1, $2, $3, '0', '0', 'open', $4, $4)",
        vec![
            id.clone().into(),
            order_id.into(),
            original.to_string().into(),
            now.into(),
        ],
    ))
    .await
    .map_err(conflict_or_storage)?;
    Ok(id)
}

async fn fulfilled_reward<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    order_id: &str,
) -> Result<u128, RecoveryError> {
    let row = conn
        .query_one(db.stmt(
            "SELECT delta_nano_usd FROM billing_ledger WHERE idempotency_key = $1",
            vec![format!("store:fulfillment:{order_id}").into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(RecoveryError::OrderNotRecoverable)?;
    let value = parse_signed(&row_string(&row, "delta_nano_usd")?)?;
    u128::try_from(value).map_err(|_| RecoveryError::OrderNotRecoverable)
}

async fn lock_user_balance<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    user_id: &str,
    lock: &str,
) -> Result<i128, RecoveryError> {
    let row = conn
        .query_one(db.stmt(
            &format!("SELECT balance_nano_usd FROM users WHERE id = $1{lock}"),
            vec![user_id.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(RecoveryError::NotFound)?;
    parse_signed(&row_string(&row, "balance_nano_usd")?)
}

#[allow(clippy::too_many_arguments)]
async fn apply_balance_delta<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    user_id: &str,
    current: i128,
    delta: i128,
    kind: &str,
    ledger_key: &str,
    order_id: &str,
    recovery_id: &str,
    now: &str,
) -> Result<(), RecoveryError> {
    let balance = current.checked_add(delta).ok_or(RecoveryError::Conflict)?;
    conn.execute(db.stmt(
        "UPDATE users SET balance_nano_usd = $2, updated_at = $3 WHERE id = $1",
        vec![user_id.into(), balance.to_string().into(), now.into()],
    ))
    .await
    .map_err(storage)?;
    conn.execute(db.stmt(
        "INSERT INTO billing_ledger
            (id, user_id, kind, delta_nano_usd, balance_after_nano_usd,
             meta_json, created_at, idempotency_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        vec![
            Uuid::new_v4().to_string().into(),
            user_id.into(),
            kind.into(),
            delta.to_string().into(),
            balance.to_string().into(),
            serde_json::json!({"order_id": order_id, "recovery_id": recovery_id})
                .to_string()
                .into(),
            now.into(),
            ledger_key.into(),
        ],
    ))
    .await
    .map_err(conflict_or_storage)?;
    Ok(())
}

async fn load_recovery<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    id: &str,
    lock: &str,
) -> Result<QueryResult, RecoveryError> {
    conn.query_one(db.stmt(
        &format!(
            "SELECT id, original_nano_usd, reserved_nano_usd, recovered_nano_usd,
                    debit_ledger_key, release_ledger_key, state
             FROM store_order_reward_recoveries WHERE id = $1{lock}"
        ),
        vec![id.into()],
    ))
    .await
    .map_err(storage)?
    .ok_or(RecoveryError::NotFound)
}

async fn load_refund_by_key<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    key: &str,
) -> Result<Option<RefundRecord>, RecoveryError> {
    load_refund(db, conn, "f.idempotency_key = $1", key).await
}

async fn load_refund_by_id<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    id: &str,
) -> Result<Option<RefundRecord>, RecoveryError> {
    load_refund(db, conn, "f.id = $1", id).await
}

async fn load_refund<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    predicate: &str,
    value: &str,
) -> Result<Option<RefundRecord>, RecoveryError> {
    conn.query_one(db.stmt(
        &format!(
            "SELECT f.id, f.order_id, f.attempt_id, f.provider_refund_id,
                    f.idempotency_key, f.state, f.amount_minor, f.currency,
                    c.recovery_id, r.original_nano_usd
             FROM store_refunds f
             JOIN store_order_recovery_claims c
               ON c.provider_claim_id = f.idempotency_key AND c.kind = 'refund'
             JOIN store_order_reward_recoveries r ON r.id = c.recovery_id
             WHERE {predicate}"
        ),
        vec![value.into()],
    ))
    .await
    .map_err(storage)?
    .map(refund_from_row)
    .transpose()
}

async fn load_refund_mutation<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    refund_id: &str,
    lock: &str,
) -> Result<QueryResult, RecoveryError> {
    conn.query_one(db.stmt(
        &format!(
            "SELECT f.id, f.order_id, f.state AS refund_state,
                    c.id AS claim_id, c.recovery_id,
                    r.original_nano_usd, r.reserved_nano_usd, r.recovered_nano_usd,
                    o.user_id, o.payment_state
             FROM store_refunds f
             JOIN store_order_recovery_claims c
               ON c.provider_claim_id = f.idempotency_key AND c.kind = 'refund'
             JOIN store_order_reward_recoveries r ON r.id = c.recovery_id
             JOIN store_orders o ON o.id = f.order_id
             WHERE f.id = $1{lock}"
        ),
        vec![refund_id.into()],
    ))
    .await
    .map_err(storage)?
    .ok_or(RecoveryError::NotFound)
}

fn refund_from_row(row: QueryResult) -> Result<RefundRecord, RecoveryError> {
    Ok(RefundRecord {
        id: row_string(&row, "id")?,
        order_id: row_string(&row, "order_id")?,
        attempt_id: row_string(&row, "attempt_id")?,
        provider_refund_id: row_optional_string(&row, "provider_refund_id")?,
        idempotency_key: row_string(&row, "idempotency_key")?,
        state: row_string(&row, "state")?,
        amount_minor: row_string(&row, "amount_minor")?,
        currency: row_string(&row, "currency")?,
        recovery_id: row_string(&row, "recovery_id")?,
        original_nano_usd: row_string(&row, "original_nano_usd")?,
    })
}

async fn load_claim_by_provider<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    credential_id: &str,
    provider_claim_id: &str,
    kind: RecoveryClaimKind,
) -> Result<Option<RecoveryClaim>, RecoveryError> {
    conn.query_one(db.stmt(
        "SELECT c.id, c.recovery_id, r.order_id, c.credential_version_id,
                c.provider_claim_id, c.provider_event_row_id, c.kind,
                c.amount_nano_usd, c.state
         FROM store_order_recovery_claims c
         JOIN store_order_reward_recoveries r ON r.id = c.recovery_id
         WHERE c.credential_version_id = $1 AND c.provider_claim_id = $2 AND c.kind = $3",
        vec![
            credential_id.into(),
            provider_claim_id.into(),
            kind.as_str().into(),
        ],
    ))
    .await
    .map_err(storage)?
    .map(claim_from_row)
    .transpose()
}

async fn load_claim_by_id<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    id: &str,
) -> Result<Option<RecoveryClaim>, RecoveryError> {
    conn.query_one(db.stmt(
        "SELECT c.id, c.recovery_id, r.order_id, c.credential_version_id,
                c.provider_claim_id, c.provider_event_row_id, c.kind,
                c.amount_nano_usd, c.state
         FROM store_order_recovery_claims c
         JOIN store_order_reward_recoveries r ON r.id = c.recovery_id
         WHERE c.id = $1",
        vec![id.into()],
    ))
    .await
    .map_err(storage)?
    .map(claim_from_row)
    .transpose()
}

fn claim_from_row(row: QueryResult) -> Result<RecoveryClaim, RecoveryError> {
    Ok(RecoveryClaim {
        id: row_string(&row, "id")?,
        recovery_id: row_string(&row, "recovery_id")?,
        order_id: row_string(&row, "order_id")?,
        credential_version_id: row_string(&row, "credential_version_id")?,
        provider_claim_id: row_string(&row, "provider_claim_id")?,
        provider_event_row_id: row_optional_string(&row, "provider_event_row_id")?,
        kind: match row_string(&row, "kind")?.as_str() {
            "refund" => RecoveryClaimKind::Refund,
            "dispute" => RecoveryClaimKind::Dispute,
            "chargeback" => RecoveryClaimKind::Chargeback,
            _ => {
                return Err(RecoveryError::Storage(
                    "invalid recovery claim kind".to_string(),
                ));
            }
        },
        amount_nano_usd: row_string(&row, "amount_nano_usd")?,
        state: row_string(&row, "state")?,
    })
}

async fn load_claim_mutation<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    claim_id: &str,
    lock: &str,
) -> Result<QueryResult, RecoveryError> {
    conn.query_one(db.stmt(
        &format!(
            "SELECT c.id, c.state AS claim_state, c.kind, c.recovery_id,
                    r.order_id, r.original_nano_usd, r.reserved_nano_usd,
                    r.recovered_nano_usd, o.user_id
             FROM store_order_recovery_claims c
             JOIN store_order_reward_recoveries r ON r.id = c.recovery_id
             JOIN store_orders o ON o.id = r.order_id
             WHERE c.id = $1{lock}"
        ),
        vec![claim_id.into()],
    ))
    .await
    .map_err(storage)?
    .ok_or(RecoveryError::NotFound)
}

fn validate_refund_input(input: &BeginRefundInput) -> Result<(), RecoveryError> {
    if input.order_id.is_empty()
        || input.requested_by_admin_id.trim().is_empty()
        || !valid_key(&input.idempotency_key)
    {
        return Err(RecoveryError::InvalidInput);
    }
    Ok(())
}

fn validate_claim_input(input: &VerifiedRecoveryClaimInput) -> Result<(), RecoveryError> {
    if input.order_id.is_empty()
        || input.credential_version_id.is_empty()
        || input.provider_claim_id.is_empty()
        || input.provider_claim_id.len() > 200
        || input
            .provider_event_row_id
            .as_ref()
            .is_some_and(|value| value.is_empty())
    {
        return Err(RecoveryError::InvalidInput);
    }
    Ok(())
}

fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn parse_nonnegative(value: &str) -> Result<u128, RecoveryError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(RecoveryError::Storage(
            "stored recovery amount is invalid".to_string(),
        ));
    }
    value.parse().map_err(storage)
}

fn parse_signed(value: &str) -> Result<i128, RecoveryError> {
    if value.is_empty()
        || value == "-0"
        || (value.starts_with('-') && value.len() == 1)
        || !value
            .strip_prefix('-')
            .unwrap_or(value)
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || (value.strip_prefix('-').unwrap_or(value).len() > 1
            && value.strip_prefix('-').unwrap_or(value).starts_with('0'))
    {
        return Err(RecoveryError::Storage(
            "stored balance is invalid".to_string(),
        ));
    }
    value.parse().map_err(storage)
}

fn checked_i128(value: u128) -> Result<i128, RecoveryError> {
    i128::try_from(value).map_err(|_| RecoveryError::Conflict)
}

fn count_value(row: Option<QueryResult>) -> Result<i64, RecoveryError> {
    row.ok_or_else(|| RecoveryError::Storage("count query returned no row".to_string()))?
        .try_get("", "value")
        .map_err(storage)
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, RecoveryError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_string(row: &QueryResult, column: &str) -> Result<Option<String>, RecoveryError> {
    row.try_get("", column).map_err(storage)
}

fn row_i64(row: &QueryResult, column: &str) -> Result<i64, RecoveryError> {
    row.try_get("", column).map_err(storage)
}

fn lock_clause(db: &DbPool) -> &'static str {
    if db.is_postgres() { " FOR UPDATE" } else { "" }
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn storage(error: impl ToString) -> RecoveryError {
    RecoveryError::Storage(error.to_string())
}

fn conflict_or_storage(error: impl ToString) -> RecoveryError {
    let detail = error.to_string();
    if detail.to_ascii_lowercase().contains("unique") {
        RecoveryError::Conflict
    } else {
        RecoveryError::Storage(detail)
    }
}
