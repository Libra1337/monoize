use crate::db::DbPool;
use crate::settings::PricingProfilePattern;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, TransactionTrait};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

const BILLING_RATE_CATALOG: &str = include_str!("billing-rates.catalog.json");

pub const RATE_CURRENCY_USD: &str = "USD";
pub const RATE_CURRENCY_CNY: &str = "CNY";

/// PP-CUR-2: a rate created through the dashboard is denominated in CNY unless the caller
/// states otherwise. Upstream catalogue ingest sets `USD` explicitly, so the default only
/// applies to prices an Admin types.
pub const DEFAULT_RATE_CURRENCY: &str = RATE_CURRENCY_CNY;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbBillingRateRecord {
    pub id: String,
    pub source: String,
    pub pricing_profile: String,
    pub model_pattern: Option<String>,
    pub provider_type: Option<String>,
    pub rate_kind: String,
    pub usage_class: String,
    pub unit: String,
    pub unit_price_nano: String,
    /// Currency the price is denominated in: `USD` or `CNY` (PP-CUR-1).
    pub unit_price_currency: String,
    pub context_tier: Option<String>,
    pub service_tier: Option<String>,
    pub modality: Option<String>,
    pub cache_ttl: Option<String>,
    pub match_json: Value,
    pub priority: i32,
    pub enabled: bool,
    pub raw_json: Value,
    pub updated_at: DateTime<Utc>,
}

impl DbBillingRateRecord {
    pub fn unit_price_nano(&self) -> Result<i128, String> {
        self.unit_price_nano
            .parse::<i128>()
            .map_err(|_| format!("invalid unit_price_nano for {}", self.id))
    }

    /// True when the price is quoted in the account's settlement currency rather than USD.
    pub fn is_cny_basis(&self) -> bool {
        self.unit_price_currency == RATE_CURRENCY_CNY
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpsertBillingRateInput {
    pub source: Option<String>,
    pub pricing_profile: Option<String>,
    pub model_pattern: Option<Option<String>>,
    pub provider_type: Option<Option<String>>,
    pub rate_kind: Option<String>,
    pub usage_class: Option<String>,
    pub unit: Option<String>,
    pub unit_price_nano: Option<String>,
    pub unit_price_currency: Option<String>,
    pub context_tier: Option<Option<String>>,
    pub service_tier: Option<Option<String>>,
    pub modality: Option<Option<String>>,
    pub cache_ttl: Option<Option<String>>,
    pub match_json: Option<Value>,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
    pub raw_json: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingRateSyncResult {
    pub success: bool,
    pub upserted: usize,
    pub skipped: usize,
    pub deleted: u64,
    pub fetched_at: String,
}

#[derive(Debug, Deserialize)]
struct CatalogRoot {
    rates: Vec<CatalogBillingRate>,
}

#[derive(Debug, Deserialize)]
struct CatalogBillingRate {
    id: String,
    pricing_profile: String,
    #[serde(default)]
    model_pattern: Option<String>,
    #[serde(default)]
    provider_type: Option<String>,
    rate_kind: String,
    usage_class: String,
    unit: String,
    /// Upstream catalogues publish list prices in USD, so this field is USD-denominated and
    /// the loader inserts `unit_price_currency = 'USD'` for every catalog row.
    unit_price_nano_usd: String,
    #[serde(default)]
    context_tier: Option<String>,
    #[serde(default)]
    service_tier: Option<String>,
    #[serde(default)]
    modality: Option<String>,
    #[serde(default)]
    cache_ttl: Option<String>,
    #[serde(default = "default_json_object")]
    match_json: Value,
    #[serde(default)]
    priority: i32,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_json_object")]
    raw_json: Value,
}

fn default_true() -> bool {
    true
}

fn default_json_object() -> Value {
    serde_json::json!({})
}

#[derive(Clone)]
pub struct BillingRateStore {
    db: DbPool,
}

impl BillingRateStore {
    pub async fn new(db: DbPool) -> Result<Self, String> {
        Ok(Self { db })
    }

    pub async fn list_billing_rates(&self) -> Result<Vec<DbBillingRateRecord>, String> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT id, source, pricing_profile, model_pattern, provider_type, rate_kind,
                        usage_class, unit, unit_price_nano, unit_price_currency, context_tier, service_tier,
                        modality, cache_ttl, match_json, priority, enabled, raw_json, updated_at
                 FROM billing_rate_records
                 ORDER BY pricing_profile ASC, priority DESC, id ASC",
                vec![],
            ))
            .await
            .map_err(|e| e.to_string())?;
        rows.iter().map(decode_billing_rate_row).collect()
    }

