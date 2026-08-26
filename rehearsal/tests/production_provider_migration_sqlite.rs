#[path = "../../src/migration/m20260826_000044_provider_pricing_flatten.rs"]
mod migration_under_test;

use migration_under_test::Migration;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::{MigrationTrait, SchemaManager};

async fn execute(db: &DatabaseConnection, sql: &str) {
    db.execute(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .await
        .unwrap();
}

async fn legacy_database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    execute(&db, "PRAGMA foreign_keys = ON").await;
    execute(
        &db,
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, is_default INTEGER NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO monoize_groups (id, is_default) VALUES ('group-one', 1)",
    )
    .await;
    execute(
        &db,
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, name TEXT NOT NULL, max_retries INTEGER NOT NULL, channel_max_retries INTEGER NOT NULL, channel_retry_interval_ms INTEGER NOT NULL, circuit_breaker_enabled INTEGER NOT NULL, per_model_circuit_break INTEGER NOT NULL, transforms TEXT NOT NULL, api_type_overrides TEXT NOT NULL, active_probe_enabled_override INTEGER, active_probe_interval_seconds_override INTEGER, active_probe_success_threshold_override INTEGER, active_probe_model_override TEXT, request_timeout_ms_override INTEGER, extra_fields_whitelist TEXT, strip_cross_protocol_nested_extra INTEGER, group_ids TEXT NOT NULL, enabled INTEGER NOT NULL, priority INTEGER NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO monoize_providers VALUES ('provider-one', 'Provider One', 2, 3, 100, 1, 0, '[]', '[]', NULL, NULL, NULL, NULL, NULL, NULL, NULL, '[\"group-one\"]', 1, 7, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .await;
    execute(
        &db,
        "CREATE TABLE monoize_channels (id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, name TEXT NOT NULL, provider_type TEXT NOT NULL, base_url TEXT NOT NULL, api_key TEXT NOT NULL, weight INTEGER NOT NULL, enabled INTEGER NOT NULL, passive_failure_count_threshold_override INTEGER, passive_cooldown_seconds_override INTEGER, passive_window_seconds_override INTEGER, passive_rate_limit_cooldown_seconds_override INTEGER, active_probe_enabled_override INTEGER, active_probe_interval_seconds_override INTEGER, active_probe_success_threshold_override INTEGER, active_probe_model_override TEXT, affinity_enabled_override INTEGER, affinity_idle_ttl_seconds_override INTEGER, affinity_failback_mode_override TEXT, affinity_failback_delay_seconds_override INTEGER, proxy_url TEXT, extra_headers TEXT, session_affinity_auto INTEGER, allow_missing_usage INTEGER NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO monoize_channels VALUES ('channel-one', 'provider-one', 'Channel One', 'openai', 'https://example.invalid', 'secret', 1, 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0)",
    )
    .await;
    execute(
        &db,
        "CREATE TABLE monoize_channel_models (channel_id TEXT NOT NULL, model_name TEXT NOT NULL, redirect TEXT, multiplier TEXT NOT NULL, created_at TEXT NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO monoize_channel_models VALUES ('channel-one', 'gpt-4o', NULL, '1.25', '2026-01-01T00:00:00Z')",
    )
    .await;
    db
}

#[tokio::test]
async fn production_migration_removes_obsolete_provider_retry_column() {
    let db = legacy_database().await;
    Migration.up(&SchemaManager::new(&db)).await.unwrap();

    let columns = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA table_info(monoize_providers)".to_owned(),
        ))
        .await
        .unwrap();
    let names = columns
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").unwrap())
        .collect::<Vec<_>>();

    assert!(names.iter().any(|name| name == "channel_id"));
    assert!(!names.iter().any(|name| name == "max_retries"));
}

#[tokio::test]
async fn production_migration_is_a_no_op_after_success() {
    let db = legacy_database().await;
    let manager = SchemaManager::new(&db);

    Migration.up(&manager).await.unwrap();
    Migration.up(&manager).await.unwrap();

    let provider_count: i64 = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM monoize_providers".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "count")
        .unwrap();
    assert_eq!(provider_count, 1);
}
