use super::{LegacyProvider, ModelKeys, PricingMode, transform_provider};
use anyhow::{Context, bail};
use sqlx::{Connection, Executor, Row, Sqlite, SqliteConnection, Transaction};
use std::collections::BTreeMap;

const TARGET_DDL: &[&str] = &[
    r#"CREATE TABLE IF NOT EXISTS monoize_providers (
        id TEXT PRIMARY KEY,
        group_id TEXT NOT NULL REFERENCES monoize_groups(id) ON DELETE RESTRICT,
        name TEXT NOT NULL,
        public_name TEXT NOT NULL,
        public_name_key BLOB NOT NULL CHECK(public_name_key = CAST(public_name AS BLOB)),
        priority INTEGER NOT NULL,
        enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
        pricing_profile TEXT NULL,
        multiplier TEXT NOT NULL,
        configuration_generation INTEGER NOT NULL CHECK(configuration_generation >= 1),
        created_at TEXT NOT NULL,
        channel_id TEXT NOT NULL UNIQUE,
        channel_name TEXT NOT NULL,
        channel_public_name TEXT NOT NULL,
        channel_public_name_key BLOB NOT NULL CHECK(channel_public_name_key = CAST(channel_public_name AS BLOB)),
        channel_provider_type TEXT NOT NULL,
        channel_base_url TEXT NOT NULL,
        channel_api_key TEXT NOT NULL,
        channel_enabled INTEGER NOT NULL CHECK(channel_enabled IN (0, 1)),
        channel_max_retries INTEGER NOT NULL CHECK(channel_max_retries >= 0)
    )"#,
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_provider_public_name ON monoize_providers(group_id, public_name_key)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_channel_public_name ON monoize_providers(group_id, channel_public_name_key)",
    "CREATE INDEX IF NOT EXISTS idx_provider_route ON monoize_providers(group_id, priority, created_at, id)",
    r#"CREATE TABLE IF NOT EXISTS monoize_provider_models (
        provider_id TEXT NOT NULL REFERENCES monoize_providers(id) ON DELETE CASCADE,
        model_name TEXT NOT NULL,
        model_name_key BLOB NOT NULL CHECK(model_name_key = CAST(model_name AS BLOB)),
        model_search_key BLOB NOT NULL,
        redirect TEXT NULL,
        pricing_profile_mode TEXT NOT NULL CHECK(pricing_profile_mode IN ('inherit', 'override', 'unpriced')),
        pricing_profile_override TEXT NULL,
        multiplier_override TEXT NULL,
        created_at TEXT NOT NULL,
        PRIMARY KEY(provider_id, model_name_key),
        CHECK((pricing_profile_mode = 'override' AND pricing_profile_override IS NOT NULL AND length(trim(pricing_profile_override)) > 0)
           OR (pricing_profile_mode IN ('inherit', 'unpriced') AND pricing_profile_override IS NULL))
    )"#,
    "CREATE INDEX IF NOT EXISTS idx_provider_models_lookup ON monoize_provider_models(model_name_key, provider_id)",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationFailurePoint {
    AfterLegacyRename,
    AfterTargetSchema,
    AfterProviders,
    BeforeLegacyDrop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationOutcome {
    Migrated {
        provider_count: usize,
        model_count: usize,
    },
    AlreadyMigrated,
}

pub async fn create_sqlite_target_schema(db: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let mut transaction = db.begin().await?;
    execute_target_ddl(&mut transaction).await?;
    transaction.commit().await
}

pub async fn migrate_sqlite_provider_schema(
    db: &mut SqliteConnection,
    sources: &[LegacyProvider],
    failure: Option<MigrationFailurePoint>,
) -> anyhow::Result<MigrationOutcome> {
    let has_provider_models = sqlite_table_exists(db, "monoize_provider_models").await?;
    let has_channels = sqlite_table_exists(db, "monoize_channels").await?;
    let has_channel_models = sqlite_table_exists(db, "monoize_channel_models").await?;
    if has_provider_models && !has_channels && !has_channel_models {
        return Ok(MigrationOutcome::AlreadyMigrated);
    }
    if !has_channels || !has_channel_models || !sqlite_table_exists(db, "monoize_providers").await?
    {
        bail!("legacy_provider_schema_incomplete");
    }

    let transformed = sources
        .iter()
        .map(transform_provider)
        .collect::<Result<Vec<_>, _>>()
        .context("transform legacy Providers")?;
    let provider_count = transformed.iter().map(|result| result.targets.len()).sum();
    let model_count = transformed
        .iter()
        .flat_map(|result| &result.targets)
        .map(|target| target.models.len())
        .sum();

    let mut transaction = db.begin().await?;
    transaction
        .execute("ALTER TABLE monoize_providers RENAME TO legacy_monoize_providers")
        .await?;
    fail_at(failure, MigrationFailurePoint::AfterLegacyRename)?;
    execute_target_ddl(&mut transaction).await?;
    fail_at(failure, MigrationFailurePoint::AfterTargetSchema)?;

    let mut targets = transformed
        .into_iter()
        .flat_map(|result| result.targets)
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| {
        left.group_id
            .as_bytes()
            .cmp(right.group_id.as_bytes())
            .then_with(|| left.priority.cmp(&right.priority))
            .then_with(|| {
                left.source_provider_id
                    .as_bytes()
                    .cmp(right.source_provider_id.as_bytes())
            })
            .then_with(|| {
                left.channel
                    .source_channel_id
                    .as_bytes()
                    .cmp(right.channel.source_channel_id.as_bytes())
            })
    });
    let mut next_priority = BTreeMap::<String, i32>::new();
    for mut target in targets {
        let priority = next_priority.entry(target.group_id.clone()).or_default();
        target.priority = *priority;
        *priority = priority.checked_add(1).context("group priority overflow")?;
        let provider_public_name = format!(
            "{} / {}",
            source_name(sources, &target.source_provider_id),
            target.channel.name
        );
        let provider_key = provider_public_name.as_bytes();
        let channel_public_name = target.channel.name.clone();
        let channel_key = channel_public_name.as_bytes();
        sqlx::query(
            r#"INSERT INTO monoize_providers (
                    id, group_id, name, public_name, public_name_key, priority, enabled,
                    pricing_profile, multiplier, configuration_generation, created_at,
                    channel_id, channel_name, channel_public_name, channel_public_name_key,
                    channel_provider_type, channel_base_url, channel_api_key, channel_enabled,
                    channel_max_retries
                ) VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?, 1, ?, ?, ?, ?, ?, 'responses',
                          'https://redacted.invalid', 'redacted', ?, 0)"#,
        )
        .bind(&target.id)
        .bind(&target.group_id)
        .bind(source_name(sources, &target.source_provider_id))
        .bind(&provider_public_name)
        .bind(provider_key)
        .bind(target.priority)
        .bind(&target.pricing_profile)
        .bind(target.multiplier.as_str())
        .bind("2026-08-26T00:00:00Z")
        .bind(&target.channel.id)
        .bind(&target.channel.name)
        .bind(&channel_public_name)
        .bind(channel_key)
        .bind(target.channel.enabled)
        .execute(&mut *transaction)
        .await?;

        for model in target.models {
            let keys = ModelKeys::new(&model.name).context("derive model keys")?;
            let (mode, profile) = match model.pricing {
                PricingMode::Inherit => ("inherit", None),
                PricingMode::Override(profile) => ("override", Some(profile)),
                PricingMode::Unpriced => ("unpriced", None),
            };
            sqlx::query(
                r#"INSERT INTO monoize_provider_models (
                        provider_id, model_name, model_name_key, model_search_key, redirect,
                        pricing_profile_mode, pricing_profile_override, multiplier_override,
                        created_at
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, '2026-08-26T00:00:00Z')"#,
            )
            .bind(&target.id)
            .bind(&keys.model_name)
            .bind(keys.name)
            .bind(keys.search)
            .bind(&model.redirect)
            .bind(mode)
            .bind(profile)
            .bind(
                model
                    .multiplier_override
                    .as_ref()
                    .map(|value| value.as_str()),
            )
            .execute(&mut *transaction)
            .await?;
        }
    }
    fail_at(failure, MigrationFailurePoint::AfterProviders)?;
    fail_at(failure, MigrationFailurePoint::BeforeLegacyDrop)?;
    transaction
        .execute("DROP TABLE monoize_channel_models")
        .await?;
    transaction.execute("DROP TABLE monoize_channels").await?;
    transaction
        .execute("DROP TABLE legacy_monoize_providers")
        .await?;
    transaction.commit().await?;
    Ok(MigrationOutcome::Migrated {
        provider_count,
        model_count,
    })
}

