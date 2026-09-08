use chrono::Utc;
use rust_decimal::Decimal;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbBackend, Statement, TransactionTrait, Value,
};
use sea_orm_migration::prelude::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
#[cfg(not(test))]
use std::fs;
use std::str::FromStr;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Deserialize)]
struct PricingProfilePattern {
    pattern: String,
    pricing_profile: String,
}

#[derive(Debug, Deserialize)]
struct ApiTypeOverride {
    pattern: String,
    api_type: String,
}

#[derive(Debug)]
struct LegacyModel {
    model_name: String,
    redirect: Option<String>,
    multiplier: String,
    resolved_profile: Option<String>,
    created_at: String,
}

#[derive(Debug)]
struct BillingRate {
    pricing_profile: String,
    model_pattern: Option<String>,
    provider_type: Option<String>,
    rate_kind: String,
    usage_class: String,
    unit_price_nano_usd: String,
    context_tier: Option<String>,
    service_tier: Option<String>,
    modality: Option<String>,
    cache_ttl: Option<String>,
    match_json: serde_json::Value,
}

struct LegacyPricingState {
    patterns: Vec<PricingProfilePattern>,
    reasoning_suffix_map: HashMap<String, String>,
    metadata_profiles: HashMap<String, String>,
    rates: Vec<BillingRate>,
}

#[derive(Debug, Deserialize)]
struct ApprovedPublicNameManifest {
    schema_version: u8,
    source_fingerprint: String,
    #[serde(default)]
    approved_semantic_change_source_provider_ids: Vec<String>,
    groups: Vec<ApprovedGroupPublicName>,
    targets: Vec<ApprovedTargetPublicNames>,
}

#[derive(Debug, Deserialize)]
struct ApprovedGroupPublicName {
    source_group_id: String,
    public_name: String,
    public_name_key_hex: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ApprovedTargetPublicNames {
    source_provider_id: String,
    source_channel_id: String,
    target_group_id: String,
    target_provider_id: String,
    target_channel_id: String,
    provider_public_name: String,
    provider_public_name_key_hex: String,
    channel_public_name: String,
    channel_public_name_key_hex: String,
}

#[derive(Clone)]
struct ApprovedName {
    value: String,
    key: Vec<u8>,
}

struct MigrationApproval {
    groups: BTreeMap<String, ApprovedName>,
    targets: BTreeMap<(String, String, String), ApprovedTargetPublicNames>,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let tx = manager.get_connection().begin().await?;
        if let Err(error) = migrate_up(&tx, backend).await {
            let _ = tx.rollback().await;
            return Err(error);
        }
        tx.commit().await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "provider pricing flatten is destructive; restore a database backup to roll back"
                .to_string(),
        ))
    }
}

