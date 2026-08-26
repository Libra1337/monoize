use super::{
    EndpointKind, Envelope, FixtureManifest, ListCursor, ListKey, MarketplaceQuery, OfferCursor,
    OfferKey, OfferQueryInput, QueryCase, QueryInput, QueryKind, canonical_filter_digest,
};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Executor, QueryBuilder, Row, Sqlite, SqliteConnection};
use std::collections::BTreeMap;
use std::time::Instant;
use sysinfo::{ProcessesToUpdate, System, get_current_pid};

use crate::public_contract::{
    MarketplaceItem, MarketplaceListResponse, OfferRate, OfferResponse, ProviderOffer, RateRange,
    encode_public,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkMode {
    Smoke,
    Qualification,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchmarkConfig {
    pub seed: u64,
    pub envelope: Envelope,
    pub mode: BenchmarkMode,
    pub query_limit: Option<usize>,
    pub git_commit: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub schema_version: u8,
    pub backend: String,
    pub mode: BenchmarkMode,
    pub envelope: Envelope,
    pub git_commit: String,
    pub fixture_recipe_sha256: String,
    pub loaded_source_sha256: String,
    pub query_set_sha256: String,
    pub loaded_groups: u64,
    pub loaded_providers: u64,
    pub loaded_provider_models: u64,
    pub loaded_rate_rows: u64,
    pub loaded_metadata_rows: u64,
    pub declared_offer_rate_entries: u64,
    pub materialized_offer_rate_entries: u64,
    pub samples: u64,
    pub failed_samples: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub statement_count: u64,
    pub response_bytes: u64,
    pub p50_microseconds: u64,
    pub p95_microseconds: u64,
    pub p99_microseconds: u64,
    pub elapsed_milliseconds: u64,
    pub cpu_milliseconds: u64,
    pub rss_before_bytes: u64,
    pub rss_after_bytes: u64,
    pub rss_delta_bytes: i64,
    pub workers: u16,
    pub warmup_seconds: u64,
    pub measured_seconds: u64,
    pub gate_b_qualified: bool,
    pub qualification_blockers: Vec<String>,
    pub list: OperationMetrics,
    pub offers: OperationMetrics,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationMetrics {
    pub samples: u64,
    pub failed_samples: u64,
    pub statement_count: u64,
    pub response_bytes: u64,
    pub max_response_bytes: u64,
    pub p50_microseconds: u64,
    pub p95_microseconds: u64,
    pub p99_microseconds: u64,
}

#[derive(Default)]
struct OperationAccumulator {
    latencies: Vec<u64>,
    failed_samples: u64,
    statement_count: u64,
    response_bytes: u64,
    max_response_bytes: u64,
}

struct LoadedFixture {
    source_sha256: String,
    groups: u64,
    providers: u64,
    provider_models: u64,
    rate_rows: u64,
    metadata_rows: u64,
    materialized_offer_rate_entries: u64,
}

struct PreparedQuery {
    query: QueryCase,
    list_after: Option<ListKey>,
    offer_after: Option<OfferKey>,
    expected_list: Option<ExpectedListPage>,
    expected_offers: Option<ExpectedOfferPage>,
}

struct ExpectedListPage {
    items: Vec<(String, u64)>,
    next_key: Option<ListKey>,
}

struct ExpectedOfferPage {
    items: Vec<OfferKey>,
    next_key: Option<OfferKey>,
}

impl BenchmarkConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.mode == BenchmarkMode::Qualification && self.envelope != Envelope::Qualification {
            bail!("qualification_requires_maximum_envelope");
        }
        if self.query_limit == Some(0) {
            bail!("query_limit_must_be_positive");
        }
        if self.git_commit.is_empty() {
            bail!("git_commit_is_required");
        }
        Ok(())
    }
}

pub async fn run_sqlite_benchmark(config: BenchmarkConfig) -> anyhow::Result<BenchmarkReport> {
    config.validate()?;
    let fixture_manifest = FixtureManifest::generate(config.seed, config.envelope)?;
    let query_set = fixture_manifest.query_set();
    let query_set_sha256 = hash_json(&query_set)?;
    let temporary = tempfile::TempDir::new().context("create benchmark directory")?;
    let options = SqliteConnectOptions::new()
        .filename(temporary.path().join("marketplace.sqlite3"))
        .create_if_missing(true);
    let mut database = SqliteConnection::connect_with(&options).await?;
    configure_sqlite(&mut database).await?;
    create_sqlite_schema(&mut database).await?;
    let loaded_fixture = load_sqlite_fixture(&mut database, &fixture_manifest).await?;
    database.execute("ANALYZE").await?;

    let cursor_key = [0x4c; 32];
    let prepared_queries = prepare_queries(&query_set, &fixture_manifest)?;
    let materialized_offer_rate_entries = loaded_fixture.materialized_offer_rate_entries;

    let (rss_before_bytes, cpu_before_milliseconds) = process_sample()?;
    let started = Instant::now();
    let mut list_metrics = OperationAccumulator::default();
    let mut offer_metrics = OperationAccumulator::default();
    let selected = config
        .query_limit
        .unwrap_or(prepared_queries.len())
        .min(prepared_queries.len());
    for prepared in prepared_queries.into_iter().take(selected) {
        let PreparedQuery {
            query,
            list_after,
            offer_after,
            expected_list,
            expected_offers,
        } = prepared;
        let sample_started = Instant::now();
        let (valid, bytes, statements) = match query.kind {
            QueryKind::List => {
                execute_list_sample(
                    &mut database,
                    &query,
                    list_after,
                    expected_list.as_ref().context("expected list page")?,
                    &cursor_key,
                )
                .await?
            }
            QueryKind::Offers => {
                execute_offer_sample(
                    &mut database,
                    &query,
                    offer_after,
                    expected_offers.as_ref().context("expected offer page")?,
                    &cursor_key,
                )
                .await?
            }
        };
        let target = match query.kind {
            QueryKind::List => &mut list_metrics,
            QueryKind::Offers => &mut offer_metrics,
        };
        target.record(micros(sample_started.elapsed()), valid, bytes, statements);
    }
    let elapsed = started.elapsed();
    let (rss_after_bytes, cpu_after_milliseconds) = process_sample()?;
    let list = list_metrics.finish()?;
    let offers = offer_metrics.finish()?;
    let samples = list.samples.saturating_add(offers.samples);
    let failed_samples = list.failed_samples.saturating_add(offers.failed_samples);
    let statement_count = list.statement_count.saturating_add(offers.statement_count);
    let response_bytes = list.response_bytes.saturating_add(offers.response_bytes);
    let mut combined_latencies = list_metrics.latencies;
    combined_latencies.extend(offer_metrics.latencies);
    combined_latencies.sort_unstable();
    let qualification_blockers = qualification_blockers(config.mode);

    Ok(BenchmarkReport {
        schema_version: 1,
        backend: "sqlite".to_owned(),
        mode: config.mode,
        envelope: config.envelope,
        git_commit: config.git_commit,
        fixture_recipe_sha256: fixture_manifest.sha256,
        loaded_source_sha256: loaded_fixture.source_sha256,
        query_set_sha256,
        loaded_groups: loaded_fixture.groups,
        loaded_providers: loaded_fixture.providers,
        loaded_provider_models: loaded_fixture.provider_models,
        loaded_rate_rows: loaded_fixture.rate_rows,
        loaded_metadata_rows: loaded_fixture.metadata_rows,
        declared_offer_rate_entries: fixture_manifest.offer_rate_entries,
        materialized_offer_rate_entries,
        samples,
        failed_samples,
        cache_hits: 0,
        cache_misses: samples,
        statement_count,
        response_bytes,
        p50_microseconds: percentile(&combined_latencies, 50),
        p95_microseconds: percentile(&combined_latencies, 95),
        p99_microseconds: percentile(&combined_latencies, 99),
        elapsed_milliseconds: millis(elapsed),
        cpu_milliseconds: cpu_after_milliseconds.saturating_sub(cpu_before_milliseconds),
        rss_before_bytes,
        rss_after_bytes,
        rss_delta_bytes: signed_delta(rss_after_bytes, rss_before_bytes),
        workers: 1,
        warmup_seconds: 0,
        measured_seconds: elapsed.as_secs(),
        gate_b_qualified: false,
        qualification_blockers,
        list,
        offers,
    })
}

fn prepare_queries(
    queries: &[QueryCase],
    manifest: &FixtureManifest,
) -> anyhow::Result<Vec<PreparedQuery>> {
    let fixture = manifest.fixture();
    let mut offer_count_by_model = BTreeMap::<String, u64>::new();
    for row in fixture.provider_models() {
        *offer_count_by_model.entry(row.model_name).or_default() += 1;
    }
    queries
        .iter()
        .cloned()
        .map(|query| {
            let list_after = if query.kind == QueryKind::List {
                let filtered = offer_count_by_model
                    .iter()
                    .filter(|(model, _)| {
                        query
                            .query
                            .as_deref()
                            .is_none_or(|value| model.to_ascii_lowercase().contains(value))
                    })
                    .map(|(model, count)| (model.clone(), *count))
                    .collect::<Vec<_>>();
                list_after_key(&filtered, query.cursor_position, query.limit)
            } else {
                None
            };
            let offer_after = if query.kind == QueryKind::Offers {
                offer_after_key(manifest.providers, query.cursor_position, query.limit)
            } else {
                None
            };
            let expected_list = (query.kind == QueryKind::List)
                .then(|| expected_list_page(&offer_count_by_model, &query, list_after.clone()));
            let expected_offers = (query.kind == QueryKind::Offers)
                .then(|| expected_offer_page(manifest.providers, query.limit, offer_after.clone()));
            Ok(PreparedQuery {
                query,
                list_after,
                offer_after,
                expected_list,
                expected_offers,
            })
        })
        .collect()
}

fn list_after_key(models: &[(String, u64)], position: u8, limit: u16) -> Option<ListKey> {
    let after_index = match position {
        0 => return None,
        1 => models.len().checked_div(2)?.checked_sub(1)?,
        2 => models
            .len()
            .checked_sub(usize::from(limit))?
            .checked_sub(1)?,
        _ => return None,
    };
    models.get(after_index).map(|(model, _)| ListKey {
        group_ordinal: 0,
        model_name: model.clone(),
    })
}

fn expected_list_page(
    offer_count_by_model: &BTreeMap<String, u64>,
    query: &QueryCase,
    after: Option<ListKey>,
) -> ExpectedListPage {
    let mut candidates = offer_count_by_model
        .iter()
        .filter(|(model, _)| {
            query
                .query
                .as_deref()
                .is_none_or(|search| model.to_ascii_lowercase().contains(search))
        })
        .filter(|(model, _)| {
            after
                .as_ref()
                .is_none_or(|key| model.as_bytes() > key.model_name.as_bytes())
        })
        .map(|(model, count)| (model.clone(), *count))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let has_more = candidates.len() > usize::from(query.limit);
    let items = candidates
        .into_iter()
        .take(usize::from(query.limit))
        .collect::<Vec<_>>();
    ExpectedListPage {
        next_key: has_more.then(|| ListKey {
            group_ordinal: 0,
            model_name: items.last().expect("non-empty list page").0.clone(),
        }),
        items,
    }
}

fn expected_offer_page(
    provider_count: u64,
    limit: u16,
    after: Option<OfferKey>,
) -> ExpectedOfferPage {
    let start = after
        .as_ref()
        .and_then(|key| u64::try_from(key.priority).ok())
        .and_then(|value| value.checked_add(1))
        .unwrap_or(0);
    let end = start.saturating_add(u64::from(limit)).min(provider_count);
    let items = (start..end)
        .map(|index| OfferKey {
            priority: i32::try_from(index).expect("provider index fits i32"),
            provider_public_name: format!("Provider {index:05}"),
            channel_public_name: format!("Channel {index:05}"),
        })
        .collect::<Vec<_>>();
    let has_more = end < provider_count;
    ExpectedOfferPage {
        next_key: has_more.then(|| items.last().expect("non-empty offer page").clone()),
        items,
    }
}

fn offer_after_key(provider_count: u64, position: u8, limit: u16) -> Option<OfferKey> {
    let after_index = match position {
        0 => return None,
        1 => provider_count.checked_div(2)?.checked_sub(1)?,
        2 => provider_count
            .checked_sub(u64::from(limit))?
            .checked_sub(1)?,
        _ => return None,
    };
    Some(OfferKey {
        priority: i32::try_from(after_index).ok()?,
        provider_public_name: format!("Provider {after_index:05}"),
        channel_public_name: format!("Channel {after_index:05}"),
    })
}

async fn execute_list_sample(
    database: &mut SqliteConnection,
    query: &QueryCase,
    after: Option<ListKey>,
    expected: &ExpectedListPage,
    cursor_key: &[u8; 32],
) -> anyhow::Result<(bool, u64, u64)> {
    let page = MarketplaceQuery::list_sqlite(
        database,
        QueryInput {
            query: query.query.clone(),
            group: query.group.clone(),
            after,
            limit: query.limit,
        },
    )
    .await?;
    let mut statements = 1_u64;
    let model_names = page
        .items
        .iter()
        .map(|item| item.model.clone())
        .collect::<Vec<_>>();
    let (enrichment, enrichment_statements) = load_enrichment(database, &model_names).await?;
    statements = statements.saturating_add(enrichment_statements);
    let next_cursor = page
        .next_key
        .as_ref()
        .map(|key| {
            let q = query.query.as_deref().unwrap_or_default();
            let group = query.group.as_deref().unwrap_or_default();
            let digest = canonical_filter_digest(EndpointKind::List, &[(1, q), (2, group)]);
            let ordinal = u64::try_from(key.group_ordinal).map_err(anyhow::Error::msg)?;
            ListCursor::new(1, query.limit, digest, ordinal, &key.model_name)
                .and_then(|cursor| cursor.encode(cursor_key))
                .map_err(|_| anyhow::anyhow!("encode list cursor"))
        })
        .transpose()?;
    let items = page
        .items
        .iter()
        .map(|item| marketplace_item(item, &enrichment))
        .collect::<Vec<_>>();
    let bytes = encode_public(&MarketplaceListResponse {
        generated_at: "2026-08-26T00:00:00.000000Z".to_owned(),
        revision: "1".to_owned(),
        next_cursor,
        items,
    })
    .map_err(|_| anyhow::anyhow!("encode list response"))?;
    let valid = validate_exact_list_page(&page, expected)
        && enrichment.covers(&model_names)
        && bytes.len() <= 1_048_576;
    Ok((valid, bytes.len() as u64, statements))
}

async fn execute_offer_sample(
    database: &mut SqliteConnection,
    query: &QueryCase,
    after: Option<OfferKey>,
    expected: &ExpectedOfferPage,
    cursor_key: &[u8; 32],
) -> anyhow::Result<(bool, u64, u64)> {
    let group = query.group.clone().context("offers query group")?;
    let model = query.model.clone().context("offers query model")?;
    let page = MarketplaceQuery::offers_sqlite(
        database,
        OfferQueryInput {
            group: group.clone(),
            model: model.clone(),
            after,
            limit: query.limit,
        },
    )
    .await?;
    let mut statements = 1_u64;
    let (enrichment, enrichment_statements) =
        load_enrichment(database, std::slice::from_ref(&model)).await?;
    statements = statements.saturating_add(enrichment_statements);
    let next_cursor = page
        .next_key
        .as_ref()
        .map(|key| {
            let digest = canonical_filter_digest(EndpointKind::Offers, &[(1, &group), (2, &model)]);
            OfferCursor::new(
                1,
                query.limit,
                digest,
                key.priority,
                &key.provider_public_name,
                &key.channel_public_name,
            )
            .and_then(|cursor| cursor.encode(cursor_key))
            .map_err(|_| anyhow::anyhow!("encode offer cursor"))
        })
        .transpose()?;
    let rates = enrichment.offer_rates(&model);
    let offers = page
        .items
        .iter()
        .map(|item| ProviderOffer {
            public_provider_name: item.provider_public_name.clone(),
            public_channel_name: item.channel_public_name.clone(),
            api_type: "responses".to_owned(),
            rates: rates.clone(),
        })
        .collect::<Vec<_>>();
    let bytes = encode_public(&OfferResponse {
        generated_at: "2026-08-26T00:00:00.000000Z".to_owned(),
        revision: "1".to_owned(),
        public_group_name: group,
        model: model.clone(),
        next_cursor,
        offers,
    })
    .map_err(|_| anyhow::anyhow!("encode offers response"))?;
    let valid = validate_exact_offer_page(&page, expected)
        && enrichment.covers(std::slice::from_ref(&model))
        && !rates.is_empty()
        && bytes.len() <= 1_048_576;
    Ok((valid, bytes.len() as u64, statements))
}

#[derive(Default)]
struct Enrichment {
    capabilities: BTreeMap<String, Vec<String>>,
    rates: BTreeMap<String, Vec<RateData>>,
}

#[derive(Clone)]
struct RateData {
    usage_class: String,
    unit_price: String,
    public_repeat_count: u8,
}

impl Enrichment {
    fn covers(&self, models: &[String]) -> bool {
        models
            .iter()
            .all(|model| self.capabilities.contains_key(model) && self.rates.contains_key(model))
    }

    fn offer_rates(&self, model: &str) -> Vec<OfferRate> {
        self.rates
            .get(model)
            .into_iter()
            .flatten()
            .flat_map(|rate| {
                (0..rate.public_repeat_count).map(|_| OfferRate {
                    usage_class: rate.usage_class.clone(),
                    unit: "token".to_owned(),
                    display_rate_nano_usd: rate.unit_price.clone(),
                    context_tier: None,
                    service_tier: None,
                    modality: None,
                    cache_ttl: None,
                })
            })
            .collect()
    }
}

async fn load_enrichment(
    database: &mut SqliteConnection,
    models: &[String],
) -> anyhow::Result<(Enrichment, u64)> {
    if models.is_empty() {
        return Ok((Enrichment::default(), 0));
    }
    let mut statement_count = 0_u64;
    let mut metadata_query = QueryBuilder::<Sqlite>::new(
        "SELECT model_name, capability FROM model_metadata_records WHERE model_name IN (",
    );
    let mut separated = metadata_query.separated(",");
    for model in models {
        separated.push_bind(model);
    }
    separated.push_unseparated(") ORDER BY model_name, id");
    let metadata_rows = metadata_query.build().fetch_all(&mut *database).await?;
    statement_count = statement_count.saturating_add(1);

    let mut rate_query = QueryBuilder::<Sqlite>::new(
        "SELECT model_name, usage_class, unit_price, public_repeat_count FROM billing_rate_records WHERE model_name IN (",
    );
    let mut separated = rate_query.separated(",");
    for model in models {
        separated.push_bind(model);
    }
    separated.push_unseparated(") ORDER BY model_name, id");
    let rate_rows = rate_query.build().fetch_all(&mut *database).await?;
    statement_count = statement_count.saturating_add(1);

    let mut enrichment = Enrichment::default();
    for row in metadata_rows {
        enrichment
            .capabilities
            .entry(row.try_get("model_name")?)
            .or_default()
            .push(row.try_get("capability")?);
    }
    for row in rate_rows {
        let repeat = u8::try_from(row.try_get::<i64, _>("public_repeat_count")?)
            .context("public repeat count overflow")?;
        enrichment
            .rates
            .entry(row.try_get("model_name")?)
            .or_default()
            .push(RateData {
                usage_class: row.try_get("usage_class")?,
                unit_price: row.try_get("unit_price")?,
                public_repeat_count: repeat,
            });
    }
    Ok((enrichment, statement_count))
}

fn marketplace_item(item: &super::MarketplaceRow, enrichment: &Enrichment) -> MarketplaceItem {
    let rates = enrichment.rates.get(&item.model);
    let range = |usage_class: &str| {
        let mut values = rates
            .into_iter()
            .flatten()
            .filter(|rate| rate.usage_class == usage_class && rate.public_repeat_count > 0)
            .filter_map(|rate| rate.unit_price.parse::<u64>().ok())
            .collect::<Vec<_>>();
        values.sort_unstable();
        Some(RateRange {
            min: values.first()?.to_string(),
            max: values.last()?.to_string(),
            unit: "token".to_owned(),
        })
    };
    MarketplaceItem {
        public_group_name: item.group.clone(),
        model: item.model.clone(),
        capabilities: enrichment
            .capabilities
            .get(&item.model)
            .cloned()
            .unwrap_or_default(),
        input_rate_range: range("input"),
        output_rate_range: range("output"),
        offer_count: item.offer_count,
    }
}

impl OperationAccumulator {
    fn record(&mut self, latency: u64, valid: bool, bytes: u64, statements: u64) {
        self.latencies.push(latency);
        if !valid {
            self.failed_samples = self.failed_samples.saturating_add(1);
        }
        self.statement_count = self.statement_count.saturating_add(statements);
        self.response_bytes = self.response_bytes.saturating_add(bytes);
        self.max_response_bytes = self.max_response_bytes.max(bytes);
    }

    fn finish(&mut self) -> anyhow::Result<OperationMetrics> {
        self.latencies.sort_unstable();
        Ok(OperationMetrics {
            samples: u64::try_from(self.latencies.len()).context("sample count overflow")?,
            failed_samples: self.failed_samples,
            statement_count: self.statement_count,
            response_bytes: self.response_bytes,
            max_response_bytes: self.max_response_bytes,
            p50_microseconds: percentile(&self.latencies, 50),
            p95_microseconds: percentile(&self.latencies, 95),
            p99_microseconds: percentile(&self.latencies, 99),
        })
    }
}

pub fn write_benchmark_report(
    root: &std::path::Path,
    output: impl AsRef<std::path::Path>,
    report: &BenchmarkReport,
) -> anyhow::Result<()> {
    let output = if output.as_ref().is_absolute() {
        output.as_ref().to_owned()
    } else {
        root.join(output)
    };
    let evidence = root.join("rehearsal/evidence");
    if output.parent() != Some(evidence.as_path()) {
        bail!("evidence output must be directly under rehearsal/evidence");
    }
    std::fs::create_dir_all(&evidence)?;
    std::fs::write(output, crate::provider::canonical_json(report)?)?;
    Ok(())
}

async fn configure_sqlite(database: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    for statement in [
        "PRAGMA journal_mode = WAL",
        "PRAGMA synchronous = NORMAL",
        "PRAGMA temp_store = MEMORY",
        "PRAGMA cache_size = -65536",
    ] {
        database.execute(statement).await?;
    }
    Ok(())
}

async fn create_sqlite_schema(database: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    for statement in [
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL UNIQUE, sort_order INTEGER NOT NULL)",
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, public_name TEXT NOT NULL, public_name_key BLOB NOT NULL, priority INTEGER NOT NULL, enabled INTEGER NOT NULL, channel_public_name TEXT NOT NULL, channel_public_name_key BLOB NOT NULL, channel_enabled INTEGER NOT NULL)",
        "CREATE TABLE monoize_provider_models (provider_id TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key BLOB NOT NULL, model_search_key BLOB NOT NULL, PRIMARY KEY(provider_id, model_name_key))",
        "CREATE TABLE billing_rate_records (id TEXT PRIMARY KEY, model_name TEXT NOT NULL, usage_class TEXT NOT NULL, unit_price TEXT NOT NULL, public_repeat_count INTEGER NOT NULL)",
        "CREATE TABLE model_metadata_records (id TEXT PRIMARY KEY, model_name TEXT NOT NULL, capability TEXT NOT NULL)",
        "CREATE INDEX idx_marketplace_provider_group ON monoize_providers(group_id, enabled, channel_enabled, priority, public_name_key)",
        "CREATE INDEX idx_marketplace_model_provider ON monoize_provider_models(provider_id, model_name_key)",
        "CREATE INDEX idx_marketplace_model_name ON monoize_provider_models(model_name_key, provider_id)",
        "CREATE INDEX idx_marketplace_rates_model ON billing_rate_records(model_name, id)",
        "CREATE INDEX idx_marketplace_metadata_model ON model_metadata_records(model_name, id)",
    ] {
        database.execute(statement).await?;
    }
    Ok(())
}

