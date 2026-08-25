use sqlx::{Connection, Executor, Row, SqliteConnection};

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

pub async fn create_sqlite_target_schema(db: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let mut transaction = db.begin().await?;
    for statement in TARGET_DDL {
        transaction.execute(*statement).await?;
    }
    transaction.commit().await
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
