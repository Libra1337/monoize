use chrono::Utc;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbBackend, Statement, TransactionTrait, Value,
};
use sea_orm_migration::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt::Write;

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

    let now = Utc::now().to_rfc3339();
    let suffix = if backend == DbBackend::Postgres {
        "BYTEA"
    } else {
        "BLOB"
    };
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
    let statements = vec![
        "DROP INDEX IF EXISTS idx_mc_provider_id".to_string(),
        "DROP INDEX IF EXISTS idx_mcm_channel_id".to_string(),
        "DROP INDEX IF EXISTS uq_mcm_channel_id_model_name".to_string(),
        "DROP INDEX IF EXISTS idx_mcm_model_name_channel_id".to_string(),
        "ALTER TABLE monoize_providers RENAME TO monoize_providers_legacy_flatten".to_string(),
        "ALTER TABLE monoize_channels RENAME TO monoize_channels_legacy_flatten".to_string(),
        "ALTER TABLE monoize_channel_models RENAME TO monoize_channel_models_legacy_flatten".to_string(),
        format!("CREATE TABLE monoize_providers (id TEXT PRIMARY KEY NOT NULL, group_id TEXT NOT NULL REFERENCES monoize_groups(id) ON DELETE RESTRICT, name TEXT NOT NULL, public_name TEXT NOT NULL, public_name_key {suffix} NOT NULL CHECK(public_name_key = {provider_key_cast}), priority INTEGER NOT NULL, enabled INTEGER NOT NULL, pricing_profile TEXT, multiplier TEXT NOT NULL DEFAULT '1', configuration_generation BIGINT NOT NULL DEFAULT 1, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, channel_id TEXT NOT NULL UNIQUE, channel_name TEXT NOT NULL, channel_public_name TEXT NOT NULL, channel_public_name_key {suffix} NOT NULL CHECK(channel_public_name_key = {channel_key_cast}), channel_provider_type TEXT NOT NULL, channel_base_url TEXT NOT NULL, channel_api_key TEXT NOT NULL, channel_enabled INTEGER NOT NULL, channel_max_retries INTEGER NOT NULL DEFAULT 0, channel_passive_failure_count_threshold_override INTEGER, channel_passive_cooldown_seconds_override INTEGER, channel_passive_window_seconds_override INTEGER, channel_passive_rate_limit_cooldown_seconds_override INTEGER, channel_active_probe_enabled_override INTEGER, channel_active_probe_interval_seconds_override INTEGER, channel_active_probe_success_threshold_override INTEGER, channel_active_probe_model_override TEXT, channel_affinity_enabled_override INTEGER, channel_affinity_idle_ttl_seconds_override INTEGER, channel_affinity_failback_mode_override TEXT, channel_affinity_failback_delay_seconds_override INTEGER, channel_proxy_url TEXT, channel_extra_headers TEXT, channel_session_affinity_auto INTEGER, channel_allow_missing_usage INTEGER NOT NULL DEFAULT 0, transforms TEXT NOT NULL DEFAULT '[]', api_type_overrides TEXT NOT NULL DEFAULT '[]', active_probe_enabled_override INTEGER, active_probe_interval_seconds_override INTEGER, active_probe_success_threshold_override INTEGER, active_probe_model_override TEXT, request_timeout_ms_override INTEGER, extra_fields_whitelist TEXT, strip_cross_protocol_nested_extra INTEGER, circuit_breaker_enabled INTEGER NOT NULL DEFAULT 1, per_model_circuit_break INTEGER NOT NULL DEFAULT 0, channel_retry_interval_ms INTEGER NOT NULL DEFAULT 0)"),
        format!("CREATE TABLE monoize_provider_models (provider_id TEXT NOT NULL, model_name TEXT NOT NULL, model_name_key {suffix} NOT NULL CHECK(model_name_key = {key_cast}), model_search_key {suffix} NOT NULL CHECK(model_search_key = {model_search_check}), redirect TEXT, pricing_profile_mode TEXT NOT NULL CHECK(pricing_profile_mode IN ('inherit','override','unpriced')), pricing_profile_override TEXT, multiplier_override TEXT, created_at TEXT NOT NULL, PRIMARY KEY(provider_id, model_name_key), FOREIGN KEY(provider_id) REFERENCES monoize_providers(id) ON DELETE CASCADE)"),
        "CREATE INDEX idx_monoize_provider_route ON monoize_providers(group_id, priority, created_at, id)".to_string(),
        "CREATE INDEX idx_monoize_provider_models_lookup ON monoize_provider_models(model_name_key, provider_id)".to_string(),
        "CREATE UNIQUE INDEX uq_monoize_provider_public_name ON monoize_providers(group_id, public_name_key)".to_string(),
        "CREATE UNIQUE INDEX uq_monoize_channel_public_name ON monoize_providers(group_id, channel_public_name_key)".to_string(),
    ];
    for sql in statements {
        tx.execute(Statement::from_string(backend, sql.to_string()))
            .await?;
    }

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
        );
        let groups = if groups.is_empty() {
            vec![default_group(tx, backend).await?]
        } else {
            groups
        };
        let channels = tx.query_all(Statement::from_sql_and_values(
            backend,
            numbered(backend, "SELECT id, name, provider_type, base_url, api_key, weight, enabled, created_at, passive_failure_count_threshold_override, passive_cooldown_seconds_override, passive_window_seconds_override, passive_rate_limit_cooldown_seconds_override, active_probe_enabled_override, active_probe_interval_seconds_override, active_probe_success_threshold_override, active_probe_model_override, affinity_enabled_override, affinity_idle_ttl_seconds_override, affinity_failback_mode_override, affinity_failback_delay_seconds_override, proxy_url, extra_headers, session_affinity_auto, allow_missing_usage FROM monoize_channels_legacy_flatten WHERE provider_id = ? ORDER BY created_at, id"), vec![old_id.clone().into()],
        )).await?;
        if channels.is_empty() {
            return Err(DbErr::Custom(format!("provider {old_id} has no channel")));
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
                    group_id,
                    &provider_id,
                    &channel_id,
                    &target_name,
                    &channel_name,
                    channel,
                    target_priority,
                    &now,
                )
                .await?;
                let models = tx.query_all(Statement::from_sql_and_values(backend, numbered(backend, "SELECT model_name, redirect, multiplier, created_at FROM monoize_channel_models_legacy_flatten WHERE channel_id = ? ORDER BY model_name"), vec![legacy_channel_id.into()])).await?;
                for model in models {
                    let model_name: String = model.try_get("", "model_name")?;
                    let redirect: Option<String> = model.try_get("", "redirect")?;
                    let multiplier: String = model
                        .try_get::<String>("", "multiplier")
                        .unwrap_or_else(|_| "1".to_string());
                    let created_at: String = model
                        .try_get::<String>("", "created_at")
                        .unwrap_or_else(|_| now.clone());
                    tx.execute(Statement::from_sql_and_values(backend, numbered(backend, "INSERT INTO monoize_provider_models (provider_id, model_name, model_name_key, model_search_key, redirect, pricing_profile_mode, pricing_profile_override, multiplier_override, created_at) VALUES (?,?,?,?,?,'override',NULL,?,?)"), vec![provider_id.clone().into(), model_name.clone().into(), bytes(backend, model_name.as_bytes()), bytes(backend, model_name.to_ascii_lowercase().as_bytes()), redirect.into(), multiplier.into(), created_at.into()])).await?;
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

async fn insert_provider(
    tx: &DatabaseTransaction,
    backend: DbBackend,
    row: &sea_orm::QueryResult,
    group_id: &str,
    provider_id: &str,
    channel_id: &str,
    name: &str,
    channel_name: &str,
    channel: &sea_orm::QueryResult,
    priority: i32,
    now: &str,
) -> Result<(), DbErr> {
    let value = |column: &str| row.try_get::<String>("", column).unwrap_or_default();
    let opt = |column: &str| row.try_get::<Option<i32>>("", column).unwrap_or(None);
    let ch_opt = |column: &str| channel.try_get::<Option<i32>>("", column).unwrap_or(None);
    let sql = numbered(
        backend,
        "INSERT INTO monoize_providers (id,group_id,name,public_name,public_name_key,priority,enabled,multiplier,configuration_generation,created_at,updated_at,channel_id,channel_name,channel_public_name,channel_public_name_key,channel_provider_type,channel_base_url,channel_api_key,channel_enabled,channel_max_retries,channel_passive_failure_count_threshold_override,channel_passive_cooldown_seconds_override,channel_passive_window_seconds_override,channel_passive_rate_limit_cooldown_seconds_override,channel_active_probe_enabled_override,channel_active_probe_interval_seconds_override,channel_active_probe_success_threshold_override,channel_active_probe_model_override,channel_affinity_enabled_override,channel_affinity_idle_ttl_seconds_override,channel_affinity_failback_mode_override,channel_affinity_failback_delay_seconds_override,channel_proxy_url,channel_extra_headers,channel_session_affinity_auto,channel_allow_missing_usage,transforms,api_type_overrides,active_probe_enabled_override,active_probe_interval_seconds_override,active_probe_success_threshold_override,active_probe_model_override,request_timeout_ms_override,extra_fields_whitelist,strip_cross_protocol_nested_extra,circuit_breaker_enabled,per_model_circuit_break,channel_retry_interval_ms) VALUES (?,?,?,?,?,?,?,'1',0,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    );
    let public_name = name.trim().to_string();
    let channel_public_name = channel_name.trim().to_string();
    let mut values: Vec<Value> = vec![
        provider_id.into(),
        group_id.into(),
        name.into(),
        public_name.clone().into(),
        bytes(backend, public_name.as_bytes()),
        priority.into(),
        row.try_get::<i32>("", "enabled")?.into(),
        now.into(),
        now.into(),
        channel_id.into(),
        channel_name.into(),
        channel_public_name.clone().into(),
        bytes(backend, channel_public_name.as_bytes()),
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

fn decode_groups(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

async fn default_group(tx: &DatabaseTransaction, backend: DbBackend) -> Result<String, DbErr> {
    tx.query_one(Statement::from_string(
        backend,
        "SELECT id FROM monoize_groups WHERE is_default = 1 LIMIT 1".to_string(),
    ))
    .await?
    .ok_or_else(|| DbErr::Custom("default group row missing".to_string()))
    .and_then(|row| row.try_get("", "id"))
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

fn bytes(backend: DbBackend, value: &[u8]) -> Value {
    if backend == DbBackend::Postgres {
        Value::Bytes(Some(Box::new(value.to_vec())))
    } else {
        Value::Bytes(Some(Box::new(value.to_vec())))
    }
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

pub(crate) fn target_provider_ddl() -> Vec<String> {
    vec![
        "CREATE TABLE monoize_provider_models (...)".to_string(),
        "CREATE TABLE monoize_providers (... channel_id TEXT NOT NULL UNIQUE ...)".to_string(),
    ]
}