async fn load_sqlite_fixture(
    database: &mut SqliteConnection,
    manifest: &FixtureManifest,
) -> anyhow::Result<LoadedFixture> {
    let fixture = manifest.fixture();
    let mut transaction = database.begin().await?;
    for row in fixture.groups() {
        sqlx::query("INSERT INTO monoize_groups VALUES (?, ?, ?)")
            .bind(row.id)
            .bind(row.public_name)
            .bind(row.sort_order)
            .execute(&mut *transaction)
            .await?;
    }
    for row in fixture.providers() {
        let provider_key = row.public_name.as_bytes().to_vec();
        let channel_key = row.channel_public_name.as_bytes().to_vec();
        sqlx::query("INSERT INTO monoize_providers VALUES (?, ?, ?, ?, ?, 1, ?, ?, 1)")
            .bind(row.id)
            .bind(row.group_id)
            .bind(row.public_name)
            .bind(provider_key)
            .bind(row.priority)
            .bind(row.channel_public_name)
            .bind(channel_key)
            .execute(&mut *transaction)
            .await?;
    }
    for row in fixture.provider_models() {
        let model_key = row.model_name.as_bytes().to_vec();
        let search_key = row.model_name.to_ascii_lowercase().into_bytes();
        sqlx::query("INSERT INTO monoize_provider_models VALUES (?, ?, ?, ?)")
            .bind(row.provider_id)
            .bind(row.model_name)
            .bind(model_key)
            .bind(search_key)
            .execute(&mut *transaction)
            .await?;
    }
    for row in fixture.metadata() {
        sqlx::query("INSERT INTO model_metadata_records VALUES (?, ?, ?)")
            .bind(row.id)
            .bind(row.model_name)
            .bind(row.capability)
            .execute(&mut *transaction)
            .await?;
    }
    for row in fixture.rates() {
        sqlx::query("INSERT INTO billing_rate_records VALUES (?, ?, ?, ?, ?)")
            .bind(row.id)
            .bind(row.model_name)
            .bind(row.usage_class)
            .bind(row.unit_price)
            .bind(i64::from(row.public_repeat_count))
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    observe_loaded_fixture(database).await
}

async fn observe_loaded_fixture(database: &mut SqliteConnection) -> anyhow::Result<LoadedFixture> {
    use futures_util::TryStreamExt;
    use sha2::Digest;

    let groups = observed_count(database, "monoize_groups").await?;
    let providers = observed_count(database, "monoize_providers").await?;
    let provider_models = observed_count(database, "monoize_provider_models").await?;
    let rate_rows = observed_count(database, "billing_rate_records").await?;
    let metadata_rows = observed_count(database, "model_metadata_records").await?;
    let derived_row = sqlx::query(
        "SELECT COALESCE(SUM(br.public_repeat_count), 0) AS count FROM monoize_provider_models pm JOIN billing_rate_records br ON br.model_name = pm.model_name",
    )
    .fetch_one(&mut *database)
    .await?;
    let materialized_offer_rate_entries = u64::try_from(derived_row.try_get::<i64, _>("count")?)?;

    let mut digest = sha2::Sha256::new();
    for (kind, query) in [
        (
            "group",
            "SELECT json_array(id, public_name, sort_order) AS encoded FROM monoize_groups ORDER BY id",
        ),
        (
            "provider",
            "SELECT json_array(id, group_id, public_name, hex(public_name_key), priority, enabled, channel_public_name, hex(channel_public_name_key), channel_enabled) AS encoded FROM monoize_providers ORDER BY id",
        ),
        (
            "provider_model",
            "SELECT json_array(provider_id, model_name, hex(model_name_key), hex(model_search_key)) AS encoded FROM monoize_provider_models ORDER BY provider_id, model_name_key",
        ),
        (
            "metadata",
            "SELECT json_array(id, model_name, capability) AS encoded FROM model_metadata_records ORDER BY id",
        ),
        (
            "rate",
            "SELECT json_array(id, model_name, usage_class, unit_price, public_repeat_count) AS encoded FROM billing_rate_records ORDER BY id",
        ),
    ] {
        let mut rows = sqlx::query(query).fetch(&mut *database);
        while let Some(row) = rows.try_next().await? {
            let encoded = row.try_get::<String, _>("encoded")?;
            digest.update((kind.len() as u32).to_be_bytes());
            digest.update(kind.as_bytes());
            digest.update((encoded.len() as u32).to_be_bytes());
            digest.update(encoded.as_bytes());
        }
    }
    Ok(LoadedFixture {
        source_sha256: hex::encode(digest.finalize()),
        groups,
        providers,
        provider_models,
        rate_rows,
        metadata_rows,
        materialized_offer_rate_entries,
    })
}

async fn observed_count(database: &mut SqliteConnection, table: &str) -> anyhow::Result<u64> {
    let query = format!("SELECT COUNT(*) AS count FROM {table}");
    let row = sqlx::query(&query).fetch_one(database).await?;
    Ok(u64::try_from(row.try_get::<i64, _>("count")?)?)
}

fn validate_exact_list_page(page: &super::ListPage, expected: &ExpectedListPage) -> bool {
    page.next_key == expected.next_key
        && page.items.len() == expected.items.len()
        && page
            .items
            .iter()
            .zip(&expected.items)
            .all(|(actual, (model, offer_count))| {
                actual.group == "Group 000"
                    && actual.model == *model
                    && actual.offer_count == *offer_count
            })
}

fn validate_exact_offer_page(page: &super::OfferPage, expected: &ExpectedOfferPage) -> bool {
    page.next_key == expected.next_key
        && page.items.len() == expected.items.len()
        && page.items.iter().zip(&expected.items).all(|(actual, key)| {
            actual.priority == key.priority
                && actual.provider_public_name == key.provider_public_name
                && actual.channel_public_name == key.channel_public_name
        })
}

fn process_sample() -> anyhow::Result<(u64, u64)> {
    let pid = get_current_pid().map_err(|error| anyhow::anyhow!(error))?;
    let mut system = System::new();
    let pids = [pid];
    system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
    let process = system.process(pid).context("read current process")?;
    Ok((process.memory(), process.accumulated_cpu_time()))
}

fn qualification_blockers(mode: BenchmarkMode) -> Vec<String> {
    let mut blockers = vec![
        "insufficient_workers".to_owned(),
        "warmup_duration_not_met".to_owned(),
        "measurement_duration_not_met".to_owned(),
        "minimum_samples_not_met".to_owned(),
        "write_qualification_not_run".to_owned(),
        "postgres_qualification_not_run".to_owned(),
    ];
    if mode == BenchmarkMode::Smoke {
        blockers.insert(0, "smoke_mode".to_owned());
    }
    blockers
}

fn hash_json(value: &impl Serialize) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};

    let bytes = serde_json::to_vec(value).context("encode query set")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = sorted.len().saturating_mul(percentile).div_ceil(100);
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn micros(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn signed_delta(after: u64, before: u64) -> i64 {
    if after >= before {
        i64::try_from(after - before).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(before - after).unwrap_or(i64::MAX)
    }
}