    pub async fn list_matching_rates(
        &self,
        pricing_profile: &str,
        provider_type: Option<&str>,
        model: &str,
    ) -> Result<Vec<DbBillingRateRecord>, String> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT id, source, pricing_profile, model_pattern, provider_type, rate_kind,
                        usage_class, unit, unit_price_nano, unit_price_currency, context_tier, service_tier,
                        modality, cache_ttl, match_json, priority, enabled, raw_json, updated_at
                 FROM billing_rate_records
                 WHERE enabled = 1
                   AND pricing_profile = $1
                   AND (provider_type IS NULL OR provider_type = $2)
                 ORDER BY priority DESC, id ASC",
                vec![pricing_profile.into(), provider_type.unwrap_or("").into()],
            ))
            .await
            .map_err(|e| e.to_string())?;

        let mut rates = Vec::with_capacity(rows.len());
        for row in &rows {
            let rate = decode_billing_rate_row(row)?;
            if rate
                .model_pattern
                .as_deref()
                .is_none_or(|pattern| glob_matches(pattern, model))
            {
                rates.push(rate);
            }
        }
        Ok(rates)
    }

    pub async fn list_matching_rates_for_profiles(
        &self,
        pricing_profiles: &[String],
        provider_type: Option<&str>,
        model: &str,
    ) -> Result<Vec<DbBillingRateRecord>, String> {
        if pricing_profiles.is_empty() {
            return Ok(Vec::new());
        }
        let pricing_profiles = pricing_profiles
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut rates = Vec::new();
        const PROFILE_LOOKUP_CHUNK_SIZE: usize = 399;
        for chunk in pricing_profiles.chunks(PROFILE_LOOKUP_CHUNK_SIZE) {
            let placeholders = (0..chunk.len())
                .map(|index| format!("${}", index + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let mut values: Vec<sea_orm::Value> = Vec::with_capacity(chunk.len() + 1);
            values.push(provider_type.unwrap_or("").into());
            values.extend(chunk.iter().cloned().map(Into::into));
            let rows = self
                .db
                .read()
                .query_all(self.db.stmt(
                    &format!(
                        "SELECT id, source, pricing_profile, model_pattern, provider_type, rate_kind,
                                usage_class, unit, unit_price_nano, unit_price_currency, context_tier, service_tier,
                                modality, cache_ttl, match_json, priority, enabled, raw_json, updated_at
                         FROM billing_rate_records
                         WHERE enabled = 1
                           AND (provider_type IS NULL OR provider_type = $1)
                           AND pricing_profile IN ({placeholders})
                         ORDER BY priority DESC, id ASC"
                    ),
                    values,
                ))
                .await
                .map_err(|e| e.to_string())?;
            for row in &rows {
                let rate = decode_billing_rate_row(row)?;
                if rate
                    .model_pattern
                    .as_deref()
                    .is_none_or(|pattern| glob_matches(pattern, model))
                {
                    rates.push(rate);
                }
            }
        }
        rates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(rates)
    }

    pub async fn list_candidate_rates_for_profiles_and_provider_types(
        &self,
        pricing_profiles: &[String],
        provider_types: &[String],
    ) -> Result<Vec<DbBillingRateRecord>, String> {
        if pricing_profiles.is_empty() || provider_types.is_empty() {
            return Ok(Vec::new());
        }
        let pricing_profiles = pricing_profiles
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let provider_types = provider_types
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut rates_by_id = std::collections::HashMap::new();
        const SET_LOOKUP_CHUNK_SIZE: usize = 200;
        for profile_chunk in pricing_profiles.chunks(SET_LOOKUP_CHUNK_SIZE) {
            for type_chunk in provider_types.chunks(SET_LOOKUP_CHUNK_SIZE) {
                let profile_placeholders = (0..profile_chunk.len())
                    .map(|index| format!("${}", index + 1))
                    .collect::<Vec<_>>();
                let type_placeholders = (0..type_chunk.len())
                    .map(|index| format!("${}", profile_chunk.len() + index + 1))
                    .collect::<Vec<_>>();
                let mut values: Vec<sea_orm::Value> =
                    profile_chunk.iter().cloned().map(Into::into).collect();
                values.extend(type_chunk.iter().cloned().map(Into::into));
                let rows = self
                    .db
                    .read()
                    .query_all(self.db.stmt(
                        &format!(
                            "SELECT id, source, pricing_profile, model_pattern, provider_type, rate_kind,
                                    usage_class, unit, unit_price_nano, unit_price_currency, context_tier, service_tier,
                                    modality, cache_ttl, match_json, priority, enabled, raw_json, updated_at
                             FROM billing_rate_records
                             WHERE enabled = 1
                               AND pricing_profile IN ({})
                               AND (provider_type IS NULL OR provider_type IN ({}))
                             ORDER BY priority DESC, id ASC",
                            profile_placeholders.join(", "),
                            type_placeholders.join(", ")
                        ),
                        values,
                    ))
                    .await
                    .map_err(|e| e.to_string())?;
                for row in &rows {
                    let rate = decode_billing_rate_row(row)?;
                    rates_by_id.insert(rate.id.clone(), rate);
                }
            }
        }
        let mut rates = rates_by_id.into_values().collect::<Vec<_>>();
        rates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(rates)
    }

    pub async fn upsert_billing_rate(
        &self,
        id: &str,
        input: UpsertBillingRateInput,
    ) -> Result<DbBillingRateRecord, String> {
        if id.trim().is_empty() {
            return Err("id must not be empty".to_string());
        }

        let write_guard = self.db.write().await;
        let txn = write_guard.begin().await.map_err(|e| e.to_string())?;
        if self.db.is_postgres() {
            txn.execute_unprepared("LOCK TABLE billing_rate_records IN SHARE ROW EXCLUSIVE MODE")
                .await
                .map_err(|e| e.to_string())?;
        }
        let lock_suffix = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        let existing_row = txn
            .query_one(self.db.stmt(
                &format!(
                    "SELECT id, source, pricing_profile, model_pattern, provider_type, rate_kind,
                            usage_class, unit, unit_price_nano, unit_price_currency, context_tier, service_tier,
                            modality, cache_ttl, match_json, priority, enabled, raw_json, updated_at
                     FROM billing_rate_records WHERE id = $1{lock_suffix}"
                ),
                vec![id.into()],
            ))
            .await
            .map_err(|e| e.to_string())?;
        let existing = existing_row
            .as_ref()
            .map(decode_billing_rate_row)
            .transpose()?;
        let source = input.source.unwrap_or_else(|| "manual".to_string());
        let pricing_profile = input
            .pricing_profile
            .or_else(|| existing.as_ref().map(|r| r.pricing_profile.clone()))
            .ok_or_else(|| "pricing_profile is required".to_string())?;
        let rate_kind = input
            .rate_kind
            .or_else(|| existing.as_ref().map(|r| r.rate_kind.clone()))
            .ok_or_else(|| "rate_kind is required".to_string())?;
        let usage_class = input
            .usage_class
            .or_else(|| existing.as_ref().map(|r| r.usage_class.clone()))
            .ok_or_else(|| "usage_class is required".to_string())?;
        let unit = input
            .unit
            .or_else(|| existing.as_ref().map(|r| r.unit.clone()))
            .ok_or_else(|| "unit is required".to_string())?;
        let unit_price_nano = input
            .unit_price_nano
            .or_else(|| existing.as_ref().map(|r| r.unit_price_nano.clone()))
            .ok_or_else(|| "unit_price_nano is required".to_string())?;
        let parsed_unit_price = unit_price_nano
            .parse::<i128>()
            .map_err(|_| "unit_price_nano must be an integer string".to_string())?;
        if parsed_unit_price < 0 || parsed_unit_price.to_string() != unit_price_nano {
            return Err(
                "unit_price_nano must be a canonical non-negative integer string".to_string(),
            );
        }
        // An existing rate keeps its currency, so an update that omits the field can never
        // silently re-denominate a stored price.
        let unit_price_currency = input
            .unit_price_currency
            .or_else(|| existing.as_ref().map(|r| r.unit_price_currency.clone()))
            .unwrap_or_else(|| DEFAULT_RATE_CURRENCY.to_string());
        if !matches!(
            unit_price_currency.as_str(),
            RATE_CURRENCY_USD | RATE_CURRENCY_CNY
        ) {
            return Err(format!(
                "unit_price_currency must be {RATE_CURRENCY_USD} or {RATE_CURRENCY_CNY}"
            ));
        }

        let model_pattern = input
            .model_pattern
            .unwrap_or_else(|| existing.as_ref().and_then(|r| r.model_pattern.clone()));
        let provider_type = input
            .provider_type
            .unwrap_or_else(|| existing.as_ref().and_then(|r| r.provider_type.clone()));
        let context_tier = input
            .context_tier
            .unwrap_or_else(|| existing.as_ref().and_then(|r| r.context_tier.clone()));
        let service_tier = input
            .service_tier
            .unwrap_or_else(|| existing.as_ref().and_then(|r| r.service_tier.clone()));
        let modality = input
            .modality
            .unwrap_or_else(|| existing.as_ref().and_then(|r| r.modality.clone()));
        let cache_ttl = input
            .cache_ttl
            .unwrap_or_else(|| existing.as_ref().and_then(|r| r.cache_ttl.clone()));
        let match_json = input
            .match_json
            .or_else(|| existing.as_ref().map(|r| r.match_json.clone()))
            .unwrap_or_else(|| serde_json::json!({}));
        let priority = input
            .priority
            .or_else(|| existing.as_ref().map(|r| r.priority))
            .unwrap_or(0);
        let enabled = input
            .enabled
            .or_else(|| existing.as_ref().map(|r| r.enabled))
            .unwrap_or(true);
        let raw_json = input
            .raw_json
            .or_else(|| existing.as_ref().map(|r| r.raw_json.clone()))
            .unwrap_or_else(|| serde_json::json!({}));
        require_json_object(id, "match_json", &match_json)?;
        require_json_object(id, "raw_json", &raw_json)?;
        let now = Utc::now().to_rfc3339();

        txn.execute(self.db.stmt(
                "INSERT INTO billing_rate_records
                 (id, source, pricing_profile, model_pattern, provider_type, rate_kind, usage_class,
                  unit, unit_price_nano, unit_price_currency, context_tier, service_tier, modality, cache_ttl,
                  match_json, priority, enabled, raw_json, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
                 ON CONFLICT(id) DO UPDATE SET
                   source = excluded.source,
                   pricing_profile = excluded.pricing_profile,
                   model_pattern = excluded.model_pattern,
                   provider_type = excluded.provider_type,
                   rate_kind = excluded.rate_kind,
                   usage_class = excluded.usage_class,
                   unit = excluded.unit,
                   unit_price_nano = excluded.unit_price_nano,
                   unit_price_currency = excluded.unit_price_currency,
                   context_tier = excluded.context_tier,
                   service_tier = excluded.service_tier,
                   modality = excluded.modality,
                   cache_ttl = excluded.cache_ttl,
                   match_json = excluded.match_json,
                   priority = excluded.priority,
                   enabled = excluded.enabled,
                   raw_json = excluded.raw_json,
                   updated_at = excluded.updated_at",
                vec![
                    id.to_string().into(),
                    source.into(),
                    pricing_profile.into(),
                    model_pattern.into(),
                    provider_type.into(),
                    rate_kind.into(),
                    usage_class.into(),
                    unit.into(),
                    unit_price_nano.into(),
                    unit_price_currency.into(),
                    context_tier.into(),
                    service_tier.into(),
                    modality.into(),
                    cache_ttl.into(),
                    match_json.to_string().into(),
                    priority.into(),
                    (if enabled { 1_i32 } else { 0_i32 }).into(),
                    raw_json.to_string().into(),
                    now.into(),
                ],
            ))
            .await
            .map_err(|e| e.to_string())?;

        txn.commit().await.map_err(|e| e.to_string())?;

        self.get_billing_rate(id)
            .await?
            .ok_or_else(|| "upsert succeeded but billing rate not found".to_string())
    }

    /// Copies every rate row of `source_profile` to `target_profile` (MB-A7).
    ///
    /// Returns the number of rows copied. The whole copy runs in one transaction, so a
    /// partially copied profile is never visible: a profile that bills traffic must be
    /// complete or absent.
    ///
    /// Copied rows become `manual` and take ids outside the `model_metadata:` namespace
    /// (MB-A7b). Both are required for the copy to survive: catalog sync deletes every
    /// `catalog` row, and deleting a model metadata record deletes every rate whose id starts
    /// with `model_metadata:`. A copy that inherited either property would vanish when its
    /// unrelated source was next synced or removed.
    pub async fn copy_profile(
        &self,
        source_profile: &str,
        target_profile: &str,
    ) -> Result<usize, CopyProfileError> {
        let target = target_profile.trim();
        if target.is_empty() {
            return Err(CopyProfileError::InvalidTarget);
        }
        if target == source_profile {
            return Err(CopyProfileError::SameProfile);
        }

        let write_guard = self.db.write().await;
        let txn = write_guard
            .begin()
            .await
            .map_err(|e| CopyProfileError::Storage(e.to_string()))?;
        if self.db.is_postgres() {
            txn.execute_unprepared("LOCK TABLE billing_rate_records IN SHARE ROW EXCLUSIVE MODE")
                .await
                .map_err(|e| CopyProfileError::Storage(e.to_string()))?;
        }

        let target_count: i64 = txn
            .query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM billing_rate_records WHERE pricing_profile = $1",
                vec![target.into()],
            ))
            .await
            .map_err(|e| CopyProfileError::Storage(e.to_string()))?
            .map(|row| row.try_get::<i64>("", "value").unwrap_or(0))
            .unwrap_or(0);
        if target_count > 0 {
            return Err(CopyProfileError::TargetNotEmpty);
        }

        let rows = txn
            .query_all(self.db.stmt(
                "SELECT id, model_pattern, provider_type, rate_kind, usage_class, unit,
                        unit_price_nano, unit_price_currency, context_tier, service_tier,
                        modality, cache_ttl, match_json, priority, enabled, raw_json
                 FROM billing_rate_records WHERE pricing_profile = $1 ORDER BY id ASC",
                vec![source_profile.into()],
            ))
            .await
            .map_err(|e| CopyProfileError::Storage(e.to_string()))?;
        if rows.is_empty() {
            return Err(CopyProfileError::SourceNotFound);
        }

        let now = Utc::now().to_rfc3339();
        for row in &rows {
            let source_id: String = row
                .try_get("", "id")
                .map_err(|e| CopyProfileError::Storage(e.to_string()))?;
            let copied_id = copied_rate_id(target, &source_id);
            txn.execute(self.db.stmt(
                "INSERT INTO billing_rate_records
                 (id, source, pricing_profile, model_pattern, provider_type, rate_kind,
                  usage_class, unit, unit_price_nano, unit_price_currency, context_tier,
                  service_tier, modality, cache_ttl, match_json, priority, enabled, raw_json,
                  updated_at)
                 SELECT $1, 'manual', $2, model_pattern, provider_type, rate_kind, usage_class,
                        unit, unit_price_nano, unit_price_currency, context_tier, service_tier,
                        modality, cache_ttl, match_json, priority, enabled, raw_json, $4
                 FROM billing_rate_records WHERE id = $3",
                vec![
                    copied_id.into(),
                    target.into(),
                    source_id.into(),
                    now.clone().into(),
                ],
            ))
            .await
            .map_err(|e| CopyProfileError::Storage(e.to_string()))?;
        }
        txn.commit()
            .await
            .map_err(|e| CopyProfileError::Storage(e.to_string()))?;
        Ok(rows.len())
    }

    pub async fn get_billing_rate(&self, id: &str) -> Result<Option<DbBillingRateRecord>, String> {
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT id, source, pricing_profile, model_pattern, provider_type, rate_kind,
                        usage_class, unit, unit_price_nano, unit_price_currency, context_tier, service_tier,
                        modality, cache_ttl, match_json, priority, enabled, raw_json, updated_at
                 FROM billing_rate_records
                 WHERE id = $1",
                vec![id.into()],
            ))
            .await
            .map_err(|e| e.to_string())?;
        row.as_ref().map(decode_billing_rate_row).transpose()
    }

    pub async fn delete_billing_rate(&self, id: &str) -> Result<bool, String> {
        let result = self
            .db
            .write()
            .await
            .execute(self.db.stmt(
                "DELETE FROM billing_rate_records WHERE id = $1",
                vec![id.into()],
            ))
            .await
            .map_err(|e| e.to_string())?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn sync_catalog(&self) -> Result<BillingRateSyncResult, String> {
        let catalog: CatalogRoot = serde_json::from_str(BILLING_RATE_CATALOG)
            .map_err(|e| format!("catalog_parse_failed: {e}"))?;
        for rate in &catalog.rates {
            let parsed = rate.unit_price_nano_usd.parse::<i128>().map_err(|_| {
                format!(
                    "catalog_parse_failed: invalid unit_price_nano_usd for {}",
                    rate.id
                )
            })?;
            if parsed < 0 || parsed.to_string() != rate.unit_price_nano_usd {
                return Err(format!(
                    "catalog_parse_failed: non-canonical unit_price_nano_usd for {}",
                    rate.id
                ));
            }
            require_json_object(&rate.id, "match_json", &rate.match_json)
                .map_err(|error| format!("catalog_parse_failed: {error}"))?;
            require_json_object(&rate.id, "raw_json", &rate.raw_json)
                .map_err(|error| format!("catalog_parse_failed: {error}"))?;
        }
        let fetched_at = Utc::now().to_rfc3339();
        let _write_guard = self.db.write().await;
        let txn = _write_guard.begin().await.map_err(|e| e.to_string())?;

        let manual_rows = txn
            .query_all(self.db.stmt(
                "SELECT id FROM billing_rate_records WHERE source = 'manual'",
                vec![],
            ))
            .await
            .map_err(|e| e.to_string())?;
        let manual_ids: HashSet<String> = manual_rows
            .iter()
            .map(|row| {
                row.try_get::<String>("", "id")
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<_, _>>()?;

        let del_result = txn
            .execute(self.db.stmt(
                "DELETE FROM billing_rate_records WHERE source = 'catalog'",
                vec![],
            ))
            .await
            .map_err(|e| e.to_string())?;
        let deleted = del_result.rows_affected();

        let mut skipped = 0usize;
        let mut writes = Vec::with_capacity(catalog.rates.len());
        for rate in catalog.rates {
            if manual_ids.contains(&rate.id) {
                skipped += 1;
                continue;
            }
            writes.push(rate);
        }

        const CATALOG_SYNC_CHUNK_SIZE: usize = 23;
        for chunk in writes.chunks(CATALOG_SYNC_CHUNK_SIZE) {
            let mut values: Vec<sea_orm::Value> = Vec::with_capacity(chunk.len() * 18);
            let mut rows = Vec::with_capacity(chunk.len());
            for rate in chunk {
                let start = values.len() + 1;
                values.extend([
                    rate.id.clone().into(),
                    rate.pricing_profile.clone().into(),
                    rate.model_pattern.clone().into(),
                    rate.provider_type.clone().into(),
                    rate.rate_kind.clone().into(),
                    rate.usage_class.clone().into(),
                    rate.unit.clone().into(),
                    rate.unit_price_nano_usd.clone().into(),
                    RATE_CURRENCY_USD.into(),
                    rate.context_tier.clone().into(),
                    rate.service_tier.clone().into(),
                    rate.modality.clone().into(),
                    rate.cache_ttl.clone().into(),
                    rate.match_json.to_string().into(),
                    rate.priority.into(),
                    (if rate.enabled { 1_i32 } else { 0_i32 }).into(),
                    rate.raw_json.to_string().into(),
                    fetched_at.clone().into(),
                ]);
                let mut placeholders = vec![format!("${start}"), "'catalog'".to_string()];
                placeholders.extend((start + 1..start + 18).map(|index| format!("${index}")));
                rows.push(format!("({})", placeholders.join(", ")));
            }
            txn.execute(self.db.stmt(
                &format!(
                    "INSERT INTO billing_rate_records
                     (id, source, pricing_profile, model_pattern, provider_type, rate_kind,
                      usage_class, unit, unit_price_nano, unit_price_currency, context_tier, service_tier,
                      modality, cache_ttl, match_json, priority, enabled, raw_json, updated_at)
                     VALUES {}",
                    rows.join(", ")
                ),
                values,
            ))
            .await
            .map_err(|e| e.to_string())?;
        }

        txn.commit().await.map_err(|e| e.to_string())?;
        let upserted = writes.len();
        Ok(BillingRateSyncResult {
            success: true,
            upserted,
            skipped,
            deleted,
            fetched_at,
        })
    }
}

pub fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut last_star_index = None;
    let mut last_star_match_index = 0;

    while value_index < value.len() {
        if pattern_index < pattern.len()
            && pattern[pattern_index] != b'*'
            && (pattern[pattern_index] == b'?'
                || pattern[pattern_index].eq_ignore_ascii_case(&value[value_index]))
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            last_star_index = Some(pattern_index);
            pattern_index += 1;
            last_star_match_index = value_index;
        } else if let Some(star_index) = last_star_index {
            last_star_match_index += 1;
            value_index = last_star_match_index;
            pattern_index = star_index + 1;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

pub fn select_pricing_profile<'a>(
    patterns: &'a [PricingProfilePattern],
    model: &str,
) -> Option<&'a str> {
    patterns
        .iter()
        .find(|entry| glob_matches(&entry.pattern, model))
        .map(|entry| entry.pricing_profile.as_str())
}

fn require_json_object(id: &str, column: &str, value: &Value) -> Result<(), String> {
    if value.is_object() {
        Ok(())
    } else {
        Err(format!("billing rate {id} {column} must be a JSON object"))
    }
}

fn decode_json_object(id: &str, column: &str, raw: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| format!("invalid billing_rate_records.{column} for row {id}: {error}"))?;
    require_json_object(id, column, &value)?;
    Ok(value)
}

