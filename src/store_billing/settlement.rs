use super::money::Currency;
use crate::db::DbPool;
use chrono::{NaiveDate, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementLineClass {
    Gross,
    Refund,
    Dispute,
    Fee,
    Tax,
    CurrencyConversion,
    Net,
}

impl SettlementLineClass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Gross => "gross",
            Self::Refund => "refund",
            Self::Dispute => "dispute",
            Self::Fee => "fee",
            Self::Tax => "tax",
            Self::CurrencyConversion => "currency_conversion",
            Self::Net => "net",
        }
    }

    const fn requires_order_match(self) -> bool {
        matches!(self, Self::Gross | Self::Refund | Self::Dispute)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementLineInput {
    pub provider_line_id: String,
    pub class: SettlementLineClass,
    pub amount_minor: String,
    pub currency: Currency,
    pub provider_transaction_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementReportInput {
    pub channel_id: String,
    pub credential_version_id: String,
    pub provider_report_id: String,
    pub report_date: String,
    pub body_digest: String,
    pub lines: Vec<SettlementLineInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementImportResult {
    pub report_id: String,
    pub replayed: bool,
    pub line_count: usize,
    pub unmatched_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SettlementError {
    #[error("invalid settlement input")]
    InvalidInput,
    #[error("settlement report conflicts with stored evidence")]
    Conflict,
    #[error("settlement storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone)]
pub struct SettlementStore {
    db: DbPool,
}

impl SettlementStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn import_report(
        &self,
        input: SettlementReportInput,
    ) -> Result<SettlementImportResult, SettlementError> {
        validate_report(&input)?;
        let tx = self.db.begin_write().await.map_err(storage)?;
        if let Some(existing) = tx
            .query_one(self.db.stmt(
                "SELECT id, channel_id, report_date, body_digest
                 FROM store_settlement_reports
                 WHERE credential_version_id = $1 AND provider_report_id = $2",
                vec![
                    input.credential_version_id.clone().into(),
                    input.provider_report_id.clone().into(),
                ],
            ))
            .await
            .map_err(storage)?
        {
            if row_string(&existing, "channel_id")? != input.channel_id
                || row_string(&existing, "report_date")? != input.report_date
                || row_string(&existing, "body_digest")? != input.body_digest
            {
                return Err(SettlementError::Conflict);
            }
            let result = report_counts(&self.db, &*tx, &row_string(&existing, "id")?, true).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        let credential_matches = tx
            .query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM store_channel_credentials
                 WHERE id = $1 AND channel_id = $2",
                vec![
                    input.credential_version_id.clone().into(),
                    input.channel_id.clone().into(),
                ],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| SettlementError::Storage("credential count is missing".to_string()))?
            .try_get::<i64>("", "value")
            .map_err(storage)?;
        if credential_matches != 1 {
            return Err(SettlementError::InvalidInput);
        }

        let report_id = Uuid::new_v4().to_string();
        let now = timestamp();
        tx.execute(self.db.stmt(
            "INSERT INTO store_settlement_reports
                (id, channel_id, credential_version_id, provider_report_id,
                 report_date, body_digest, imported_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            vec![
                report_id.clone().into(),
                input.channel_id.clone().into(),
                input.credential_version_id.clone().into(),
                input.provider_report_id.into(),
                input.report_date.into(),
                input.body_digest.into(),
                now.clone().into(),
            ],
        ))
        .await
        .map_err(conflict_or_storage)?;

        let line_count = input.lines.len();
        let mut unmatched_count = 0usize;
        for line in input.lines {
            let matched_order_id = match_order(
                &self.db,
                &*tx,
                &input.credential_version_id,
                line.class,
                line.provider_transaction_id.as_deref(),
            )
            .await?;
            if line.class.requires_order_match() && matched_order_id.is_none() {
                unmatched_count += 1;
                upsert_unmatched_case(
                    &self.db,
                    &*tx,
                    &input.channel_id,
                    &input.credential_version_id,
                    &line.provider_line_id,
                    line.class,
                    line.provider_transaction_id.as_deref(),
                    &now,
                )
                .await?;
            }
            tx.execute(self.db.stmt(
                "INSERT INTO store_settlement_lines
                    (id, report_id, credential_version_id, provider_line_id,
                     class, amount_minor, currency, provider_transaction_id,
                     matched_order_id, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                vec![
                    Uuid::new_v4().to_string().into(),
                    report_id.clone().into(),
                    input.credential_version_id.clone().into(),
                    line.provider_line_id.into(),
                    line.class.as_str().into(),
                    line.amount_minor.into(),
                    currency_string(line.currency).into(),
                    line.provider_transaction_id.into(),
                    matched_order_id.into(),
                    now.clone().into(),
                ],
            ))
            .await
            .map_err(conflict_or_storage)?;
        }
        let result = SettlementImportResult {
            report_id,
            replayed: false,
            line_count,
            unmatched_count,
        };
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
}

async fn match_order<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    credential_id: &str,
    class: SettlementLineClass,
    provider_transaction_id: Option<&str>,
) -> Result<Option<String>, SettlementError> {
    if !class.requires_order_match() {
        return Ok(None);
    }
    let Some(provider_id) = provider_transaction_id.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let (sql, values) = match class {
        SettlementLineClass::Gross => (
            "SELECT order_id FROM store_payment_attempts
             WHERE credential_version_id = $1 AND provider_transaction_id = $2
             ORDER BY order_id LIMIT 2",
            vec![credential_id.into(), provider_id.into()],
        ),
        SettlementLineClass::Refund => (
            "SELECT f.order_id
             FROM store_refunds f
             JOIN store_payment_attempts a ON a.id = f.attempt_id
             WHERE a.credential_version_id = $1 AND f.provider_refund_id = $2
             ORDER BY f.order_id LIMIT 2",
            vec![credential_id.into(), provider_id.into()],
        ),
        SettlementLineClass::Dispute => (
            "SELECT r.order_id
             FROM store_order_recovery_claims c
             JOIN store_order_reward_recoveries r ON r.id = c.recovery_id
             WHERE c.credential_version_id = $1 AND c.provider_claim_id = $2
               AND c.kind IN ('dispute', 'chargeback')
             ORDER BY r.order_id LIMIT 2",
            vec![credential_id.into(), provider_id.into()],
        ),
        _ => return Ok(None),
    };
    let rows = conn
        .query_all(db.stmt(sql, values))
        .await
        .map_err(storage)?;
    if rows.len() == 1 {
        Ok(Some(row_string(&rows[0], "order_id")?))
    } else {
        Ok(None)
    }
}

#[allow(clippy::too_many_arguments)]
async fn upsert_unmatched_case<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    channel_id: &str,
    credential_id: &str,
    provider_line_id: &str,
    class: SettlementLineClass,
    provider_transaction_id: Option<&str>,
    now: &str,
) -> Result<(), SettlementError> {
    let case_id = format!(
        "settlement:{}",
        lower_hex(&Sha256::digest(
            format!("{credential_id}\0{provider_line_id}").as_bytes()
        ))
    );
    let evidence = serde_json::json!({
        "credential_version_id": credential_id,
        "provider_line_id": provider_line_id,
        "class": class.as_str(),
        "provider_transaction_id": provider_transaction_id,
    });
    conn.execute(db.stmt(
        "INSERT INTO store_reconciliation_cases
            (id, order_id, channel_id, severity, kind, state,
             evidence_json, created_at, updated_at)
         VALUES ($1, NULL, $2, 'critical', 'unmatched_settlement', 'open', $3, $4, $4)
         ON CONFLICT (id) DO UPDATE SET
            evidence_json = excluded.evidence_json, updated_at = excluded.updated_at",
        vec![
            case_id.into(),
            channel_id.into(),
            evidence.to_string().into(),
            now.into(),
        ],
    ))
    .await
    .map_err(storage)?;
    Ok(())
}

async fn report_counts<C: ConnectionTrait>(
    db: &DbPool,
    conn: &C,
    report_id: &str,
    replayed: bool,
) -> Result<SettlementImportResult, SettlementError> {
    let row = conn
        .query_one(db.stmt(
            "SELECT COUNT(*) AS line_count,
                    SUM(CASE WHEN class IN ('gross', 'refund', 'dispute')
                                  AND matched_order_id IS NULL THEN 1 ELSE 0 END) AS unmatched_count
             FROM store_settlement_lines WHERE report_id = $1",
            vec![report_id.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or_else(|| SettlementError::Storage("settlement count is missing".to_string()))?;
    Ok(SettlementImportResult {
        report_id: report_id.to_string(),
        replayed,
        line_count: usize::try_from(row.try_get::<i64>("", "line_count").map_err(storage)?)
            .map_err(storage)?,
        unmatched_count: usize::try_from(
            row.try_get::<i64>("", "unmatched_count").map_err(storage)?,
        )
        .map_err(storage)?,
    })
}

fn validate_report(input: &SettlementReportInput) -> Result<(), SettlementError> {
    if input.channel_id.is_empty()
        || input.credential_version_id.is_empty()
        || input.provider_report_id.is_empty()
        || input.provider_report_id.len() > 200
        || NaiveDate::parse_from_str(&input.report_date, "%Y-%m-%d").is_err()
        || input.body_digest.len() != 64
        || !input
            .body_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || input.lines.is_empty()
        || input.lines.len() > 100_000
    {
        return Err(SettlementError::InvalidInput);
    }
    let mut ids = BTreeSet::new();
    for line in &input.lines {
        if line.provider_line_id.is_empty()
            || line.provider_line_id.len() > 200
            || !ids.insert(line.provider_line_id.as_str())
            || parse_signed_minor(&line.amount_minor).is_none()
            || line
                .provider_transaction_id
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 200)
        {
            return Err(SettlementError::InvalidInput);
        }
    }
    Ok(())
}

fn parse_signed_minor(value: &str) -> Option<i128> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty()
        || value == "-0"
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

fn currency_string(currency: Currency) -> &'static str {
    match currency {
        Currency::CNY => "CNY",
        Currency::USD => "USD",
    }
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, SettlementError> {
    row.try_get("", column).map_err(storage)
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn storage(error: impl ToString) -> SettlementError {
    SettlementError::Storage(error.to_string())
}

fn conflict_or_storage(error: impl ToString) -> SettlementError {
    let detail = error.to_string();
    if detail.to_ascii_lowercase().contains("unique") {
        SettlementError::Conflict
    } else {
        SettlementError::Storage(detail)
    }
}
