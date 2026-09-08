#[path = "../../src/migration/m20260826_000048_provider_pricing_flatten.rs"]
mod migration_under_test;

use migration_under_test::Migration;
use monoize_lynshen_rehearsal::provider::deterministic_id;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::{MigrationTrait, SchemaManager};
use serde_json::json;

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
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, name TEXT NOT NULL, is_default INTEGER NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO monoize_groups (id, name, is_default) VALUES ('group-one', 'Group One', 1)",
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
        "CREATE TABLE monoize_channels (id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, name TEXT NOT NULL, provider_type TEXT NOT NULL, base_url TEXT NOT NULL, api_key TEXT NOT NULL, weight INTEGER NOT NULL, enabled INTEGER NOT NULL, created_at TEXT NOT NULL, passive_failure_count_threshold_override INTEGER, passive_cooldown_seconds_override INTEGER, passive_window_seconds_override INTEGER, passive_rate_limit_cooldown_seconds_override INTEGER, active_probe_enabled_override INTEGER, active_probe_interval_seconds_override INTEGER, active_probe_success_threshold_override INTEGER, active_probe_model_override TEXT, affinity_enabled_override INTEGER, affinity_idle_ttl_seconds_override INTEGER, affinity_failback_mode_override TEXT, affinity_failback_delay_seconds_override INTEGER, proxy_url TEXT, extra_headers TEXT, session_affinity_auto INTEGER, allow_missing_usage INTEGER NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO monoize_channels VALUES ('channel-z', 'provider-one', 'Channel Z', 'openai', 'https://example.invalid', 'secret', 1, 1, '2026-01-01T00:00:00Z', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0)",
    )
    .await;
    execute(
        &db,
        "CREATE TABLE monoize_channel_models (channel_id TEXT NOT NULL, model_name TEXT NOT NULL, redirect TEXT, multiplier TEXT NOT NULL, created_at TEXT NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO monoize_channel_models VALUES ('channel-z', 'gpt-4o', NULL, '1.25', '2026-01-01T00:00:00Z')",
    )
    .await;
    execute(
        &db,
        "CREATE TABLE system_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO system_settings VALUES ('reasoning_suffix_map', '{}', '2026-01-01T00:00:00Z'), ('pricing_profile_model_patterns', '[{\"pattern\":\"gpt-*\",\"pricing_profile\":\"openai\"}]', '2026-01-01T00:00:00Z')",
    )
    .await;
    execute(
        &db,
        "CREATE TABLE model_metadata_records (model_id TEXT PRIMARY KEY, models_dev_provider TEXT)",
    )
    .await;
    execute(
        &db,
        "CREATE TABLE billing_rate_records (id TEXT PRIMARY KEY, pricing_profile TEXT NOT NULL, model_pattern TEXT, provider_type TEXT, rate_kind TEXT NOT NULL, usage_class TEXT NOT NULL, unit TEXT NOT NULL, unit_price_nano_usd TEXT NOT NULL, context_tier TEXT, service_tier TEXT, modality TEXT, cache_ttl TEXT, match_json TEXT NOT NULL, priority INTEGER NOT NULL, enabled INTEGER NOT NULL)",
    )
    .await;
    execute(
        &db,
        "INSERT INTO billing_rate_records VALUES ('openai-gpt-input', 'openai', 'gpt-*', 'openai', 'token', 'input_uncached', 'token', '1000', NULL, NULL, NULL, NULL, '{}', 0, 1), ('openai-gpt-output', 'openai', 'gpt-*', 'openai', 'token', 'output', 'token', '2000', NULL, NULL, NULL, NULL, '{}', 0, 1)",
    )
    .await;
    execute(
        &db,
        "CREATE TABLE state_records (tenant_id TEXT NOT NULL, kind TEXT NOT NULL, id TEXT NOT NULL, value TEXT NOT NULL, expires_at INTEGER, PRIMARY KEY (tenant_id, kind, id))",
    )
    .await;
    db
}