fn decode_billing_rate_row(row: &sea_orm::QueryResult) -> Result<DbBillingRateRecord, String> {
    let id: String = row.try_get("", "id").map_err(|e| e.to_string())?;
    let match_json_raw: String = row.try_get("", "match_json").map_err(|e| e.to_string())?;
    let raw_json_raw: String = row.try_get("", "raw_json").map_err(|e| e.to_string())?;
    let match_json = decode_json_object(&id, "match_json", &match_json_raw)?;
    let raw_json = decode_json_object(&id, "raw_json", &raw_json_raw)?;
    let updated_at_raw: String = row.try_get("", "updated_at").map_err(|e| e.to_string())?;
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_raw)
        .map_err(|e| e.to_string())?
        .with_timezone(&Utc);
    let enabled_i: i32 = row.try_get("", "enabled").map_err(|e| e.to_string())?;

    Ok(DbBillingRateRecord {
        id,
        source: row.try_get("", "source").map_err(|e| e.to_string())?,
        pricing_profile: row
            .try_get("", "pricing_profile")
            .map_err(|e| e.to_string())?,
        model_pattern: row
            .try_get("", "model_pattern")
            .map_err(|e| e.to_string())?,
        provider_type: row
            .try_get("", "provider_type")
            .map_err(|e| e.to_string())?,
        rate_kind: row.try_get("", "rate_kind").map_err(|e| e.to_string())?,
        usage_class: row.try_get("", "usage_class").map_err(|e| e.to_string())?,
        unit: row.try_get("", "unit").map_err(|e| e.to_string())?,
        unit_price_nano: row
            .try_get("", "unit_price_nano")
            .map_err(|e| e.to_string())?,
        unit_price_currency: row
            .try_get("", "unit_price_currency")
            .map_err(|e| e.to_string())?,
        context_tier: row.try_get("", "context_tier").map_err(|e| e.to_string())?,
        service_tier: row.try_get("", "service_tier").map_err(|e| e.to_string())?,
        modality: row.try_get("", "modality").map_err(|e| e.to_string())?,
        cache_ttl: row.try_get("", "cache_ttl").map_err(|e| e.to_string())?,
        match_json,
        priority: row.try_get("", "priority").map_err(|e| e.to_string())?,
        enabled: enabled_i != 0,
        raw_json,
        updated_at,
    })
}

