use super::{LegacyProvider, ModelKeys, PricingMode, transform_provider};
use anyhow::{Context, bail};
use sqlx::{Connection, Executor, Row, Sqlite, SqliteConnection, Transaction};

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
        CHECK((pricing_profile_mode = 'override') = (pricing_profile_override IS NOT NULL))
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

    for result in transformed {
        for target in result.targets {
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
    for statement in TARGET_DDL {
        transaction.execute(*statement).await?;
    }
    Ok(())
}

fn fail_at(
    selected: Option<MigrationFailurePoint>,
    current: MigrationFailurePoint,
) -> anyhow::Result<()> {
    if selected == Some(current) {
        bail!("injected_migration_failure:{current:?}");
    }
    Ok(())
}

fn source_name<'a>(sources: &'a [LegacyProvider], provider_id: &str) -> &'a str {
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