async fn migrate_up(tx: &DatabaseTransaction, backend: DbBackend) -> Result<(), DbErr> {
    let already_flattened = if backend == DbBackend::Postgres {
        tx.query_one(Statement::from_string(
            backend,
            "SELECT 1 FROM information_schema.columns WHERE table_name = 'monoize_providers' AND column_name = 'channel_id' LIMIT 1".to_string(),
        ))
        .await?
        .is_some()
    } else {
        tx.query_one(Statement::from_string(
            backend,
            "SELECT 1 FROM pragma_table_info('monoize_providers') WHERE name = 'channel_id' LIMIT 1".to_string(),
        ))
        .await?
        .is_some()
    };
    if already_flattened {
        return Ok(());
    }

    let approval = load_and_validate_migration_approval(tx, backend).await?;

    let now = Utc::now().to_rfc3339();
    let suffix = if backend == DbBackend::Postgres {
        "BYTEA"
    } else {
        "BLOB"
    };
    migrate_group_public_names(tx, backend, suffix, &approval.groups).await?;
    let model_search_expr = ascii_fold_expression("model_name");
    let key_cast = if backend == DbBackend::Postgres {
        "convert_to(model_name, 'UTF8')"
    } else {
        "CAST(model_name AS BLOB)"
    };
    let model_search_check = if backend == DbBackend::Postgres {
        format!("convert_to({model_search_expr}, 'UTF8')")
    } else {
        format!("CAST({model_search_expr} AS BLOB)")
    };

    let provider_key_cast = key_cast.replace("model_name", "public_name");
    let channel_key_cast = key_cast.replace("model_name", "channel_public_name");
    let provider_multiplier_check = canonical_decimal_check("multiplier", backend);
    let model_multiplier_check = canonical_decimal_check("multiplier_override", backend);
    let statements = vec![
        "DROP INDEX IF EXISTS idx_mc_provider_id".to_string(),
        "DROP INDEX IF EXISTS idx_mcm_channel_id".to_string(),
        "DROP INDEX IF EXISTS uq_mcm_channel_id_model_name".to_string(),
        "DROP INDEX IF EXISTS idx_mcm_model_name_channel_id".to_string(),
        "ALTER TABLE monoize_providers RENAME TO monoize_providers_legacy_flatten".to_string(),
        "ALTER TABLE monoize_channels RENAME TO monoize_channels_legacy_flatten".to_string(),
        "ALTER TABLE monoize_channel_models RENAME TO monoize_channel_models_legacy_flatten".to_string(),
        format!("CREATE TABLE monoize_providers (id TEXT PRIMARY KEY NOT NULL, group_id TEXT NOT NULL REFERENCES monoize_groups(id) ON DELETE RESTRICT, name TEXT NOT NULL, public_name TEXT NOT NULL, public_name_key {suffix} NOT NULL CHECK(public_name_key = {provider_key_cast}), priority INTEGER NOT NULL, enabled INTEGER NOT NULL, pricing_profile TEXT CHECK(pricing_profile IS NULL OR length(trim(pricing_profile)) > 0), multiplier TEXT NOT NULL DEFAULT '1' CHECK({provider_multiplier_check}), configuration_generation BIGINT NOT NULL DEFAULT 1, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, channel_id TEXT NOT NULL UNIQUE, channel_name TEXT NOT NULL, channel_public_name TEXT NOT NULL, channel_public_name_key {suffix} NOT NULL CHECK(channel_public_name_key = {channel_key_cast}), channel_provider_type TEXT NOT NULL, channel_base_url TEXT NOT NULL, channel_api_key TEXT NOT NULL, channel_enabled INTEGER NOT NULL, channel_max_retries INTEGER NOT NULL DEFAULT 0, channel_passive_failure_count_threshold_override INTEGER, channel_passive_cooldown_seconds_override INTEGER, channel_passive_window_seconds_override INTEGER, channel_passive_rate_limit_cooldown_seconds_override INTEGER, channel_active_probe_enabled_override INTEGER, channel_active_probe_interval_seconds_override INTEGER, channel_active_probe_success_threshold_override INTEGER, channel_active_probe_model_override TEXT, channel_affinity_enabled_override INTEGER, channel_affinity_idle_ttl_seconds_override INTEGER, channel_affinity_failback_mode_override TEXT, channel_affinity_failback_delay_seconds_override INTEGER, channel_proxy_url TEXT, channel_extra_headers TEXT, channel_session_affinity_auto INTEGER, channel_allow_missing_usage INTEGER NOT NULL DEFAULT 0, transforms TEXT NOT NULL DEFAULT '[]', api_type_overrides TEXT NOT NULL DEFAULT '[]', active_probe_enabled_override INTEGER, active_probe_interval_seconds_override INTEGER, active_probe_success_threshold_override INTEGER, active_probe_model_override TEXT, request_timeout_ms_override INTEGER, extra_fields_whitelist TEXT, strip_cross_protocol_nested_extra INTEGER, circuit_breaker_enabled INTEGER NOT NULL DEFAULT 1, per_model_circuit_break INTEGER NOT NULL DEFAULT 0, channel_retry_interval_ms INTEGER NOT NULL DEFAULT 0)"),
        format!("CREATE TABLE monoize_provider_models (provider_id TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key {suffix} NOT NULL CHECK(model_name_key = {key_cast}), model_search_key {suffix} NOT NULL CHECK(model_search_key = {model_search_check}), redirect TEXT, pricing_profile_mode TEXT NOT NULL CHECK(pricing_profile_mode IN ('inherit','override','unpriced')), pricing_profile_override TEXT, multiplier_override TEXT CHECK(multiplier_override IS NULL OR ({model_multiplier_check})), created_at TEXT NOT NULL, PRIMARY KEY(provider_id, model_name_key), FOREIGN KEY(provider_id) REFERENCES monoize_providers(id) ON DELETE CASCADE, CHECK((pricing_profile_mode = 'override' AND pricing_profile_override IS NOT NULL AND length(trim(pricing_profile_override)) > 0) OR (pricing_profile_mode IN ('inherit','unpriced') AND pricing_profile_override IS NULL)))"),
        "CREATE INDEX idx_monoize_provider_route ON monoize_providers(group_id, priority, created_at, id)".to_string(),
        "CREATE INDEX idx_monoize_provider_models_lookup ON monoize_provider_models(model_name_key, provider_id)".to_string(),
        "CREATE UNIQUE INDEX uq_monoize_provider_public_name ON monoize_providers(group_id, public_name_key)".to_string(),
        "CREATE UNIQUE INDEX uq_monoize_channel_public_name ON monoize_providers(group_id, channel_public_name_key)".to_string(),
    ];
    for sql in statements {
        tx.execute(Statement::from_string(backend, sql.to_string()))
            .await?;
    }

    let pricing_state = load_legacy_pricing_state(tx, backend).await?;

    let providers = tx.query_all(Statement::from_string(
        backend,
        "SELECT id, name, max_retries, channel_max_retries, channel_retry_interval_ms, circuit_breaker_enabled, per_model_circuit_break, transforms, api_type_overrides, active_probe_enabled_override, active_probe_interval_seconds_override, active_probe_success_threshold_override, active_probe_model_override, request_timeout_ms_override, extra_fields_whitelist, strip_cross_protocol_nested_extra, group_ids, enabled, priority, created_at, updated_at FROM monoize_providers_legacy_flatten ORDER BY priority, created_at, id".to_string(),
    )).await?;
    let mut next_priority = HashMap::<String, i32>::new();
    for provider in providers {
        let old_id: String = provider.try_get("", "id")?;
        let groups = decode_groups(
            provider
                .try_get::<Option<String>>("", "group_ids")?
                .as_deref(),
            &old_id,
        )?;
        let channels = tx.query_all(Statement::from_sql_and_values(
            backend,
            numbered(backend, "SELECT id, name, provider_type, base_url, api_key, weight, enabled, created_at, passive_failure_count_threshold_override, passive_cooldown_seconds_override, passive_window_seconds_override, passive_rate_limit_cooldown_seconds_override, active_probe_enabled_override, active_probe_interval_seconds_override, active_probe_success_threshold_override, active_probe_model_override, affinity_enabled_override, affinity_idle_ttl_seconds_override, affinity_failback_mode_override, affinity_failback_delay_seconds_override, proxy_url, extra_headers, session_affinity_auto, allow_missing_usage FROM monoize_channels_legacy_flatten WHERE provider_id = ? ORDER BY created_at, id"), vec![old_id.clone().into()],
        )).await?;
        if channels.is_empty() {
            return Err(DbErr::Custom(format!("provider {old_id} has no channel")));
        }
        let api_type_overrides = serde_json::from_str::<Vec<ApiTypeOverride>>(
            &provider
                .try_get::<String>("", "api_type_overrides")
                .map_err(|error| {
                    DbErr::Custom(format!(
                        "provider {old_id} missing api_type_overrides: {error}"
                    ))
                })?,
        )
        .map_err(|error| {
            DbErr::Custom(format!(
                "provider {old_id} has invalid api_type_overrides: {error}"
            ))
        })?;
        let mut channel_models = HashMap::<String, Vec<LegacyModel>>::new();
        for channel in &channels {
            let legacy_channel_id: String = channel.try_get("", "id")?;
            let provider_type: String = channel.try_get("", "provider_type")?;
            channel_models.insert(
                legacy_channel_id.clone(),
                load_legacy_models(
                    tx,
                    backend,
                    &legacy_channel_id,
                    &provider_type,
                    &api_type_overrides,
                    &pricing_state,
                    &now,
                )
                .await?,
            );
        }
        for (group_index, group_id) in groups.iter().enumerate() {
            let group_name = group_name(tx, backend, group_id).await?;
            for (channel_index, channel) in channels.iter().enumerate() {
                let first_pair = group_index == 0 && channel_index == 0;
                let legacy_channel_id: String = channel.try_get("", "id")?;
                let provider_id = if first_pair {
                    old_id.clone()
                } else {
                    deterministic_id("provider", &old_id, group_id, &legacy_channel_id)
                };
                let channel_id = if group_index == 0 {
                    legacy_channel_id.clone()
                } else {
                    deterministic_id("channel", &old_id, group_id, &legacy_channel_id)
                };
                let name: String = provider.try_get("", "name")?;
                let channel_name: String = channel.try_get("", "name")?;
                let models = channel_models.get(&legacy_channel_id).ok_or_else(|| {
                    DbErr::Custom(format!(
                        "channel {legacy_channel_id} model snapshot missing"
                    ))
                })?;
                let pricing_profile = infer_default_profile(models);
                let multiplier = infer_default_multiplier(models)?;
                let approved_names = approval
                    .targets
                    .get(&(old_id.clone(), group_id.clone(), legacy_channel_id.clone()))
                    .ok_or_else(|| {
                        DbErr::Custom(format!(
                            "approved migration manifest target missing for provider {old_id}"
                        ))
                    })?;
                let target_name = if first_pair {
                    name.clone()
                } else {
                    format!("{name} / {group_name} / {channel_name}")
                };
                let priority = next_priority.entry(group_id.clone()).or_default();
                let target_priority = *priority;
                *priority = priority
                    .checked_add(1)
                    .ok_or_else(|| DbErr::Custom("group priority overflow".to_string()))?;
                insert_provider(
                    tx,
                    backend,
                    &provider,
                    channel,
                    ProviderInsert {
                        group_id,
                        provider_id: &provider_id,
                        channel_id: &channel_id,
                        name: &target_name,
                        channel_name: &channel_name,
                        public_name: &approved_names.provider_public_name,
                        channel_public_name: &approved_names.channel_public_name,
                        priority: target_priority,
                        pricing_profile: pricing_profile.as_deref(),
                        multiplier: &multiplier,
                        now: &now,
                    },
                )
                .await?;
                for model in models {
                    let (pricing_profile_mode, pricing_profile_override) =
                        match (&model.resolved_profile, &pricing_profile) {
                            (None, _) => ("unpriced", None),
                            (Some(profile), Some(default)) if profile == default => {
                                ("inherit", None)
                            }
                            (Some(profile), _) => ("override", Some(profile.clone())),
                        };
                    let multiplier_override =
                        (model.multiplier != multiplier).then(|| model.multiplier.clone());
                    tx.execute(Statement::from_sql_and_values(backend, numbered(backend, "INSERT INTO monoize_provider_models (provider_id, model_name, model_name_key, model_search_key, redirect, pricing_profile_mode, pricing_profile_override, multiplier_override, created_at) VALUES (?,?,?,?,?,?,?,?,?)"), vec![provider_id.clone().into(), model.model_name.clone().into(), bytes(backend, model.model_name.as_bytes()), bytes(backend, ascii_fold_bytes(model.model_name.as_bytes())), model.redirect.clone().into(), pricing_profile_mode.into(), pricing_profile_override.into(), multiplier_override.into(), model.created_at.clone().into()])).await?;
                }
            }
        }
    }
    for table in [
        "monoize_channel_models_legacy_flatten",
        "monoize_channels_legacy_flatten",
        "monoize_providers_legacy_flatten",
    ] {
        tx.execute(Statement::from_string(
            backend,
            format!("DROP TABLE {table}"),
        ))
        .await?;
    }
    Ok(())
}