#[cfg(test)]
mod tests {

    use super::{
        BillingRateStore, CopyProfileError, UpsertBillingRateInput, glob_matches,
        select_pricing_profile,
    };
    use crate::db::DbPool;
    use crate::migration::Migrator;
    use crate::settings::PricingProfilePattern;
    use sea_orm::ConnectionTrait;
    use sea_orm_migration::MigratorTrait;

    async fn store_with_rate(profile: &str, id: &str, price: &str) -> BillingRateStore {
        let db = DbPool::connect("sqlite::memory:").await.expect("connect");
        {
            let write = db.write().await;
            Migrator::up(&*write, None)
                .await
                .expect("migrate");
        }
        let store = BillingRateStore::new(db).await.expect("store");
        store
            .upsert_billing_rate(
                id,
                UpsertBillingRateInput {
                    source: Some("catalog".to_string()),
                    pricing_profile: Some(profile.to_string()),
                    model_pattern: Some(Some("deepseek-*".to_string())),
                    rate_kind: Some("token".to_string()),
                    usage_class: Some("input".to_string()),
                    unit: Some("token".to_string()),
                    unit_price_nano: Some(price.to_string()),
                    unit_price_currency: Some("CNY".to_string()),
                    priority: Some(7),
                    enabled: Some(true),
                    provider_type: None,
                    context_tier: None,
                    service_tier: None,
                    modality: None,
                    cache_ttl: None,
                    match_json: None,
                    raw_json: None,
                },
            )
            .await
            .expect("seed rate");
        store
    }