fn lower_hex(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn approve_migration(db: &DatabaseConnection) {
    let fingerprint = migration_under_test::migration_source_fingerprint(
        db,
        DbBackend::Sqlite,
        b"monoize-provider-migration-test-key",
    )
    .await
    .unwrap();
    let group_rows = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id, name FROM monoize_groups ORDER BY id".to_owned(),
        ))
        .await
        .unwrap();
    let groups = group_rows
        .into_iter()
        .map(|row| {
            let id = row.try_get::<String>("", "id").unwrap();
            let public_name = format!("Public {}", row.try_get::<String>("", "name").unwrap());
            json!({
                "source_group_id": id,
                "public_name": public_name,
                "public_name_key_hex": lower_hex(&public_name),
            })
        })
        .collect::<Vec<_>>();
    let provider_rows = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id, group_ids FROM monoize_providers ORDER BY id".to_owned(),
        ))
        .await
        .unwrap();
    let mut targets = Vec::new();
    let mut semantic_changes = Vec::new();
    for provider in provider_rows {
        let provider_id = provider.try_get::<String>("", "id").unwrap();
        let group_ids = provider
            .try_get::<String>("", "group_ids")
            .ok()
            .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
            .unwrap_or_default();
        let channels = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                format!(
                    "SELECT id, enabled, weight FROM monoize_channels WHERE provider_id = '{provider_id}' ORDER BY created_at, id"
                ),
            ))
            .await
            .unwrap();
        let positive_enabled = channels
            .iter()
            .filter(|row| {
                row.try_get::<i32>("", "enabled").unwrap() != 0
                    && row.try_get::<i32>("", "weight").unwrap() > 0
            })
            .count();
        if group_ids.len() > 1 || positive_enabled > 1 {
            semantic_changes.push(provider_id.clone());
        }
        for (group_index, group_id) in group_ids.iter().enumerate() {
            for (channel_index, channel) in channels.iter().enumerate() {
                let source_channel_id = channel.try_get::<String>("", "id").unwrap();
                let target_provider_id = if group_index == 0 && channel_index == 0 {
                    provider_id.clone()
                } else {
                    deterministic_id("provider", &provider_id, group_id, &source_channel_id)
                };
                let target_channel_id = if group_index == 0 {
                    source_channel_id.clone()
                } else {
                    deterministic_id("channel", &provider_id, group_id, &source_channel_id)
                };
                let provider_public_name = format!("Public {target_provider_id}");
                let channel_public_name = format!("Public {target_channel_id}");
                targets.push(json!({
                    "source_provider_id": provider_id,
                    "source_channel_id": source_channel_id,
                    "target_group_id": group_id,
                    "target_provider_id": target_provider_id,
                    "target_channel_id": target_channel_id,
                    "provider_public_name": provider_public_name,
                    "provider_public_name_key_hex": lower_hex(&provider_public_name),
                    "channel_public_name": channel_public_name,
                    "channel_public_name_key_hex": lower_hex(&channel_public_name),
                }));
            }
        }
    }
    let manifest = json!({
        "schema_version": 1,
        "source_fingerprint": fingerprint,
        "approved_semantic_change_source_provider_ids": semantic_changes,
        "groups": groups,
        "targets": targets,
    })
    .to_string();
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "INSERT INTO state_records (tenant_id, kind, id, value, expires_at) VALUES (?, ?, ?, ?, NULL)".to_owned(),
        vec![
            "system".into(),
            "provider_pricing_migration".into(),
            "approved_manifest_v1".into(),
            manifest.into(),
        ],
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn production_migration_requires_an_approved_public_name_manifest() {
    let db = legacy_database().await;

    let error = Migration
        .up(&SchemaManager::new(&db))
        .await
        .expect_err("migration without an approved manifest must fail");

    assert!(error.to_string().contains("approved migration manifest"));
    let columns = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA table_info(monoize_groups)".to_owned(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").unwrap())
        .collect::<Vec<_>>();
    assert!(!columns.iter().any(|name| name == "public_name"));
}

#[tokio::test]
async fn production_migration_rejects_malformed_and_empty_group_ids() {
    for invalid in ["not-json", "[]"] {
        let db = legacy_database().await;
        db.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE monoize_providers SET group_ids = ? WHERE id = 'provider-one'".to_owned(),
            vec![invalid.into()],
        ))
        .await
        .unwrap();
        approve_migration(&db).await;

        let error = Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect_err("invalid Group membership must fail migration");

        let message = error.to_string();
        assert!(
            message.contains("malformed group_ids") || message.contains("zero Groups"),
            "unexpected migration error: {message}"
        );
        assert!(
            db.query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT 1 FROM monoize_channels LIMIT 1".to_owned(),
            ))
            .await
            .is_ok(),
            "legacy schema was not rolled back"
        );
    }
}

#[tokio::test]
async fn production_migration_rejects_a_stale_approved_fingerprint() {
    let db = legacy_database().await;
    approve_migration(&db).await;
    execute(
        &db,
        "UPDATE monoize_providers SET enabled = 0 WHERE id = 'provider-one'",
    )
    .await;

    let error = Migration
        .up(&SchemaManager::new(&db))
        .await
        .expect_err("source change after approval must fail migration");

    assert!(error.to_string().contains("fingerprint does not match"));
}