async fn load_and_validate_migration_approval(
    tx: &DatabaseTransaction,
    backend: DbBackend,
) -> Result<MigrationApproval, DbErr> {
    let source_groups = tx
        .query_all(Statement::from_string(
            backend,
            "SELECT id, name, is_default FROM monoize_groups ORDER BY id".to_string(),
        ))
        .await?;
    let source_providers = tx
        .query_all(Statement::from_string(
            backend,
            "SELECT id, group_ids FROM monoize_providers ORDER BY id".to_string(),
        ))
        .await?;

    if source_providers.is_empty()
        && source_groups.len() == 1
        && source_groups[0].try_get::<String>("", "name")? == "default"
        && source_groups[0].try_get::<i32>("", "is_default")? == 1
    {
        let id = source_groups[0].try_get::<String>("", "id")?;
        return Ok(MigrationApproval {
            groups: BTreeMap::from([(
                id,
                ApprovedName {
                    value: "Default".to_string(),
                    key: b"Default".to_vec(),
                },
            )]),
            targets: BTreeMap::new(),
        });
    }

    let manifest_row = tx
        .query_one(Statement::from_sql_and_values(
            backend,
            numbered(
                backend,
                "SELECT value FROM state_records WHERE tenant_id = ? AND kind = ? AND id = ?",
            ),
            vec![
                "system".into(),
                "provider_pricing_migration".into(),
                "approved_manifest_v1".into(),
            ],
        ))
        .await
        .map_err(|_| DbErr::Custom("approved migration manifest is unavailable".to_string()))?
        .ok_or_else(|| DbErr::Custom("approved migration manifest is missing".to_string()))?;
    let manifest_text: String = manifest_row.try_get("", "value")?;
    let manifest: ApprovedPublicNameManifest =
        serde_json::from_str(&manifest_text).map_err(|error| {
            DbErr::Custom(format!("approved migration manifest is invalid: {error}"))
        })?;
    if manifest.schema_version != 1 {
        return Err(DbErr::Custom(
            "approved migration manifest schema version must be 1".to_string(),
        ));
    }
    if !is_lower_hex(&manifest.source_fingerprint, 64) {
        return Err(DbErr::Custom(
            "approved migration manifest source fingerprint is invalid".to_string(),
        ));
    }
    let comparison_key = load_migration_comparison_key()?;
    let actual_fingerprint = migration_source_fingerprint(tx, backend, &comparison_key).await?;
    if actual_fingerprint != manifest.source_fingerprint {
        return Err(DbErr::Custom(
            "approved migration manifest source fingerprint does not match".to_string(),
        ));
    }

    let source_group_ids = source_groups
        .iter()
        .map(|row| row.try_get::<String>("", "id"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut groups = BTreeMap::new();
    for entry in manifest.groups {
        if !source_group_ids.contains(&entry.source_group_id) {
            return Err(DbErr::Custom(
                "approved migration manifest contains an unknown Group".to_string(),
            ));
        }
        let approved =
            validate_approved_name(&entry.public_name, &entry.public_name_key_hex, "Group")?;
        if groups.insert(entry.source_group_id, approved).is_some() {
            return Err(DbErr::Custom(
                "approved migration manifest contains a duplicate Group".to_string(),
            ));
        }
    }
    if groups.len() != source_group_ids.len() {
        return Err(DbErr::Custom(
            "approved migration manifest Group set is incomplete".to_string(),
        ));
    }

    let mut targets = BTreeMap::new();
    for entry in manifest.targets {
        validate_approved_name(
            &entry.provider_public_name,
            &entry.provider_public_name_key_hex,
            "Provider",
        )?;
        validate_approved_name(
            &entry.channel_public_name,
            &entry.channel_public_name_key_hex,
            "Channel",
        )?;
        let key = (
            entry.source_provider_id.clone(),
            entry.target_group_id.clone(),
            entry.source_channel_id.clone(),
        );
        if targets.insert(key, entry).is_some() {
            return Err(DbErr::Custom(
                "approved migration manifest contains a duplicate target".to_string(),
            ));
        }
    }

    let approved_semantic_changes = manifest
        .approved_semantic_change_source_provider_ids
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut expected_semantic_changes = BTreeSet::new();
    let mut expected_target_count = 0usize;
    for provider in source_providers {
        let provider_id: String = provider.try_get("", "id")?;
        let group_ids = decode_groups(
            provider
                .try_get::<Option<String>>("", "group_ids")?
                .as_deref(),
            &provider_id,
        )?;
        if group_ids.iter().any(|id| !source_group_ids.contains(id)) {
            return Err(DbErr::Custom(format!(
                "provider {provider_id} references an unknown Group"
            )));
        }
        let channels = tx
            .query_all(Statement::from_sql_and_values(
                backend,
                numbered(
                    backend,
                    "SELECT id, enabled, weight FROM monoize_channels WHERE provider_id = ? ORDER BY created_at, id",
                ),
                vec![provider_id.clone().into()],
            ))
            .await?;
        if channels.is_empty() {
            return Err(DbErr::Custom(format!(
                "provider {provider_id} has no channel"
            )));
        }
        let enabled_positive = channels
            .iter()
            .filter(|row| {
                row.try_get::<i32>("", "enabled").unwrap_or_default() != 0
                    && row.try_get::<i32>("", "weight").unwrap_or_default() > 0
            })
            .count();
        if group_ids.len() > 1 || enabled_positive > 1 {
            expected_semantic_changes.insert(provider_id.clone());
        }
        for (group_index, group_id) in group_ids.iter().enumerate() {
            for (channel_index, channel) in channels.iter().enumerate() {
                let channel_id: String = channel.try_get("", "id")?;
                let first_pair = group_index == 0 && channel_index == 0;
                let expected_provider_id = if first_pair {
                    provider_id.clone()
                } else {
                    deterministic_id("provider", &provider_id, group_id, &channel_id)
                };
                let expected_channel_id = if group_index == 0 {
                    channel_id.clone()
                } else {
                    deterministic_id("channel", &provider_id, group_id, &channel_id)
                };
                let entry = targets
                    .get(&(provider_id.clone(), group_id.clone(), channel_id.clone()))
                    .ok_or_else(|| {
                        DbErr::Custom(format!(
                            "approved migration manifest target missing for provider {provider_id}"
                        ))
                    })?;
                if entry.target_provider_id != expected_provider_id
                    || entry.target_channel_id != expected_channel_id
                {
                    return Err(DbErr::Custom(format!(
                        "approved migration manifest target ID mismatch for provider {provider_id}"
                    )));
                }
                expected_target_count = expected_target_count
                    .checked_add(1)
                    .ok_or_else(|| DbErr::Custom("migration target count overflow".to_string()))?;
            }
        }
    }
    if targets.len() != expected_target_count {
        return Err(DbErr::Custom(
            "approved migration manifest target set contains additional entries".to_string(),
        ));
    }
    if approved_semantic_changes != expected_semantic_changes {
        return Err(DbErr::Custom(
            "approved migration manifest semantic-change approvals do not match".to_string(),
        ));
    }
    Ok(MigrationApproval { groups, targets })
}

fn validate_approved_name(value: &str, key_hex: &str, entity: &str) -> Result<ApprovedName, DbErr> {
    let canonical = canonical_public_name(value)?;
    if canonical != value {
        return Err(DbErr::Custom(format!(
            "approved migration manifest {entity} name is not canonical"
        )));
    }
    let expected_hex = lowercase_hex(value.as_bytes());
    if key_hex != expected_hex {
        return Err(DbErr::Custom(format!(
            "approved migration manifest {entity} key does not match"
        )));
    }
    Ok(ApprovedName {
        value: value.to_string(),
        key: value.as_bytes().to_vec(),
    })
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(*byte, b'a'..=b'f'))
}

fn lowercase_hex(value: &[u8]) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[cfg(test)]
fn load_migration_comparison_key() -> Result<Vec<u8>, DbErr> {
    Ok(b"monoize-provider-migration-test-key".to_vec())
}

#[cfg(not(test))]
fn load_migration_comparison_key() -> Result<Vec<u8>, DbErr> {
    let path = std::env::var("MONOIZE_PROVIDER_MIGRATION_COMPARISON_KEY_FILE").map_err(|_| {
        DbErr::Custom("MONOIZE_PROVIDER_MIGRATION_COMPARISON_KEY_FILE is required".to_string())
    })?;
    let key = fs::read(path)
        .map_err(|_| DbErr::Custom("migration comparison key file is unreadable".to_string()))?;
    if key.is_empty() || key.len() > 4096 {
        return Err(DbErr::Custom(
            "migration comparison key file must contain 1 through 4096 bytes".to_string(),
        ));
    }
    Ok(key)
}

pub(crate) async fn migration_source_fingerprint<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    comparison_key: &[u8],
) -> Result<String, DbErr> {
    let mut hash = Sha256::new();

    for row in connection
        .query_all(Statement::from_string(
            backend,
            "SELECT id, name, is_default FROM monoize_groups ORDER BY id".to_string(),
        ))
        .await?
    {
        let id: String = row.try_get("", "id")?;
        fingerprint_text(
            &mut hash,
            "group",
            &id,
            "id",
            Some(&id),
            false,
            comparison_key,
        );
        fingerprint_text(
            &mut hash,
            "group",
            &id,
            "name",
            Some(&row.try_get::<String>("", "name")?),
            false,
            comparison_key,
        );
        fingerprint_integer(
            &mut hash,
            "group",
            &id,
            "is_default",
            row.try_get::<i32>("", "is_default")?,
            comparison_key,
        );
    }

    for row in connection
        .query_all(Statement::from_string(
            backend,
            "SELECT id, name, max_retries, channel_max_retries, channel_retry_interval_ms, circuit_breaker_enabled, per_model_circuit_break, transforms, api_type_overrides, active_probe_enabled_override, active_probe_interval_seconds_override, active_probe_success_threshold_override, active_probe_model_override, request_timeout_ms_override, extra_fields_whitelist, strip_cross_protocol_nested_extra, group_ids, enabled, priority, created_at, updated_at FROM monoize_providers ORDER BY id".to_string(),
        ))
        .await?
    {
        let id: String = row.try_get("", "id")?;
        for column in [
            "id",
            "name",
            "transforms",
            "api_type_overrides",
            "active_probe_model_override",
            "extra_fields_whitelist",
            "group_ids",
            "created_at",
            "updated_at",
        ] {
            let value = if column == "id" {
                Some(id.clone())
            } else {
                row.try_get::<Option<String>>("", column)?
            };
            fingerprint_text(
                &mut hash,
                "provider",
                &id,
                column,
                value.as_deref(),
                false,
                comparison_key,
            );
        }
        for column in [
            "max_retries",
            "channel_max_retries",
            "channel_retry_interval_ms",
            "circuit_breaker_enabled",
            "per_model_circuit_break",
            "active_probe_enabled_override",
            "active_probe_interval_seconds_override",
            "active_probe_success_threshold_override",
            "request_timeout_ms_override",
            "strip_cross_protocol_nested_extra",
            "enabled",
            "priority",
        ] {
            fingerprint_optional_integer(
                &mut hash,
                "provider",
                &id,
                column,
                row.try_get::<Option<i32>>("", column)?,
                comparison_key,
            );
        }
    }

    for row in connection
        .query_all(Statement::from_string(
            backend,
            "SELECT id, provider_id, name, provider_type, base_url, api_key, weight, enabled, created_at, passive_failure_count_threshold_override, passive_cooldown_seconds_override, passive_window_seconds_override, passive_rate_limit_cooldown_seconds_override, active_probe_enabled_override, active_probe_interval_seconds_override, active_probe_success_threshold_override, active_probe_model_override, affinity_enabled_override, affinity_idle_ttl_seconds_override, affinity_failback_mode_override, affinity_failback_delay_seconds_override, proxy_url, extra_headers, session_affinity_auto, allow_missing_usage FROM monoize_channels ORDER BY provider_id, created_at, id".to_string(),
        ))
        .await?
    {
        let id: String = row.try_get("", "id")?;
        for column in [
            "id",
            "provider_id",
            "name",
            "provider_type",
            "base_url",
            "api_key",
            "created_at",
            "active_probe_model_override",
            "affinity_failback_mode_override",
            "proxy_url",
            "extra_headers",
        ] {
            let value = if column == "id" {
                Some(id.clone())
            } else {
                row.try_get::<Option<String>>("", column)?
            };
            fingerprint_text(
                &mut hash,
                "channel",
                &id,
                column,
                value.as_deref(),
                matches!(column, "api_key" | "proxy_url" | "extra_headers"),
                comparison_key,
            );
        }
        for column in [
            "weight",
            "enabled",
            "passive_failure_count_threshold_override",
            "passive_cooldown_seconds_override",
            "passive_window_seconds_override",
            "passive_rate_limit_cooldown_seconds_override",
            "active_probe_enabled_override",
            "active_probe_interval_seconds_override",
            "active_probe_success_threshold_override",
            "affinity_enabled_override",
            "affinity_idle_ttl_seconds_override",
            "affinity_failback_delay_seconds_override",
            "session_affinity_auto",
            "allow_missing_usage",
        ] {
            fingerprint_optional_integer(
                &mut hash,
                "channel",
                &id,
                column,
                row.try_get::<Option<i32>>("", column)?,
                comparison_key,
            );
        }
    }

    for row in connection
        .query_all(Statement::from_string(
            backend,
            "SELECT channel_id, model_name, redirect, multiplier, created_at FROM monoize_channel_models ORDER BY channel_id, model_name".to_string(),
        ))
        .await?
    {
        let channel_id: String = row.try_get("", "channel_id")?;
        let model_name: String = row.try_get("", "model_name")?;
        let row_id = format!("{channel_id}\0{model_name}");
        for (column, value) in [
            ("channel_id", Some(channel_id.as_str())),
            ("model_name", Some(model_name.as_str())),
            (
                "redirect",
                row.try_get::<Option<String>>("", "redirect")?.as_deref(),
            ),
            (
                "multiplier",
                Some(row.try_get::<String>("", "multiplier")?.as_str()),
            ),
            (
                "created_at",
                Some(row.try_get::<String>("", "created_at")?.as_str()),
            ),
        ] {
            fingerprint_text(
                &mut hash,
                "model",
                &row_id,
                column,
                value,
                false,
                comparison_key,
            );
        }
    }

    for row in connection
        .query_all(Statement::from_string(
            backend,
            "SELECT key, value FROM system_settings WHERE key IN ('pricing_profile_model_patterns','reasoning_suffix_map') ORDER BY key".to_string(),
        ))
        .await?
    {
        let key: String = row.try_get("", "key")?;
        fingerprint_text(&mut hash, "setting", &key, "key", Some(&key), false, comparison_key);
        fingerprint_text(
            &mut hash,
            "setting",
            &key,
            "value",
            Some(&row.try_get::<String>("", "value")?),
            false,
            comparison_key,
        );
    }

    for row in connection
        .query_all(Statement::from_string(
            backend,
            "SELECT model_id, models_dev_provider FROM model_metadata_records WHERE models_dev_provider IS NOT NULL ORDER BY model_id".to_string(),
        ))
        .await?
    {
        let model_id: String = row.try_get("", "model_id")?;
        fingerprint_text(
            &mut hash,
            "metadata",
            &model_id,
            "model_id",
            Some(&model_id),
            false,
            comparison_key,
        );
        fingerprint_text(
            &mut hash,
            "metadata",
            &model_id,
            "models_dev_provider",
            row.try_get::<Option<String>>("", "models_dev_provider")?
                .as_deref(),
            false,
            comparison_key,
        );
    }

    for row in connection
        .query_all(Statement::from_string(
            backend,
            "SELECT id, pricing_profile, model_pattern, provider_type, rate_kind, usage_class, unit_price_nano_usd, context_tier, service_tier, modality, cache_ttl, match_json, priority FROM billing_rate_records WHERE enabled = 1 ORDER BY priority DESC, id ASC".to_string(),
        ))
        .await?
    {
        let id: String = row.try_get("", "id")?;
        for column in [
            "id",
            "pricing_profile",
            "model_pattern",
            "provider_type",
            "rate_kind",
            "usage_class",
            "unit_price_nano_usd",
            "context_tier",
            "service_tier",
            "modality",
            "cache_ttl",
            "match_json",
        ] {
            let value = if column == "id" {
                Some(id.clone())
            } else {
                row.try_get::<Option<String>>("", column)?
            };
            fingerprint_text(
                &mut hash,
                "rate",
                &id,
                column,
                value.as_deref(),
                false,
                comparison_key,
            );
        }
        fingerprint_integer(
            &mut hash,
            "rate",
            &id,
            "priority",
            row.try_get::<i32>("", "priority")?,
            comparison_key,
        );
    }

    Ok(lowercase_hex(&hash.finalize()))
}