    /// MB-A7b: a copy must keep every price digit but must not keep the properties that would
    /// let an unrelated sync delete it.
    #[tokio::test]
    async fn copying_a_profile_preserves_prices_and_detaches_from_sync() {
        let store = store_with_rate("DeepSeek", "catalog:deepseek:input", "1234").await;

        let copied = store.copy_profile("DeepSeek", "deepseek-std").await.unwrap();
        assert_eq!(copied, 1);

        let rows = store.list_billing_rates().await.unwrap();
        let copy = rows
            .iter()
            .find(|row| row.pricing_profile == "deepseek-std")
            .expect("the copied row must exist");
        assert_eq!(copy.unit_price_nano, "1234", "prices must copy exactly");
        assert_eq!(copy.unit_price_currency, "CNY");
        assert_eq!(copy.priority, 7);
        assert_eq!(copy.model_pattern.as_deref(), Some("deepseek-*"));
        assert_eq!(
            copy.source, "manual",
            "a catalog source would be deleted by the next catalog sync"
        );
        assert!(
            !copy.id.starts_with("model_metadata:"),
            "that namespace is deleted with its metadata record: {}",
            copy.id
        );

        // The source is untouched.
        let source = rows
            .iter()
            .find(|row| row.pricing_profile == "DeepSeek")
            .expect("source row");
        assert_eq!(source.source, "catalog");
        assert_eq!(source.unit_price_nano, "1234");
    }