#[tokio::test]
async fn production_migration_uses_only_manifest_public_names() {
    let db = legacy_database().await;
    approve_migration(&db).await;

    Migration.up(&SchemaManager::new(&db)).await.unwrap();

    let group = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT public_name FROM monoize_groups WHERE id = 'group-one'".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "public_name")
        .unwrap();
    let provider = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT public_name, channel_public_name FROM monoize_providers WHERE id = 'provider-one'"
                .to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group, "Public Group One");
    assert_eq!(
        provider.try_get::<String>("", "public_name").unwrap(),
        "Public provider-one"
    );
    assert_eq!(
        provider
            .try_get::<String>("", "channel_public_name")
            .unwrap(),
        "Public channel-z"
    );
}

#[tokio::test]
async fn production_migration_preserves_selected_incomplete_profile() {
    let db = legacy_database().await;
    execute(
        &db,
        "DELETE FROM billing_rate_records WHERE usage_class = 'output'",
    )
    .await;
    approve_migration(&db).await;

    Migration.up(&SchemaManager::new(&db)).await.unwrap();

    let provider = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT pricing_profile FROM monoize_providers WHERE id = 'provider-one'".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    let model = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT pricing_profile_mode FROM monoize_provider_models WHERE provider_id = 'provider-one' AND model_name = 'gpt-4o'".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        provider
            .try_get::<Option<String>>("", "pricing_profile")
            .unwrap()
            .as_deref(),
        Some("openai")
    );
    assert_eq!(
        model.try_get::<String>("", "pricing_profile_mode").unwrap(),
        "inherit"
    );
}

#[tokio::test]
async fn production_migration_preserves_profile_without_current_model_rates() {
    let db = legacy_database().await;
    execute(
        &db,
        "UPDATE billing_rate_records SET model_pattern = 'other-*'",
    )
    .await;
    approve_migration(&db).await;

    Migration.up(&SchemaManager::new(&db)).await.unwrap();

    let provider = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT pricing_profile FROM monoize_providers WHERE id = 'provider-one'".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        provider
            .try_get::<Option<String>>("", "pricing_profile")
            .unwrap()
            .as_deref(),
        Some("openai")
    );
}

