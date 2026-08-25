use monoize_lynshen_rehearsal::provider::{
    CanonicalDecimal, LegacyChannel, LegacyModel, LegacyProvider, MigrationFailurePoint,
    MigrationOutcome, migrate_sqlite_provider_schema, sqlite_table_exists,
};
use sqlx::{Connection, Executor, Row, SqliteConnection};

fn source() -> LegacyProvider {
    LegacyProvider {
        id: "provider-old".to_owned(),
        name: "Provider".to_owned(),
        priority: 4,
        group_ids: vec!["g1".to_owned(), "g2".to_owned()],
        channels: vec![
            LegacyChannel {
                id: "c1".to_owned(),
                name: "Channel 1".to_owned(),
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                enabled: true,
                weight: 1,
                models: vec![LegacyModel {
                    name: "GPT-4o".to_owned(),
                    redirect: Some("gpt-4o-upstream".to_owned()),
                    resolved_profile: Some("S".to_owned()),
                    multiplier: CanonicalDecimal::parse("1.2").unwrap(),
                }],
            },
            LegacyChannel {
                id: "c2".to_owned(),
                name: "Channel 2".to_owned(),
                created_at: "2026-01-02T00:00:00Z".to_owned(),
                enabled: true,
                weight: 0,
                models: vec![],
            },
        ],
    }
}

async fn legacy_database() -> SqliteConnection {
    let mut db = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    db.execute("PRAGMA foreign_keys = ON").await.unwrap();
    db.execute("CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL, public_name_key BLOB NOT NULL UNIQUE)")
        .await
        .unwrap();
    db.execute("INSERT INTO monoize_groups VALUES ('g1', 'Group One', CAST('Group One' AS BLOB)), ('g2', 'Group Two', CAST('Group Two' AS BLOB))")
        .await
        .unwrap();
    db.execute("CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, name TEXT NOT NULL, group_ids TEXT NOT NULL, max_retries INTEGER NOT NULL)")
        .await
        .unwrap();
    db.execute("CREATE TABLE monoize_channels (id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, name TEXT NOT NULL, weight INTEGER NOT NULL)")
        .await
        .unwrap();
    db.execute("CREATE TABLE monoize_channel_models (id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, model_name TEXT NOT NULL, multiplier TEXT NOT NULL)")
        .await
        .unwrap();
    db.execute(
        "INSERT INTO monoize_providers VALUES ('provider-old', 'Provider', '[\"g1\",\"g2\"]', -1)",
    )
    .await
    .unwrap();
    db
}

#[tokio::test]
async fn migration_replaces_legacy_tables_and_expands_every_pair() {
    let mut db = legacy_database().await;
    let outcome = migrate_sqlite_provider_schema(&mut db, &[source()], None)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        MigrationOutcome::Migrated {
            provider_count: 4,
            model_count: 2
        }
    );
    assert!(
        !sqlite_table_exists(&mut db, "monoize_channels")
            .await
            .unwrap()
    );
    assert!(
        !sqlite_table_exists(&mut db, "monoize_channel_models")
            .await
            .unwrap()
    );
    assert!(
        sqlite_table_exists(&mut db, "monoize_provider_models")
            .await
            .unwrap()
    );

    let provider_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM monoize_providers")
        .fetch_one(&mut db)
        .await
        .unwrap()
        .try_get("count")
        .unwrap();
    let enabled_count: i64 =
        sqlx::query("SELECT COUNT(*) AS count FROM monoize_providers WHERE channel_enabled = 1")
            .fetch_one(&mut db)
            .await
            .unwrap()
            .try_get("count")
            .unwrap();
    assert_eq!(provider_count, 4);
    assert_eq!(enabled_count, 2);
}

#[tokio::test]
async fn every_injected_failure_rolls_back_schema_and_rows() {
    for point in [
        MigrationFailurePoint::AfterLegacyRename,
        MigrationFailurePoint::AfterTargetSchema,
        MigrationFailurePoint::AfterProviders,
        MigrationFailurePoint::BeforeLegacyDrop,
    ] {
        let mut db = legacy_database().await;
        assert!(
            migrate_sqlite_provider_schema(&mut db, &[source()], Some(point))
                .await
                .is_err()
        );
        assert!(
            sqlite_table_exists(&mut db, "monoize_providers")
                .await
                .unwrap()
        );
        assert!(
            sqlite_table_exists(&mut db, "monoize_channels")
                .await
                .unwrap()
        );
        assert!(
            sqlite_table_exists(&mut db, "monoize_channel_models")
                .await
                .unwrap()
        );
        assert!(
            !sqlite_table_exists(&mut db, "monoize_provider_models")
                .await
                .unwrap()
        );
        let columns = sqlx::query("PRAGMA table_info(monoize_providers)")
            .fetch_all(&mut db)
            .await
            .unwrap();
        assert!(
            columns
                .iter()
                .any(|row| row.try_get::<String, _>("name").unwrap() == "group_ids")
        );
    }
}

#[tokio::test]
async fn second_invocation_is_no_op() {
    let mut db = legacy_database().await;
    migrate_sqlite_provider_schema(&mut db, &[source()], None)
        .await
        .unwrap();
    assert_eq!(
        migrate_sqlite_provider_schema(&mut db, &[source()], None)
            .await
            .unwrap(),
        MigrationOutcome::AlreadyMigrated
    );
}