    /// MB-A7a: copying onto a profile that already prices traffic would silently reprice it,
    /// so a non-empty target is refused and a repeated call fails rather than duplicating.
    #[tokio::test]
    async fn copying_refuses_a_target_that_already_has_rates() {
        let store = store_with_rate("DeepSeek", "catalog:deepseek:input", "1234").await;
        store.copy_profile("DeepSeek", "deepseek-std").await.unwrap();

        assert_eq!(
            store.copy_profile("DeepSeek", "deepseek-std").await,
            Err(CopyProfileError::TargetNotEmpty)
        );
        assert_eq!(
            store.list_billing_rates().await.unwrap().len(),
            2,
            "the refused copy must not add rows"
        );
    }

    #[tokio::test]
    async fn copying_rejects_an_unusable_target_or_source() {
        let store = store_with_rate("DeepSeek", "catalog:deepseek:input", "1234").await;
        assert_eq!(
            store.copy_profile("DeepSeek", "   ").await,
            Err(CopyProfileError::InvalidTarget)
        );
        assert_eq!(
            store.copy_profile("DeepSeek", "DeepSeek").await,
            Err(CopyProfileError::SameProfile)
        );
        assert_eq!(
            store.copy_profile("absent", "somewhere").await,
            Err(CopyProfileError::SourceNotFound)
        );
        assert_eq!(store.list_billing_rates().await.unwrap().len(), 1);
    }