fn fingerprint_optional_integer(
    hash: &mut Sha256,
    table: &str,
    row_id: &str,
    column: &str,
    value: Option<i32>,
    comparison_key: &[u8],
) {
    let encoded = value.map(|value| value.to_string());
    fingerprint_text(
        hash,
        table,
        row_id,
        column,
        encoded.as_deref(),
        false,
        comparison_key,
    );
}

fn fingerprint_integer(
    hash: &mut Sha256,
    table: &str,
    row_id: &str,
    column: &str,
    value: i32,
    comparison_key: &[u8],
) {
    fingerprint_optional_integer(hash, table, row_id, column, Some(value), comparison_key);
}

fn fingerprint_text(
    hash: &mut Sha256,
    table: &str,
    row_id: &str,
    column: &str,
    value: Option<&str>,
    secret: bool,
    comparison_key: &[u8],
) {
    for component in [table.as_bytes(), row_id.as_bytes(), column.as_bytes()] {
        hash.update((component.len() as u64).to_be_bytes());
        hash.update(component);
    }
    match value {
        None => hash.update([0]),
        Some(value) if secret => {
            hash.update([2]);
            let tag = format!("{table}\0{row_id}\0{column}\0");
            let mut material = Vec::with_capacity(tag.len() + value.len());
            material.extend_from_slice(tag.as_bytes());
            material.extend_from_slice(value.as_bytes());
            hash.update(hmac_sha256(comparison_key, &material));
        }
        Some(value) => {
            hash.update([1]);
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
    }
}

fn hmac_sha256(key: &[u8], value: &[u8]) -> [u8; 32] {
    let normalized_key = if key.len() > 64 {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    let mut inner_pad = [0x36; 64];
    let mut outer_pad = [0x5c; 64];
    for (index, byte) in normalized_key.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(value);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

async fn migrate_group_public_names(
    tx: &DatabaseTransaction,
    backend: DbBackend,
    key_type: &str,
    approved_names: &BTreeMap<String, ApprovedName>,
) -> Result<(), DbErr> {
    let add_public_name = if backend == DbBackend::Postgres {
        "ALTER TABLE monoize_groups ADD COLUMN IF NOT EXISTS public_name TEXT"
    } else {
        "ALTER TABLE monoize_groups ADD COLUMN public_name TEXT"
    };
    let add_public_name_key = if backend == DbBackend::Postgres {
        format!("ALTER TABLE monoize_groups ADD COLUMN IF NOT EXISTS public_name_key {key_type}")
    } else {
        format!("ALTER TABLE monoize_groups ADD COLUMN public_name_key {key_type}")
    };
    tx.execute(Statement::from_string(backend, add_public_name.to_string()))
        .await?;
    tx.execute(Statement::from_string(backend, add_public_name_key))
        .await?;
    let groups = tx
        .query_all(Statement::from_string(
            backend,
            "SELECT id, name FROM monoize_groups ORDER BY id".to_string(),
        ))
        .await?;
    for group in groups {
        let id: String = group.try_get("", "id")?;
        let public_name = approved_names.get(&id).ok_or_else(|| {
            DbErr::Custom(format!(
                "approved migration manifest Group missing for source {id}"
            ))
        })?;
        tx.execute(Statement::from_sql_and_values(
            backend,
            numbered(
                backend,
                "UPDATE monoize_groups SET name = ?, public_name = ?, public_name_key = ? WHERE id = ?",
            ),
            vec![
                group.try_get::<String>("", "name")?.into(),
                public_name.value.clone().into(),
                bytes(backend, &public_name.key),
                id.into(),
            ],
        ))
        .await?;
    }
    tx.execute(Statement::from_string(
        backend,
        "DROP INDEX IF EXISTS uq_monoize_groups_name_lower".to_string(),
    ))
    .await?;
    tx.execute(Statement::from_string(
        backend,
        "CREATE UNIQUE INDEX uq_monoize_groups_public_name ON monoize_groups(public_name_key)"
            .to_string(),
    ))
    .await?;
    if backend == DbBackend::Postgres {
        for statement in [
            "ALTER TABLE monoize_groups ALTER COLUMN public_name SET NOT NULL",
            "ALTER TABLE monoize_groups ALTER COLUMN public_name_key SET NOT NULL",
            "ALTER TABLE monoize_groups ADD CONSTRAINT ck_monoize_groups_public_name_key CHECK (public_name_key = convert_to(public_name, 'UTF8'))",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
    } else {
        for statement in [
            "CREATE TRIGGER monoize_groups_public_name_insert_guard BEFORE INSERT ON monoize_groups WHEN NEW.public_name IS NULL OR NEW.public_name_key IS NULL OR NEW.public_name_key != CAST(NEW.public_name AS BLOB) BEGIN SELECT RAISE(ABORT, 'invalid group public name'); END",
            "CREATE TRIGGER monoize_groups_public_name_update_guard BEFORE UPDATE OF public_name, public_name_key ON monoize_groups WHEN NEW.public_name IS NULL OR NEW.public_name_key IS NULL OR NEW.public_name_key != CAST(NEW.public_name AS BLOB) BEGIN SELECT RAISE(ABORT, 'invalid group public name'); END",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
    }
    Ok(())
}

fn canonical_public_name(raw: &str) -> Result<String, DbErr> {
    let name = raw
        .trim_matches(char::is_whitespace)
        .nfc()
        .collect::<String>();
    if !(1..=64).contains(&name.chars().count())
        || name
            .as_bytes()
            .iter()
            .any(|byte| matches!(*byte, 0x00..=0x1f | 0x7f))
    {
        return Err(DbErr::Custom("invalid migrated public name".to_string()));
    }
    Ok(name)
}

struct ProviderInsert<'a> {
    group_id: &'a str,
    provider_id: &'a str,
    channel_id: &'a str,
    name: &'a str,
    channel_name: &'a str,
    public_name: &'a str,
    channel_public_name: &'a str,
    priority: i32,
    pricing_profile: Option<&'a str>,
    multiplier: &'a str,
    now: &'a str,
}

async fn insert_provider(
    tx: &DatabaseTransaction,
    backend: DbBackend,
    row: &sea_orm::QueryResult,
    channel: &sea_orm::QueryResult,
    input: ProviderInsert<'_>,
) -> Result<(), DbErr> {
    let value = |column: &str| row.try_get::<String>("", column).unwrap_or_default();
    let opt = |column: &str| row.try_get::<Option<i32>>("", column).unwrap_or(None);
    let ch_opt = |column: &str| channel.try_get::<Option<i32>>("", column).unwrap_or(None);
    let sql = numbered(
        backend,
        "INSERT INTO monoize_providers (id,group_id,name,public_name,public_name_key,priority,enabled,pricing_profile,multiplier,configuration_generation,created_at,updated_at,channel_id,channel_name,channel_public_name,channel_public_name_key,channel_provider_type,channel_base_url,channel_api_key,channel_enabled,channel_max_retries,channel_passive_failure_count_threshold_override,channel_passive_cooldown_seconds_override,channel_passive_window_seconds_override,channel_passive_rate_limit_cooldown_seconds_override,channel_active_probe_enabled_override,channel_active_probe_interval_seconds_override,channel_active_probe_success_threshold_override,channel_active_probe_model_override,channel_affinity_enabled_override,channel_affinity_idle_ttl_seconds_override,channel_affinity_failback_mode_override,channel_affinity_failback_delay_seconds_override,channel_proxy_url,channel_extra_headers,channel_session_affinity_auto,channel_allow_missing_usage,transforms,api_type_overrides,active_probe_enabled_override,active_probe_interval_seconds_override,active_probe_success_threshold_override,active_probe_model_override,request_timeout_ms_override,extra_fields_whitelist,strip_cross_protocol_nested_extra,circuit_breaker_enabled,per_model_circuit_break,channel_retry_interval_ms) VALUES (?,?,?,?,?,?,?,?,?,1,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    );
    let mut values: Vec<Value> = vec![
        input.provider_id.into(),
        input.group_id.into(),
        input.name.into(),
        input.public_name.into(),
        bytes(backend, input.public_name.as_bytes()),
        input.priority.into(),
        row.try_get::<i32>("", "enabled")?.into(),
        input.pricing_profile.map(str::to_owned).into(),
        input.multiplier.into(),
        input.now.into(),
        input.now.into(),
        input.channel_id.into(),
        input.channel_name.into(),
        input.channel_public_name.into(),
        bytes(backend, input.channel_public_name.as_bytes()),
        channel.try_get::<String>("", "provider_type")?.into(),
        channel.try_get::<String>("", "base_url")?.into(),
        channel.try_get::<String>("", "api_key")?.into(),
        i32::from(
            channel.try_get::<i32>("", "enabled")? != 0
                && channel.try_get::<i32>("", "weight")? > 0,
        )
        .into(),
        row.try_get::<i32>("", "channel_max_retries")?.into(),
    ];
    for col in [
        "passive_failure_count_threshold_override",
        "passive_cooldown_seconds_override",
        "passive_window_seconds_override",
        "passive_rate_limit_cooldown_seconds_override",
        "active_probe_enabled_override",
        "active_probe_interval_seconds_override",
        "active_probe_success_threshold_override",
    ] {
        values.push(ch_opt(col).into());
    }
    values.push(
        channel
            .try_get::<Option<String>>("", "active_probe_model_override")?
            .into(),
    );
    for col in [
        "affinity_enabled_override",
        "affinity_idle_ttl_seconds_override",
    ] {
        values.push(ch_opt(col).into());
    }
    values.push(
        channel
            .try_get::<Option<String>>("", "affinity_failback_mode_override")?
            .into(),
    );
    values.push(ch_opt("affinity_failback_delay_seconds_override").into());
    for col in ["proxy_url", "extra_headers"] {
        values.push(channel.try_get::<Option<String>>("", col)?.into());
    }
    values.push(
        channel
            .try_get::<Option<i32>>("", "session_affinity_auto")?
            .into(),
    );
    values.push(
        channel
            .try_get::<i32>("", "allow_missing_usage")
            .unwrap_or(0)
            .into(),
    );
    for col in ["transforms", "api_type_overrides"] {
        values.push(value(col).into());
    }
    for col in [
        "active_probe_enabled_override",
        "active_probe_interval_seconds_override",
        "active_probe_success_threshold_override",
    ] {
        values.push(opt(col).into());
    }
    values.push(
        row.try_get::<Option<String>>("", "active_probe_model_override")?
            .into(),
    );
    values.push(
        row.try_get::<Option<i32>>("", "request_timeout_ms_override")?
            .into(),
    );
    values.push(
        row.try_get::<Option<String>>("", "extra_fields_whitelist")?
            .into(),
    );
    values.push(
        row.try_get::<Option<i32>>("", "strip_cross_protocol_nested_extra")?
            .into(),
    );
    values.push(
        row.try_get::<i32>("", "circuit_breaker_enabled")
            .unwrap_or(1)
            .into(),
    );
    values.push(
        row.try_get::<i32>("", "per_model_circuit_break")
            .unwrap_or(0)
            .into(),
    );
    values.push(
        row.try_get::<i32>("", "channel_retry_interval_ms")
            .unwrap_or(0)
            .into(),
    );
    tx.execute(Statement::from_sql_and_values(backend, sql, values))
        .await?;
    Ok(())
}

async fn load_legacy_pricing_state(
    tx: &DatabaseTransaction,
    backend: DbBackend,
) -> Result<LegacyPricingState, DbErr> {
    let patterns = match load_setting(tx, backend, "pricing_profile_model_patterns").await? {
        Some(raw) => serde_json::from_str(&raw).map_err(|error| {
            DbErr::Custom(format!(
                "invalid pricing_profile_model_patterns setting: {error}"
            ))
        })?,
        None => default_pricing_profile_patterns(),
    };
    let reasoning_suffix_map = match load_setting(tx, backend, "reasoning_suffix_map").await? {
        Some(raw) => serde_json::from_str(&raw).map_err(|error| {
            DbErr::Custom(format!("invalid reasoning_suffix_map setting: {error}"))
        })?,
        None => default_reasoning_suffix_map(),
    };
    let metadata_profiles = tx
        .query_all(Statement::from_string(
            backend,
            "SELECT model_id, models_dev_provider FROM model_metadata_records WHERE models_dev_provider IS NOT NULL"
                .to_string(),
        ))
        .await?
        .into_iter()
        .filter_map(|row| {
            let model_id = row.try_get::<String>("", "model_id").ok()?;
            let profile = row
                .try_get::<String>("", "models_dev_provider")
                .ok()?
                .trim()
                .to_string();
            (!profile.is_empty()).then_some((model_id, profile))
        })
        .collect();
    let rates = tx
        .query_all(Statement::from_string(
            backend,
            "SELECT pricing_profile, model_pattern, provider_type, rate_kind, usage_class, unit_price_nano_usd, context_tier, service_tier, modality, cache_ttl, match_json FROM billing_rate_records WHERE enabled = 1 ORDER BY priority DESC, id ASC".to_string(),
        ))
        .await?
        .into_iter()
        .map(|row| {
            let match_json = row.try_get::<String>("", "match_json")?;
            Ok(BillingRate {
                pricing_profile: row.try_get("", "pricing_profile")?,
                model_pattern: row.try_get("", "model_pattern")?,
                provider_type: row.try_get("", "provider_type")?,
                rate_kind: row.try_get("", "rate_kind")?,
                usage_class: row.try_get("", "usage_class")?,
                unit_price_nano_usd: row.try_get("", "unit_price_nano_usd")?,
                context_tier: row.try_get("", "context_tier")?,
                service_tier: row.try_get("", "service_tier")?,
                modality: row.try_get("", "modality")?,
                cache_ttl: row.try_get("", "cache_ttl")?,
                match_json: serde_json::from_str(&match_json).map_err(|error| {
                    DbErr::Custom(format!("invalid billing rate match_json: {error}"))
                })?,
            })
        })
        .collect::<Result<Vec<_>, DbErr>>()?;
    Ok(LegacyPricingState {
        patterns,
        reasoning_suffix_map,
        metadata_profiles,
        rates,
    })
}

async fn load_setting(
    tx: &DatabaseTransaction,
    backend: DbBackend,
    key: &str,
) -> Result<Option<String>, DbErr> {
    tx.query_one(Statement::from_sql_and_values(
        backend,
        numbered(
            backend,
            "SELECT value FROM system_settings WHERE key = ? LIMIT 1",
        ),
        vec![key.into()],
    ))
    .await?
    .map(|row| row.try_get("", "value"))
    .transpose()
}

async fn load_legacy_models(
    tx: &DatabaseTransaction,
    backend: DbBackend,
    channel_id: &str,
    default_provider_type: &str,
    api_type_overrides: &[ApiTypeOverride],
    pricing_state: &LegacyPricingState,
    now: &str,
) -> Result<Vec<LegacyModel>, DbErr> {
    let rows = tx
        .query_all(Statement::from_sql_and_values(
            backend,
            numbered(
                backend,
                "SELECT model_name, redirect, multiplier, created_at FROM monoize_channel_models_legacy_flatten WHERE channel_id = ? ORDER BY model_name",
            ),
            vec![channel_id.into()],
        ))
        .await?;
    rows.into_iter()
        .map(|row| {
            let model_name: String = row.try_get("", "model_name")?;
            let redirect: Option<String> = row.try_get("", "redirect")?;
            let multiplier = canonical_multiplier(
                &row.try_get::<String>("", "multiplier")
                    .unwrap_or_else(|_| "1".to_string()),
            )?;
            let provider_type = api_type_overrides
                .iter()
                .find(|entry| case_sensitive_glob_matches(&entry.pattern, &model_name))
                .map(|entry| entry.api_type.as_str())
                .unwrap_or(default_provider_type);
            let upstream_model = redirect
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(&model_name);
            let resolved_profile =
                resolve_legacy_profile(pricing_state, upstream_model, &model_name, provider_type);
            Ok(LegacyModel {
                model_name,
                redirect,
                multiplier,
                resolved_profile,
                created_at: row
                    .try_get::<String>("", "created_at")
                    .unwrap_or_else(|_| now.to_string()),
            })
        })
        .collect()
}

fn resolve_legacy_profile(
    state: &LegacyPricingState,
    upstream_model: &str,
    logical_model: &str,
    provider_type: &str,
) -> Option<String> {
    let normalized_upstream =
        normalize_pricing_model_key(upstream_model, &state.reasoning_suffix_map);
    let upstream = resolve_profile_for_model(state, &normalized_upstream, provider_type);
    if let Some((profile, true)) = &upstream {
        return Some(profile.clone());
    }
    let normalized_logical =
        normalize_pricing_model_key(logical_model, &state.reasoning_suffix_map);
    if normalized_logical == normalized_upstream {
        return upstream.map(|(profile, _)| profile);
    }
    let logical = resolve_profile_for_model(state, &normalized_logical, provider_type);
    if let Some((profile, true)) = &logical {
        return Some(profile.clone());
    }
    upstream.or(logical).map(|(profile, _)| profile)
}

fn resolve_profile_for_model(
    state: &LegacyPricingState,
    model: &str,
    provider_type: &str,
) -> Option<(String, bool)> {
    let mut candidate_profiles = Vec::new();
    if let Some(profile) = state
        .patterns
        .iter()
        .find(|entry| ascii_case_insensitive_glob_matches(&entry.pattern, model))
        .map(|entry| entry.pricing_profile.clone())
    {
        candidate_profiles.push(profile);
    }
    if let Some(profile) = state.metadata_profiles.get(model)
        && !candidate_profiles
            .iter()
            .any(|candidate| candidate == profile)
    {
        candidate_profiles.push(profile.clone());
    }
    let mut first_non_empty = None;
    let mut first_known_profile = None;
    for profile in candidate_profiles {
        if first_known_profile.is_none()
            && state
                .rates
                .iter()
                .any(|rate| rate.pricing_profile == profile)
        {
            first_known_profile = Some(profile.clone());
        }
        let rates = state
            .rates
            .iter()
            .filter(|rate| {
                rate.pricing_profile == profile
                    && rate
                        .provider_type
                        .as_deref()
                        .is_none_or(|value| value == provider_type)
                    && rate
                        .model_pattern
                        .as_deref()
                        .is_none_or(|pattern| ascii_case_insensitive_glob_matches(pattern, model))
            })
            .collect::<Vec<_>>();
        if rates.is_empty() {
            continue;
        }
        let complete = billing_rate_matrix_is_complete(&rates);
        if complete {
            return Some((profile, true));
        }
        if first_non_empty.is_none() {
            first_non_empty = Some((profile, false));
        }
    }
    first_non_empty.or_else(|| first_known_profile.map(|profile| (profile, false)))
}

fn billing_rate_matrix_is_complete(rates: &[&BillingRate]) -> bool {
    if rates.iter().any(|rate| {
        rate.unit_price_nano_usd
            .parse::<i128>()
            .ok()
            .is_none_or(|value| value < 0 || value.to_string() != rate.unit_price_nano_usd)
    }) {
        return false;
    }
    let context_tiers = rates
        .iter()
        .filter_map(|rate| rate.context_tier.as_deref())
        .filter(|tier| *tier != "default")
        .collect::<BTreeSet<_>>();
    let has_dimensionless = |usage_class: &str, context_tier: Option<&str>| {
        rates.iter().any(|rate| {
            rate.rate_kind == "token"
                && rate.usage_class == usage_class
                && rate.modality.is_none()
                && rate.cache_ttl.is_none()
                && rate
                    .service_tier
                    .as_deref()
                    .is_none_or(|tier| tier == "default")
                && match context_tier {
                    Some(tier) => rate.context_tier.as_deref() == Some(tier),
                    None => rate
                        .context_tier
                        .as_deref()
                        .is_none_or(|tier| tier == "default"),
                }
        })
    };
    if context_tiers.is_empty() {
        return has_dimensionless("input_uncached", None) && has_dimensionless("output", None);
    }
    let has_threshold = rates.iter().any(|rate| {
        rate.match_json
            .get("context_threshold_tokens")
            .is_some_and(|value| {
                value.as_u64().is_some()
                    || value
                        .as_str()
                        .and_then(|raw| raw.parse::<u64>().ok())
                        .is_some()
            })
    });
    has_threshold
        && context_tiers.iter().all(|tier| {
            has_dimensionless("input_uncached", Some(tier))
                && has_dimensionless("output", Some(tier))
        })
}

fn infer_default_profile(models: &[LegacyModel]) -> Option<String> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for profile in models
        .iter()
        .filter_map(|model| model.resolved_profile.as_deref())
    {
        *counts.entry(profile).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(left_profile, left_count), (right_profile, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_profile.as_bytes().cmp(left_profile.as_bytes()))
        })
        .map(|(profile, _)| profile.to_string())
}

fn infer_default_multiplier(models: &[LegacyModel]) -> Result<String, DbErr> {
    if models.is_empty() {
        return Ok("1".to_string());
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    for model in models {
        *counts.entry(&model.multiplier).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|(left_value, left_count), (right_value, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| decimal_cmp(right_value, left_value))
        })
        .map(|(value, _)| value.to_string())
        .ok_or_else(|| DbErr::Custom("model multiplier inference failed".to_string()))
}

fn canonical_multiplier(value: &str) -> Result<String, DbErr> {
    if value.is_empty()
        || value.starts_with('+')
        || value.contains(['e', 'E'])
        || value
            .split_once('.')
            .is_some_and(|(_, fraction)| fraction.len() > 9)
    {
        return Err(DbErr::Custom(format!("invalid model multiplier: {value}")));
    }
    let decimal = Decimal::from_str(value)
        .map_err(|_| DbErr::Custom(format!("invalid model multiplier: {value}")))?;
    if decimal <= Decimal::ZERO {
        return Err(DbErr::Custom(format!("invalid model multiplier: {value}")));
    }
    Ok(decimal.normalize().to_string())
}

fn decimal_cmp(left: &str, right: &str) -> Ordering {
    Decimal::from_str(left)
        .expect("canonical multiplier")
        .cmp(&Decimal::from_str(right).expect("canonical multiplier"))
}

fn normalize_pricing_model_key(
    model_id: &str,
    reasoning_suffix_map: &HashMap<String, String>,
) -> String {
    let trimmed = model_id.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut suffixes = reasoning_suffix_map
        .keys()
        .map(String::as_str)
        .chain([
            "-none", "-minimum", "-low", "-medium", "-high", "-xhigh", "-max",
        ])
        .collect::<Vec<_>>();
    suffixes.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    suffixes.dedup();
    suffixes
        .into_iter()
        .find_map(|suffix| trimmed.strip_suffix(suffix).filter(|base| !base.is_empty()))
        .unwrap_or(trimmed)
        .to_string()
}

fn default_reasoning_suffix_map() -> HashMap<String, String> {
    [
        ("-thinking", "high"),
        ("-reasoning", "high"),
        ("-nothinking", "none"),
    ]
    .into_iter()
    .map(|(suffix, effort)| (suffix.to_string(), effort.to_string()))
    .collect()
}

fn default_pricing_profile_patterns() -> Vec<PricingProfilePattern> {
    [
        ("gpt-image-*", "openai"),
        ("text-embedding-*", "openai"),
        ("gpt-*", "openai"),
        ("o*", "openai"),
        ("claude-*", "anthropic"),
        ("gemini-*", "google"),
        ("grok-*", "xai"),
        ("*", "default"),
    ]
    .into_iter()
    .map(|(pattern, pricing_profile)| PricingProfilePattern {
        pattern: pattern.to_string(),
        pricing_profile: pricing_profile.to_string(),
    })
    .collect()
}

fn ascii_case_insensitive_glob_matches(pattern: &str, value: &str) -> bool {
    glob_matches(pattern.as_bytes(), value.as_bytes(), true)
}

fn case_sensitive_glob_matches(pattern: &str, value: &str) -> bool {
    glob_matches(pattern.as_bytes(), value.as_bytes(), false)
}

fn glob_matches(pattern: &[u8], value: &[u8], ascii_case_insensitive: bool) -> bool {
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut last_star_index = None;
    let mut last_star_match_index = 0;
    while value_index < value.len() {
        let equal = pattern_index < pattern.len()
            && pattern[pattern_index] != b'*'
            && (pattern[pattern_index] == b'?'
                || pattern[pattern_index] == value[value_index]
                || (ascii_case_insensitive
                    && pattern[pattern_index].eq_ignore_ascii_case(&value[value_index])));
        if equal {
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

fn ascii_fold_bytes(value: &[u8]) -> Vec<u8> {
    value
        .iter()
        .map(|byte| {
            if byte.is_ascii_uppercase() {
                byte + 32
            } else {
                *byte
            }
        })
        .collect()
}

fn decode_groups(raw: Option<&str>, provider_id: &str) -> Result<Vec<String>, DbErr> {
    let raw =
        raw.ok_or_else(|| DbErr::Custom(format!("provider {provider_id} has missing group_ids")))?;
    let groups = serde_json::from_str::<Vec<String>>(raw).map_err(|error| {
        DbErr::Custom(format!(
            "provider {provider_id} has malformed group_ids: {error}"
        ))
    })?;
    if groups.is_empty() {
        return Err(DbErr::Custom(format!(
            "provider {provider_id} has zero Groups"
        )));
    }
    let mut normalized = Vec::with_capacity(groups.len());
    let mut unique = BTreeSet::new();
    for group in groups {
        let group = group.trim().to_string();
        if group.is_empty() || !unique.insert(group.clone()) {
            return Err(DbErr::Custom(format!(
                "provider {provider_id} has malformed group_ids"
            )));
        }
        normalized.push(group);
    }
    Ok(normalized)
}

async fn group_name(
    tx: &DatabaseTransaction,
    backend: DbBackend,
    group_id: &str,
) -> Result<String, DbErr> {
    tx.query_one(Statement::from_sql_and_values(
        backend,
        numbered(
            backend,
            "SELECT name FROM monoize_groups WHERE id = ? LIMIT 1",
        ),
        vec![group_id.into()],
    ))
    .await?
    .ok_or_else(|| DbErr::Custom(format!("provider references unknown group {group_id}")))
    .and_then(|row| row.try_get("", "name"))
}

fn deterministic_id(kind: &str, provider_id: &str, group_id: &str, channel_id: &str) -> String {
    let mut digest = Sha256::new();
    for component in [
        "lynshen-provider-migration-v1",
        kind,
        provider_id,
        group_id,
        channel_id,
    ] {
        let bytes = component.as_bytes();
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    let mut encoded = String::with_capacity(32);
    for byte in &digest.finalize()[..16] {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    let prefix = if kind == "provider" { "p_" } else { "c_" };
    format!("{prefix}{encoded}")
}

fn bytes(_backend: DbBackend, value: impl AsRef<[u8]>) -> Value {
    let value = value.as_ref();
    Value::Bytes(Some(Box::new(value.to_vec())))
}

fn numbered(backend: DbBackend, sql: &str) -> String {
    if backend != DbBackend::Postgres {
        return sql.to_string();
    }
    let mut out = String::new();
    let mut n = 0;
    for c in sql.chars() {
        if c == '?' {
            n += 1;
            out.push_str(&format!("${n}"));
        } else {
            out.push(c);
        }
    }
    out
}

fn ascii_fold_expression(column: &str) -> String {
    (b'A'..=b'Z').fold(column.to_owned(), |expression, upper| {
        format!(
            "replace({expression}, '{}', '{}')",
            char::from(upper),
            char::from(upper + 32)
        )
    })
}

fn canonical_decimal_check(column: &str, backend: DbBackend) -> String {
    if backend == DbBackend::Postgres {
        return format!(
            "{column} ~ '^(0\\.[0-9]{{0,8}}[1-9]|[1-9][0-9]*(\\.[0-9]{{0,8}}[1-9])?)$'"
        );
    }
    format!(
        "length({column}) >= 1 \
         AND {column} NOT GLOB '*[^0-9.]*' \
         AND substr({column}, 1, 1) <> '.' \
         AND substr({column}, -1, 1) <> '.' \
         AND length({column}) - length(replace({column}, '.', '')) <= 1 \
         AND (length({column}) = 1 OR substr({column}, 1, 1) <> '0' OR substr({column}, 2, 1) = '.') \
         AND (instr({column}, '.') = 0 OR length({column}) - instr({column}, '.') BETWEEN 1 AND 9) \
         AND (instr({column}, '.') = 0 OR substr({column}, -1, 1) <> '0') \
         AND CAST({column} AS NUMERIC) > 0"
    )
}

pub(crate) fn target_provider_ddl() -> Vec<String> {
    vec![
        "CREATE TABLE monoize_provider_models (...)".to_string(),
        "CREATE TABLE monoize_providers (... channel_id TEXT NOT NULL UNIQUE ...)".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::{ascii_fold_expression, target_provider_ddl};

    #[test]
    fn target_schema_contains_flattened_tables() {
        let ddl = target_provider_ddl();
        assert!(ddl.iter().any(|sql| sql.contains("channel_id")));
        assert!(
            ddl.iter()
                .any(|sql| sql.contains("monoize_provider_models"))
        );
        assert!(!ddl.iter().any(|sql| sql.contains("monoize_channels")));
    }

    #[test]
    fn model_search_key_uses_explicit_replacements() {
        let expression = ascii_fold_expression("model_name");
        assert!(expression.contains("'A', 'a'"));
        assert!(expression.contains("'Z', 'z'"));
        assert!(!expression.contains("lower("));
    }
}
