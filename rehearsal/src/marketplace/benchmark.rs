use super::{
    EndpointKind, Envelope, FixtureManifest, ListCursor, ListKey, MarketplaceQuery, OfferCursor,
    OfferKey, OfferQueryInput, QueryCase, QueryInput, QueryKind, canonical_filter_digest,
};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgConnectOptions;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Executor, PgConnection, QueryBuilder, Row, Sqlite, SqliteConnection};
use std::collections::BTreeMap;
use std::str::FromStr;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkComparisonReport {
    pub schema_version: u8,
    pub git_commit: String,
    pub envelope: Envelope,
    pub mode: BenchmarkMode,
    pub comparison_passed: bool,
    pub gate_b_qualified: bool,
    pub sqlite: BenchmarkReport,
    pub postgres: BenchmarkReport,
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
    items: Vec<MarketplaceItem>,
    next_key: Option<ListKey>,
}

struct ExpectedOfferPage {
    offers: Vec<ProviderOffer>,
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
    build_report(
        "sqlite",
        config,
        fixture_manifest,
        loaded_fixture,
        query_set_sha256,
        list_metrics,
        offer_metrics,
        elapsed,
        rss_before_bytes,
        rss_after_bytes,
        cpu_before_milliseconds,
        cpu_after_milliseconds,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    backend: &str,
    config: BenchmarkConfig,
    fixture_manifest: FixtureManifest,
    loaded_fixture: LoadedFixture,
    query_set_sha256: String,
    mut list_metrics: OperationAccumulator,
    mut offer_metrics: OperationAccumulator,
    elapsed: std::time::Duration,
    rss_before_bytes: u64,
    rss_after_bytes: u64,
    cpu_before_milliseconds: u64,
    cpu_after_milliseconds: u64,
) -> anyhow::Result<BenchmarkReport> {
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
        backend: backend.to_owned(),
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
        materialized_offer_rate_entries: loaded_fixture.materialized_offer_rate_entries,
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

pub async fn run_postgres_benchmark(
    url: &str,
    config: BenchmarkConfig,
) -> anyhow::Result<BenchmarkReport> {
    config.validate()?;
    let options = PgConnectOptions::from_str(url)
        .map_err(|_| anyhow::anyhow!("postgres_rehearsal_database_required"))?;
    if options.get_host() != "127.0.0.1" {
        bail!("postgres_rehearsal_host_required");
    }
    if options.get_database() != Some("lynshen_rehearsal") {
        bail!("postgres_rehearsal_database_required");
    }
    let fixture_manifest = FixtureManifest::generate(config.seed, config.envelope)?;
    let query_set = fixture_manifest.query_set();
    let query_set_sha256 = hash_json(&query_set)?;
    let mut database = PgConnection::connect_with(&options).await?;
    create_postgres_schema(&mut database).await?;
    let loaded_fixture = load_postgres_fixture(&mut database, &fixture_manifest).await?;
    database
        .execute(
            "ANALYZE lynshen_marketplace_benchmark.monoize_groups, lynshen_marketplace_benchmark.monoize_providers, lynshen_marketplace_benchmark.monoize_provider_models, lynshen_marketplace_benchmark.billing_rate_records, lynshen_marketplace_benchmark.model_metadata_records",
        )
        .await?;

    let cursor_key = [0x4c; 32];
    let prepared_queries = prepare_queries(&query_set, &fixture_manifest)?;
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
                execute_list_sample_postgres(
                    &mut database,
                    &query,
                    list_after,
                    expected_list.as_ref().context("expected list page")?,
                    &cursor_key,
                )
                .await?
            }
            QueryKind::Offers => {
                execute_offer_sample_postgres(
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
    build_report(
        "postgres",
        config,
        fixture_manifest,
        loaded_fixture,
        query_set_sha256,
        list_metrics,
        offer_metrics,
        elapsed,
        rss_before_bytes,
        rss_after_bytes,
        cpu_before_milliseconds,
        cpu_after_milliseconds,
    )
}

pub fn compare_benchmark_reports(
    sqlite: BenchmarkReport,
    postgres: BenchmarkReport,
) -> anyhow::Result<BenchmarkComparisonReport> {
    for (name, equal) in [
        (
            "backend",
            sqlite.backend == "sqlite" && postgres.backend == "postgres",
        ),
        ("git_commit", sqlite.git_commit == postgres.git_commit),
        ("envelope", sqlite.envelope == postgres.envelope),
        ("mode", sqlite.mode == postgres.mode),
        (
            "fixture_recipe_sha256",
            sqlite.fixture_recipe_sha256 == postgres.fixture_recipe_sha256,
        ),
        (
            "loaded_source_sha256",
            sqlite.loaded_source_sha256 == postgres.loaded_source_sha256,
        ),
        (
            "query_set_sha256",
            sqlite.query_set_sha256 == postgres.query_set_sha256,
        ),
        (
            "loaded_groups",
            sqlite.loaded_groups == postgres.loaded_groups,
        ),
        (
            "loaded_providers",
            sqlite.loaded_providers == postgres.loaded_providers,
        ),
        (
            "loaded_provider_models",
            sqlite.loaded_provider_models == postgres.loaded_provider_models,
        ),
        (
            "loaded_rate_rows",
            sqlite.loaded_rate_rows == postgres.loaded_rate_rows,
        ),
        (
            "loaded_metadata_rows",
            sqlite.loaded_metadata_rows == postgres.loaded_metadata_rows,
        ),
        (
            "materialized_offer_rate_entries",
            sqlite.materialized_offer_rate_entries == postgres.materialized_offer_rate_entries,
        ),
    ] {
        if !equal {
            bail!("benchmark_pair_mismatch:{name}");
        }
    }
    let gate_b_qualified = sqlite.gate_b_qualified && postgres.gate_b_qualified;
    Ok(BenchmarkComparisonReport {
        schema_version: 1,
        git_commit: sqlite.git_commit.clone(),
        envelope: sqlite.envelope,
        mode: sqlite.mode,
        comparison_passed: true,
        gate_b_qualified,
        sqlite,
        postgres,
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
            let expected_list = (query.kind == QueryKind::List).then(|| {
                expected_list_page(&offer_count_by_model, manifest, &query, list_after.clone())
            });
            let expected_offers = (query.kind == QueryKind::Offers)
                .then(|| expected_offer_page(manifest, &query, offer_after.clone()));
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
    manifest: &FixtureManifest,
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
        .map(|(model, offer_count)| expected_marketplace_item(manifest, model, offer_count))
        .collect::<Vec<_>>();
    ExpectedListPage {
        next_key: has_more.then(|| ListKey {
            group_ordinal: 0,
            model_name: items.last().expect("non-empty list page").model.clone(),
        }),
        items,
    }
}

fn expected_offer_page(
    manifest: &FixtureManifest,
    query: &QueryCase,
    after: Option<OfferKey>,
) -> ExpectedOfferPage {
    let start = after
        .as_ref()
        .and_then(|key| u64::try_from(key.priority).ok())
        .and_then(|value| value.checked_add(1))
        .unwrap_or(0);
    let end = start
        .saturating_add(u64::from(query.limit))
        .min(manifest.providers);
    let model = query.model.as_deref().expect("offers query model");
    let rates = expected_offer_rates(manifest, model);
    let offers = (start..end)
        .map(|index| ProviderOffer {
            public_provider_name: format!("Provider {index:05}"),
            public_channel_name: format!("Channel {index:05}"),
            api_type: "responses".to_owned(),
            rates: rates.clone(),
        })
        .collect::<Vec<_>>();
    let has_more = end < manifest.providers;
    ExpectedOfferPage {
        next_key: has_more.then(|| {
            let offer = offers.last().expect("non-empty offer page");
            OfferKey {
                priority: i32::try_from(end - 1).expect("provider index fits i32"),
                provider_public_name: offer.public_provider_name.clone(),
                channel_public_name: offer.public_channel_name.clone(),
            }
        }),
        offers,
    }
}

fn expected_marketplace_item(
    manifest: &FixtureManifest,
    model: String,
    offer_count: u64,
) -> MarketplaceItem {
    let rates = expected_rate_data(manifest, &model);
    let range = |usage_class: &str| {
        let values = rates
            .iter()
            .filter(|rate| rate.usage_class == usage_class && rate.public_repeat_count > 0)
            .map(|rate| {
                rate.unit_price
                    .parse::<u64>()
                    .expect("fixture price fits u64")
            })
            .collect::<Vec<_>>();
        Some(RateRange {
            min: values.iter().min()?.to_string(),
            max: values.iter().max()?.to_string(),
            unit: "token".to_owned(),
        })
    };
    MarketplaceItem {
        public_group_name: "Group 000".to_owned(),
        capabilities: expected_capabilities(manifest, &model),
        input_rate_range: range("input"),
        output_rate_range: range("output"),
        model,
        offer_count,
    }
}

fn expected_capabilities(manifest: &FixtureManifest, model: &str) -> Vec<String> {
    let model_index = fixture_model_index(model);
    (model_index..manifest.metadata_rows)
        .step_by(usize::try_from(manifest.distinct_models).expect("model count fits usize"))
        .map(|index| match index % 3 {
            0 => "text",
            1 => "vision",
            _ => "tools",
        })
        .map(str::to_owned)
        .collect()
}

fn expected_rate_data(manifest: &FixtureManifest, model: &str) -> Vec<RateData> {
    let rates_per_model = manifest.rate_rows / manifest.distinct_models;
    let _ = fixture_model_index(model);
    (0..rates_per_model)
        .map(|rate_index| RateData {
            usage_class: if rate_index.is_multiple_of(2) {
                "input".to_owned()
            } else {
                "output".to_owned()
            },
            unit_price: (rate_index + 1).to_string(),
            public_repeat_count: match manifest.envelope {
                Envelope::Smoke => 2,
                Envelope::Qualification if rate_index < 8 => 1,
                Envelope::Qualification => 0,
            },
        })
        .collect()
}

fn expected_offer_rates(manifest: &FixtureManifest, model: &str) -> Vec<OfferRate> {
    expected_rate_data(manifest, model)
        .into_iter()
        .flat_map(|rate| {
            (0..rate.public_repeat_count).map(move |_| OfferRate {
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

fn fixture_model_index(model: &str) -> u64 {
    if model == "hot-model" {
        0
    } else if let Some(index) = model.strip_prefix("fifty-model-") {
        index.parse::<u64>().expect("fixture model suffix") + 1
    } else {
        model
            .strip_prefix("model-")
            .expect("fixture model prefix")
            .parse::<u64>()
            .expect("fixture model suffix")
            + 1
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
    let after = decode_list_after(after, query, cursor_key)?;
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
    let (next_cursor, cursor_valid) = list_next_cursor(&page, query, cursor_key)?;
    let items = page
        .items
        .iter()
        .map(|item| marketplace_item(item, &enrichment))
        .collect::<Vec<_>>();
    let exact_response = validate_exact_list_page(&page, &items, expected);
    let bytes = encode_public(&MarketplaceListResponse {
        generated_at: "2026-08-26T00:00:00.000000Z".to_owned(),
        revision: "1".to_owned(),
        next_cursor,
        items,
    })
    .map_err(|_| anyhow::anyhow!("encode list response"))?;
    let valid = exact_response && cursor_valid && bytes.len() <= 1_048_576;
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
    let after = decode_offer_after(after, query, &group, &model, cursor_key)?;
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
    let (next_cursor, cursor_valid) = offer_next_cursor(&page, query, &group, &model, cursor_key)?;
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
    let exact_response = validate_exact_offer_page(&page, &offers, expected);
    let bytes = encode_public(&OfferResponse {
        generated_at: "2026-08-26T00:00:00.000000Z".to_owned(),
        revision: "1".to_owned(),
        public_group_name: group,
        model: model.clone(),
        next_cursor,
        offers,
    })
    .map_err(|_| anyhow::anyhow!("encode offers response"))?;
    let valid = exact_response && cursor_valid && bytes.len() <= 1_048_576;
    Ok((valid, bytes.len() as u64, statements))
}

async fn execute_list_sample_postgres(
    database: &mut PgConnection,
    query: &QueryCase,
    after: Option<ListKey>,
    expected: &ExpectedListPage,
    cursor_key: &[u8; 32],
) -> anyhow::Result<(bool, u64, u64)> {
    let after = decode_list_after(after, query, cursor_key)?;
    let page = MarketplaceQuery::list_postgres(
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
    let (enrichment, enrichment_statements) =
        load_enrichment_postgres(database, &model_names).await?;
    statements = statements.saturating_add(enrichment_statements);
    let (next_cursor, cursor_valid) = list_next_cursor(&page, query, cursor_key)?;
    let items = page
        .items
        .iter()
        .map(|item| marketplace_item(item, &enrichment))
        .collect::<Vec<_>>();
    let exact_response = validate_exact_list_page(&page, &items, expected);
    let bytes = encode_public(&MarketplaceListResponse {
        generated_at: "2026-08-26T00:00:00.000000Z".to_owned(),
        revision: "1".to_owned(),
        next_cursor,
        items,
    })
    .map_err(|_| anyhow::anyhow!("encode list response"))?;
    let valid = exact_response && cursor_valid && bytes.len() <= 1_048_576;
    Ok((valid, bytes.len() as u64, statements))
}

async fn execute_offer_sample_postgres(
    database: &mut PgConnection,
    query: &QueryCase,
    after: Option<OfferKey>,
    expected: &ExpectedOfferPage,
    cursor_key: &[u8; 32],
) -> anyhow::Result<(bool, u64, u64)> {
    let group = query.group.clone().context("offers query group")?;
    let model = query.model.clone().context("offers query model")?;
    let after = decode_offer_after(after, query, &group, &model, cursor_key)?;
    let page = MarketplaceQuery::offers_postgres(
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
        load_enrichment_postgres(database, std::slice::from_ref(&model)).await?;
    statements = statements.saturating_add(enrichment_statements);
    let (next_cursor, cursor_valid) = offer_next_cursor(&page, query, &group, &model, cursor_key)?;
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
    let exact_response = validate_exact_offer_page(&page, &offers, expected);
    let bytes = encode_public(&OfferResponse {
        generated_at: "2026-08-26T00:00:00.000000Z".to_owned(),
        revision: "1".to_owned(),
        public_group_name: group,
        model: model.clone(),
        next_cursor,
        offers,
    })
    .map_err(|_| anyhow::anyhow!("encode offers response"))?;
    let valid = exact_response && cursor_valid && bytes.len() <= 1_048_576;
    Ok((valid, bytes.len() as u64, statements))
}

fn list_next_cursor(
    page: &super::ListPage,
    query: &QueryCase,
    cursor_key: &[u8; 32],
) -> anyhow::Result<(Option<String>, bool)> {
    let Some(key) = page.next_key.as_ref() else {
        return Ok((None, true));
    };
    let digest = list_filter_digest(query);
    let ordinal = u64::try_from(key.group_ordinal).map_err(anyhow::Error::msg)?;
    let encoded = ListCursor::new(1, query.limit, digest, ordinal, &key.model_name)
        .and_then(|cursor| cursor.encode(cursor_key))
        .map_err(|_| anyhow::anyhow!("encode list cursor"))?;
    let decoded = ListCursor::decode(&encoded, cursor_key, 1, query.limit, digest)
        .map_err(|_| anyhow::anyhow!("decode list cursor"))?;
    let valid = decoded.group_ordinal == ordinal && decoded.model == key.model_name;
    Ok((Some(encoded), valid))
}

fn offer_next_cursor(
    page: &super::OfferPage,
    query: &QueryCase,
    group: &str,
    model: &str,
    cursor_key: &[u8; 32],
) -> anyhow::Result<(Option<String>, bool)> {
    let Some(key) = page.next_key.as_ref() else {
        return Ok((None, true));
    };
    let digest = canonical_filter_digest(EndpointKind::Offers, &[(1, group), (2, model)]);
    let encoded = OfferCursor::new(
        1,
        query.limit,
        digest,
        key.priority,
        &key.provider_public_name,
        &key.channel_public_name,
    )
    .and_then(|cursor| cursor.encode(cursor_key))
    .map_err(|_| anyhow::anyhow!("encode offer cursor"))?;
    let decoded = OfferCursor::decode(&encoded, cursor_key, 1, query.limit, digest)
        .map_err(|_| anyhow::anyhow!("decode offer cursor"))?;
    let valid = decoded.provider_priority == key.priority
        && decoded.provider_public_name == key.provider_public_name
        && decoded.channel_public_name == key.channel_public_name;
    Ok((Some(encoded), valid))
}

fn decode_list_after(
    after: Option<ListKey>,
    query: &QueryCase,
    cursor_key: &[u8; 32],
) -> anyhow::Result<Option<ListKey>> {
    let Some(after) = after else {
        return Ok(None);
    };
    let digest = list_filter_digest(query);
    let ordinal = u64::try_from(after.group_ordinal).map_err(anyhow::Error::msg)?;
    let encoded = ListCursor::new(1, query.limit, digest, ordinal, &after.model_name)
        .and_then(|cursor| cursor.encode(cursor_key))
        .map_err(|_| anyhow::anyhow!("encode list input cursor"))?;
    let decoded = ListCursor::decode(&encoded, cursor_key, 1, query.limit, digest)
        .map_err(|_| anyhow::anyhow!("decode list input cursor"))?;
    Ok(Some(ListKey {
        group_ordinal: i64::try_from(decoded.group_ordinal).context("list cursor ordinal")?,
        model_name: decoded.model,
    }))
}

fn decode_offer_after(
    after: Option<OfferKey>,
    query: &QueryCase,
    group: &str,
    model: &str,
    cursor_key: &[u8; 32],
) -> anyhow::Result<Option<OfferKey>> {
    let Some(after) = after else {
        return Ok(None);
    };
    let digest = canonical_filter_digest(EndpointKind::Offers, &[(1, group), (2, model)]);
    let encoded = OfferCursor::new(
        1,
        query.limit,
        digest,
        after.priority,
        &after.provider_public_name,
        &after.channel_public_name,
    )
    .and_then(|cursor| cursor.encode(cursor_key))
    .map_err(|_| anyhow::anyhow!("encode offer input cursor"))?;
    let decoded = OfferCursor::decode(&encoded, cursor_key, 1, query.limit, digest)
        .map_err(|_| anyhow::anyhow!("decode offer input cursor"))?;
    Ok(Some(OfferKey {
        priority: decoded.provider_priority,
        provider_public_name: decoded.provider_public_name,
        channel_public_name: decoded.channel_public_name,
    }))
}

fn list_filter_digest(query: &QueryCase) -> [u8; 32] {
    let q = query.query.as_deref().unwrap_or_default();
    let group = query.group.as_deref().unwrap_or_default();
    canonical_filter_digest(EndpointKind::List, &[(1, q), (2, group)])
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

async fn load_enrichment_postgres(
    database: &mut PgConnection,
    models: &[String],
) -> anyhow::Result<(Enrichment, u64)> {
    if models.is_empty() {
        return Ok((Enrichment::default(), 0));
    }
    let mut statement_count = 0_u64;
    let mut metadata_query = QueryBuilder::<sqlx::Postgres>::new(
        "SELECT model_name, capability FROM model_metadata_records WHERE model_name IN (",
    );
    let mut separated = metadata_query.separated(",");
    for model in models {
        separated.push_bind(model);
    }
    separated.push_unseparated(") ORDER BY model_name, id");
    let metadata_rows = metadata_query.build().fetch_all(&mut *database).await?;
    statement_count = statement_count.saturating_add(1);

    let mut rate_query = QueryBuilder::<sqlx::Postgres>::new(
        "SELECT model_name, usage_class, unit_price, public_repeat_count::BIGINT AS public_repeat_count FROM billing_rate_records WHERE model_name IN (",
    );
    let mut separated = rate_query.separated(",");
    for model in models {
        separated.push_bind(model);
    }
    separated.push_unseparated(") ORDER BY model_name, id");
    let rate_rows = rate_query.build().fetch_all(&mut *database).await?;
    statement_count = statement_count.saturating_add(1);
    enrichment_from_rows(metadata_rows, rate_rows).map(|value| (value, statement_count))
}

fn enrichment_from_rows<MetadataRow, RateRow>(
    metadata_rows: Vec<MetadataRow>,
    rate_rows: Vec<RateRow>,
) -> anyhow::Result<Enrichment>
where
    MetadataRow: Row,
    RateRow: Row<Database = MetadataRow::Database>,
    for<'a> &'a str: sqlx::ColumnIndex<MetadataRow> + sqlx::ColumnIndex<RateRow>,
    String: for<'r> sqlx::Decode<'r, MetadataRow::Database> + sqlx::Type<MetadataRow::Database>,
    i64: for<'r> sqlx::Decode<'r, MetadataRow::Database> + sqlx::Type<MetadataRow::Database>,
{
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
    Ok(enrichment)
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
    write_evidence(root, output, report)
}

pub fn write_benchmark_comparison_report(
    root: &std::path::Path,
    output: impl AsRef<std::path::Path>,
    report: &BenchmarkComparisonReport,
) -> anyhow::Result<()> {
    write_evidence(root, output, report)
}

fn write_evidence(
    root: &std::path::Path,
    output: impl AsRef<std::path::Path>,
    report: &impl Serialize,
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

async fn create_postgres_schema(database: &mut PgConnection) -> Result<(), sqlx::Error> {
    let mut transaction = database.begin().await?;
    transaction
        .execute(
            "DROP TABLE IF EXISTS lynshen_marketplace_benchmark.model_metadata_records, lynshen_marketplace_benchmark.billing_rate_records, lynshen_marketplace_benchmark.monoize_provider_models, lynshen_marketplace_benchmark.monoize_providers, lynshen_marketplace_benchmark.monoize_groups RESTRICT",
        )
        .await?;
    transaction
        .execute("DROP SCHEMA IF EXISTS lynshen_marketplace_benchmark RESTRICT")
        .await?;
    transaction
        .execute("CREATE SCHEMA lynshen_marketplace_benchmark")
        .await?;
    transaction
        .execute("SET LOCAL search_path TO lynshen_marketplace_benchmark")
        .await?;
    for statement in [
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL UNIQUE, sort_order BIGINT NOT NULL)",
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, public_name TEXT NOT NULL, public_name_key BYTEA NOT NULL, priority INTEGER NOT NULL, enabled INTEGER NOT NULL, channel_public_name TEXT NOT NULL, channel_public_name_key BYTEA NOT NULL, channel_enabled INTEGER NOT NULL)",
        "CREATE TABLE monoize_provider_models (provider_id TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key BYTEA NOT NULL, model_search_key BYTEA NOT NULL, PRIMARY KEY(provider_id, model_name_key))",
        "CREATE TABLE billing_rate_records (id TEXT PRIMARY KEY, model_name TEXT NOT NULL, usage_class TEXT NOT NULL, unit_price TEXT NOT NULL, public_repeat_count INTEGER NOT NULL)",
        "CREATE TABLE model_metadata_records (id TEXT PRIMARY KEY, model_name TEXT NOT NULL, capability TEXT NOT NULL)",
        "CREATE INDEX idx_marketplace_provider_group ON monoize_providers(group_id, enabled, channel_enabled, priority, public_name_key)",
        "CREATE INDEX idx_marketplace_model_provider ON monoize_provider_models(provider_id, model_name_key)",
        "CREATE INDEX idx_marketplace_model_name ON monoize_provider_models(model_name_key, provider_id)",
        "CREATE INDEX idx_marketplace_rates_model ON billing_rate_records(model_name, id)",
        "CREATE INDEX idx_marketplace_metadata_model ON model_metadata_records(model_name, id)",
    ] {
        transaction.execute(statement).await?;
    }
    transaction.commit().await?;
    database
        .execute("SET search_path TO lynshen_marketplace_benchmark")
        .await?;
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

async fn load_postgres_fixture(
    database: &mut PgConnection,
    manifest: &FixtureManifest,
) -> anyhow::Result<LoadedFixture> {
    let fixture = manifest.fixture();
    let mut transaction = database.begin().await?;
    for row in fixture.groups() {
        sqlx::query("INSERT INTO monoize_groups VALUES ($1, $2, $3)")
            .bind(row.id)
            .bind(row.public_name)
            .bind(row.sort_order)
            .execute(&mut *transaction)
            .await?;
    }
    for row in fixture.providers() {
        let provider_key = row.public_name.as_bytes().to_vec();
        let channel_key = row.channel_public_name.as_bytes().to_vec();
        sqlx::query("INSERT INTO monoize_providers VALUES ($1, $2, $3, $4, $5, 1, $6, $7, 1)")
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
        sqlx::query("INSERT INTO monoize_provider_models VALUES ($1, $2, $3, $4)")
            .bind(row.provider_id)
            .bind(row.model_name)
            .bind(model_key)
            .bind(search_key)
            .execute(&mut *transaction)
            .await?;
    }
    for row in fixture.metadata() {
        sqlx::query("INSERT INTO model_metadata_records VALUES ($1, $2, $3)")
            .bind(row.id)
            .bind(row.model_name)
            .bind(row.capability)
            .execute(&mut *transaction)
            .await?;
    }
    for row in fixture.rates() {
        sqlx::query("INSERT INTO billing_rate_records VALUES ($1, $2, $3, $4, $5)")
            .bind(row.id)
            .bind(row.model_name)
            .bind(row.usage_class)
            .bind(row.unit_price)
            .bind(i32::from(row.public_repeat_count))
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    observe_loaded_fixture_postgres(database).await
}

async fn observe_loaded_fixture_postgres(
    database: &mut PgConnection,
) -> anyhow::Result<LoadedFixture> {
    use futures_util::TryStreamExt;
    use sha2::Digest;

    let groups = observed_count_postgres(database, "monoize_groups").await?;
    let providers = observed_count_postgres(database, "monoize_providers").await?;
    let provider_models = observed_count_postgres(database, "monoize_provider_models").await?;
    let rate_rows = observed_count_postgres(database, "billing_rate_records").await?;
    let metadata_rows = observed_count_postgres(database, "model_metadata_records").await?;
    let derived_row = sqlx::query(
        "SELECT COALESCE(SUM(br.public_repeat_count), 0)::BIGINT AS count FROM monoize_provider_models pm JOIN billing_rate_records br ON br.model_name = pm.model_name",
    )
    .fetch_one(&mut *database)
    .await?;
    let materialized_offer_rate_entries = u64::try_from(derived_row.try_get::<i64, _>("count")?)?;

    let mut digest = sha2::Sha256::new();
    {
        let mut rows = sqlx::query(
            "SELECT id, public_name, sort_order FROM monoize_groups ORDER BY id COLLATE \"C\"",
        )
        .fetch(&mut *database);
        while let Some(row) = rows.try_next().await? {
            update_source_digest(
                &mut digest,
                "group",
                &(
                    row.try_get::<String, _>("id")?,
                    row.try_get::<String, _>("public_name")?,
                    row.try_get::<i64, _>("sort_order")?,
                ),
            )?;
        }
    }
    {
        let mut rows = sqlx::query(
            "SELECT id, group_id, public_name, public_name_key, priority, enabled, channel_public_name, channel_public_name_key, channel_enabled FROM monoize_providers ORDER BY id COLLATE \"C\"",
        )
        .fetch(&mut *database);
        while let Some(row) = rows.try_next().await? {
            update_source_digest(
                &mut digest,
                "provider",
                &(
                    row.try_get::<String, _>("id")?,
                    row.try_get::<String, _>("group_id")?,
                    row.try_get::<String, _>("public_name")?,
                    hex::encode_upper(row.try_get::<Vec<u8>, _>("public_name_key")?),
                    row.try_get::<i32, _>("priority")?,
                    row.try_get::<i32, _>("enabled")?,
                    row.try_get::<String, _>("channel_public_name")?,
                    hex::encode_upper(row.try_get::<Vec<u8>, _>("channel_public_name_key")?),
                    row.try_get::<i32, _>("channel_enabled")?,
                ),
            )?;
        }
    }
    {
        let mut rows = sqlx::query(
            "SELECT provider_id, model_name, model_name_key, model_search_key FROM monoize_provider_models ORDER BY provider_id COLLATE \"C\", model_name_key",
        )
        .fetch(&mut *database);
        while let Some(row) = rows.try_next().await? {
            update_source_digest(
                &mut digest,
                "provider_model",
                &(
                    row.try_get::<String, _>("provider_id")?,
                    row.try_get::<String, _>("model_name")?,
                    hex::encode_upper(row.try_get::<Vec<u8>, _>("model_name_key")?),
                    hex::encode_upper(row.try_get::<Vec<u8>, _>("model_search_key")?),
                ),
            )?;
        }
    }
    {
        let mut rows = sqlx::query(
            "SELECT id, model_name, capability FROM model_metadata_records ORDER BY id COLLATE \"C\"",
        )
        .fetch(&mut *database);
        while let Some(row) = rows.try_next().await? {
            update_source_digest(
                &mut digest,
                "metadata",
                &(
                    row.try_get::<String, _>("id")?,
                    row.try_get::<String, _>("model_name")?,
                    row.try_get::<String, _>("capability")?,
                ),
            )?;
        }
    }
    {
        let mut rows = sqlx::query(
            "SELECT id, model_name, usage_class, unit_price, public_repeat_count FROM billing_rate_records ORDER BY id COLLATE \"C\"",
        )
        .fetch(&mut *database);
        while let Some(row) = rows.try_next().await? {
            update_source_digest(
                &mut digest,
                "rate",
                &(
                    row.try_get::<String, _>("id")?,
                    row.try_get::<String, _>("model_name")?,
                    row.try_get::<String, _>("usage_class")?,
                    row.try_get::<String, _>("unit_price")?,
                    row.try_get::<i32, _>("public_repeat_count")?,
                ),
            )?;
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

fn update_source_digest(
    digest: &mut sha2::Sha256,
    kind: &str,
    value: &impl Serialize,
) -> anyhow::Result<()> {
    use sha2::Digest;

    let encoded = serde_json::to_vec(value).context("encode observed source row")?;
    digest.update(u32::try_from(kind.len())?.to_be_bytes());
    digest.update(kind.as_bytes());
    digest.update(u32::try_from(encoded.len())?.to_be_bytes());
    digest.update(encoded);
    Ok(())
}

async fn observed_count_postgres(database: &mut PgConnection, table: &str) -> anyhow::Result<u64> {
    let query = format!("SELECT COUNT(*)::BIGINT AS count FROM {table}");
    let row = sqlx::query(&query).fetch_one(database).await?;
    Ok(u64::try_from(row.try_get::<i64, _>("count")?)?)
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

fn validate_exact_list_page(
    page: &super::ListPage,
    items: &[MarketplaceItem],
    expected: &ExpectedListPage,
) -> bool {
    page.next_key == expected.next_key && items == expected.items
}

fn validate_exact_offer_page(
    page: &super::OfferPage,
    offers: &[ProviderOffer],
    expected: &ExpectedOfferPage,
) -> bool {
    page.next_key == expected.next_key && offers == expected.offers
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