    #[test]
    fn glob_matching_is_case_insensitive_and_orderable() {
        assert!(glob_matches("gpt-*", "GPT-5.5"));
        assert!(glob_matches("claude-sonnet-4?", "claude-sonnet-45"));
        assert!(!glob_matches("claude-opus-*", "claude-sonnet-4"));
    }

    #[test]
    fn glob_matching_handles_long_values_without_recursive_stack_growth() {
        let value = format!("{}Z", "a".repeat(200_000));
        assert!(glob_matches("*a*a*a*?", &value));
        assert!(!glob_matches("*a*a*a*y", &value));
    }

    #[test]
    fn glob_matching_preserves_multiple_star_and_question_semantics() {
        assert!(glob_matches("**a***b?c**", "xxAyybZc-tail"));
        assert!(glob_matches("***", "anything"));
        assert!(glob_matches("a**", "A"));
        assert!(!glob_matches("*a?b*", "ab"));
    }

    #[test]
    fn pricing_profile_selection_uses_ordered_first_match() {
        let patterns = vec![
            PricingProfilePattern {
                pattern: "gpt-*".to_string(),
                pricing_profile: "first".to_string(),
            },
            PricingProfilePattern {
                pattern: "gpt-image-*".to_string(),
                pricing_profile: "second".to_string(),
            },
            PricingProfilePattern {
                pattern: "*".to_string(),
                pricing_profile: "fallback".to_string(),
            },
        ];

        assert_eq!(
            select_pricing_profile(&patterns, "gpt-image-2"),
            Some("first")
        );
        assert_eq!(
            select_pricing_profile(&patterns, "claude-opus-4"),
            Some("fallback")
        );
    }