async fn execute_target_ddl(transaction: &mut Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    for statement in target_ddl() {
        transaction.execute(statement.as_str()).await?;
    }
    Ok(())
}

fn target_ddl() -> Vec<String> {
    let search_check = format!(
        "model_search_key BLOB NOT NULL CHECK(model_search_key = CAST({} AS BLOB)),",
        ascii_fold_expression("model_name")
    );
    TARGET_DDL
        .iter()
        .map(|statement| {
            statement
                .replace("model_search_key BLOB NOT NULL,", search_check.as_str())
                .replace(
                    "multiplier TEXT NOT NULL,",
                    &format!(
                        "multiplier TEXT NOT NULL CHECK({}),",
                        canonical_decimal_check("multiplier")
                    ),
                )
                .replace(
                    "multiplier_override TEXT NULL,",
                    &format!(
                        "multiplier_override TEXT NULL CHECK(multiplier_override IS NULL OR ({})),",
                        canonical_decimal_check("multiplier_override")
                    ),
                )
        })
        .collect()
}

fn canonical_decimal_check(column: &str) -> String {
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

fn ascii_fold_expression(column: &str) -> String {
    (b'A'..=b'Z').fold(column.to_owned(), |expression, upper| {
        format!(
            "replace({expression}, '{}', '{}')",
            char::from(upper),
            char::from(upper + 32)
        )
    })
}

pub(super) fn fail_at(
    selected: Option<MigrationFailurePoint>,
    current: MigrationFailurePoint,
) -> anyhow::Result<()> {
    if selected == Some(current) {
        bail!("injected_migration_failure:{current:?}");
    }
    Ok(())
}

pub(super) fn source_name<'a>(sources: &'a [LegacyProvider], provider_id: &str) -> &'a str {
    sources
        .iter()
        .find(|source| source.id == provider_id)
        .map_or("migrated Provider", |source| source.name.as_str())
}

pub async fn sqlite_table_exists(
    db: &mut SqliteConnection,
    table: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS count FROM sqlite_master WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(db)
    .await?;
    Ok(row.try_get::<i64, _>("count")? == 1)
}