#[tokio::test]
async fn production_migration_removes_obsolete_provider_retry_column() {
    let db = legacy_database().await;
    approve_migration(&db).await;
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
    approve_migration(&db).await;
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

#[tokio::test]
async fn production_migration_preserves_effective_profile_and_multiplier() {
    let db = legacy_database().await;
    approve_migration(&db).await;

    Migration.up(&SchemaManager::new(&db)).await.unwrap();

    let provider = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT pricing_profile, multiplier FROM monoize_providers WHERE id = 'provider-one'"
                .to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        provider
            .try_get::<Option<String>>("", "pricing_profile")
            .unwrap()
            .as_deref(),
        Some("openai")
    );
    assert_eq!(
        provider.try_get::<String>("", "multiplier").unwrap(),
        "1.25"
    );

    let model = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT pricing_profile_mode, pricing_profile_override, multiplier_override FROM monoize_provider_models WHERE provider_id = 'provider-one' AND model_name = 'gpt-4o'".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        model.try_get::<String>("", "pricing_profile_mode").unwrap(),
        "inherit"
    );
    assert_eq!(
        model
            .try_get::<Option<String>>("", "pricing_profile_override")
            .unwrap(),
        None
    );
    assert_eq!(
        model
            .try_get::<Option<String>>("", "multiplier_override")
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn production_migration_enforces_pricing_mode_and_multiplier_checks() {
    let db = legacy_database().await;
    approve_migration(&db).await;
    Migration.up(&SchemaManager::new(&db)).await.unwrap();

    for invalid in ["0", "01", "1.0", "1.0000000001"] {
        assert!(
            db.execute(Statement::from_string(
                DbBackend::Sqlite,
                format!(
                    "UPDATE monoize_providers SET multiplier = '{invalid}' WHERE id = 'provider-one'"
                ),
            ))
            .await
            .is_err(),
            "accepted invalid Provider multiplier {invalid}"
        );
    }

    let invalid_model_rows = [
        "VALUES ('provider-one','bad-a',CAST('bad-a' AS BLOB),CAST('bad-a' AS BLOB),NULL,'override',NULL,NULL,'2026-01-01T00:00:00Z')",
        "VALUES ('provider-one','bad-b',CAST('bad-b' AS BLOB),CAST('bad-b' AS BLOB),NULL,'inherit','openai',NULL,'2026-01-01T00:00:00Z')",
        "VALUES ('provider-one','bad-c',CAST('bad-c' AS BLOB),CAST('bad-c' AS BLOB),NULL,'inherit',NULL,'0','2026-01-01T00:00:00Z')",
    ];
    for values in invalid_model_rows {
        assert!(
            db.execute(Statement::from_string(
                DbBackend::Sqlite,
                format!(
                    "INSERT INTO monoize_provider_models (provider_id,model_name,model_name_key,model_search_key,redirect,pricing_profile_mode,pricing_profile_override,multiplier_override,created_at) {values}"
                ),
            ))
            .await
            .is_err(),
            "accepted invalid model pricing row: {values}"
        );
    }
}

#[tokio::test]
async fn production_migration_adds_unique_group_public_name_keys() {
    let db = legacy_database().await;
    approve_migration(&db).await;
    Migration.up(&SchemaManager::new(&db)).await.unwrap();

    let columns = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA table_info(monoize_groups)".to_owned(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").unwrap())
        .collect::<Vec<_>>();
    assert!(columns.iter().any(|name| name == "public_name"));
    assert!(columns.iter().any(|name| name == "public_name_key"));

    assert!(
        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO monoize_groups (id,name,is_default,public_name,public_name_key) VALUES ('group-duplicate','Other',0,'Public Group One',CAST('Public Group One' AS BLOB))".to_owned(),
        ))
        .await
        .is_err()
    );

    for invalid_write in [
        "UPDATE monoize_groups SET public_name = NULL WHERE id = 'group-one'",
        "UPDATE monoize_groups SET public_name_key = NULL WHERE id = 'group-one'",
        "UPDATE monoize_groups SET public_name_key = CAST('Mismatch' AS BLOB) WHERE id = 'group-one'",
        "INSERT INTO monoize_groups (id,name,is_default,public_name,public_name_key) VALUES ('group-null','Null',0,NULL,NULL)",
        "INSERT INTO monoize_groups (id,name,is_default,public_name,public_name_key) VALUES ('group-mismatch','Mismatch',0,'Public',CAST('Wrong' AS BLOB))",
    ] {
        assert!(
            db.execute(Statement::from_string(
                DbBackend::Sqlite,
                invalid_write.to_owned(),
            ))
            .await
            .is_err(),
            "accepted invalid Group public-name write: {invalid_write}"
        );
    }
}

#[tokio::test]
async fn production_migration_preserves_pair_order_and_derives_target_routing_fields() {
    let db = legacy_database().await;
    execute(
        &db,
        "INSERT INTO monoize_groups (id, name, is_default) VALUES ('group-two', 'Group Two', 0)",
    )
    .await;
    execute(
        &db,
        "UPDATE monoize_providers SET group_ids = '[\"group-two\",\"group-one\"]' WHERE id = 'provider-one'",
    )
    .await;
    execute(
        &db,
        "INSERT INTO monoize_channels VALUES ('channel-a', 'provider-one', 'Channel A', 'openai', 'https://example.invalid', 'secret', 0, 1, '2026-02-01T00:00:00Z', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0)",
    )
    .await;
    approve_migration(&db).await;

    Migration.up(&SchemaManager::new(&db)).await.unwrap();

    let rows = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id, group_id, priority, channel_id, channel_enabled, configuration_generation FROM monoize_providers ORDER BY group_id, priority".to_owned(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                row.try_get::<String>("", "group_id").unwrap(),
                row.try_get::<i32>("", "priority").unwrap(),
                row.try_get::<String>("", "channel_id").unwrap(),
                row.try_get::<i32>("", "channel_enabled").unwrap(),
                row.try_get::<i64>("", "configuration_generation").unwrap(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        rows,
        vec![
            (
                deterministic_id("provider", "provider-one", "group-one", "channel-z"),
                "group-one".to_owned(),
                0,
                deterministic_id("channel", "provider-one", "group-one", "channel-z"),
                1,
                1,
            ),
            (
                deterministic_id("provider", "provider-one", "group-one", "channel-a"),
                "group-one".to_owned(),
                1,
                deterministic_id("channel", "provider-one", "group-one", "channel-a"),
                0,
                1,
            ),
            (
                "provider-one".to_owned(),
                "group-two".to_owned(),
                0,
                "channel-z".to_owned(),
                1,
                1,
            ),
            (
                deterministic_id("provider", "provider-one", "group-two", "channel-a"),
                "group-two".to_owned(),
                1,
                "channel-a".to_owned(),
                0,
                1,
            ),
        ]
    );
}