    #[tokio::test]
    async fn sqlite_billing_rate_json_decode_is_fail_closed() {
        let db = DbPool::connect("sqlite::memory:")
            .await
            .expect("db connects");
        {
            let write = db.write().await;
            Migrator::up(&*write, None).await.expect("migrates");
        }
        let store = BillingRateStore::new(db.clone())
            .await
            .expect("store creates");
        db.write()
            .await
            .execute(db.stmt(
                "INSERT INTO billing_rate_records
                 (id, source, pricing_profile, rate_kind, usage_class, unit,
                  unit_price_nano, match_json, priority, enabled, raw_json, updated_at)
                 VALUES ($1, 'manual', 'corrupt-test', 'token', 'input_uncached', 'token',
                         '1', $2, 0, 1, $3, '2026-01-01T00:00:00+00:00')",
                vec!["corrupt-json".into(), "{not-json".into(), "{}".into()],
            ))
            .await
            .expect("corrupt row inserts");

        let error = store
            .list_matching_rates("corrupt-test", None, "any-model")
            .await
            .expect_err("malformed match_json must fail the complete lookup");
        assert!(error.contains("corrupt-json"));
        assert!(error.contains("match_json"));

        db.write()
            .await
            .execute(db.stmt(
                "UPDATE billing_rate_records SET match_json = '{}', raw_json = $1 WHERE id = $2",
                vec!["not-json".into(), "corrupt-json".into()],
            ))
            .await
            .expect("raw json corrupts");
        let error = store
            .get_billing_rate("corrupt-json")
            .await
            .expect_err("malformed raw_json must fail the point lookup");
        assert!(error.contains("corrupt-json"));
        assert!(error.contains("raw_json"));

        db.write()
            .await
            .execute(db.stmt(
                "UPDATE billing_rate_records SET match_json = '[]', raw_json = '{}' WHERE id = $1",
                vec!["corrupt-json".into()],
            ))
            .await
            .expect("non-object match json stores");
        let error = store
            .get_billing_rate("corrupt-json")
            .await
            .expect_err("non-object match_json must fail the point lookup");
        assert!(error.contains("match_json must be a JSON object"));

        let error = store
            .upsert_billing_rate(
                "invalid-input",
                UpsertBillingRateInput {
                    source: None,
                    pricing_profile: Some("corrupt-test".to_string()),
                    model_pattern: None,
                    provider_type: None,
                    rate_kind: Some("token".to_string()),
                    usage_class: Some("output".to_string()),
                    unit: Some("token".to_string()),
                    unit_price_nano: Some("1".to_string()),
                    unit_price_currency: None,
                    context_tier: None,
                    service_tier: None,
                    modality: None,
                    cache_ttl: None,
                    match_json: Some(serde_json::json!([])),
                    priority: None,
                    enabled: None,
                    raw_json: Some(serde_json::json!({})),
                },
            )
            .await
            .expect_err("upsert must reject a non-object match_json");
        assert!(error.contains("match_json must be a JSON object"));
        assert!(
            store
                .get_billing_rate("invalid-input")
                .await
                .expect("point lookup succeeds")
                .is_none()
        );
    }
}

/// Failure modes of [`BillingRateStore::copy_profile`] (MB-A7a).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CopyProfileError {
    #[error("target profile must not be empty")]
    InvalidTarget,
    #[error("target profile must differ from the source profile")]
    SameProfile,
    #[error("source profile has no rates")]
    SourceNotFound,
    #[error("target profile already has rates")]
    TargetNotEmpty,
    #[error("storage failure: {0}")]
    Storage(String),
}

/// Builds the id of a copied rate row (MB-A7b).
///
/// The target profile leads so copies of one profile sort together, and the source id is
/// retained so a copied row can be traced back to what it was copied from.
fn copied_rate_id(target_profile: &str, source_id: &str) -> String {
    format!("manual:profile-copy:{target_profile}:{source_id}")
}
