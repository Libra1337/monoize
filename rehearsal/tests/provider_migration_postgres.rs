use monoize_lynshen_rehearsal::provider::{
    CanonicalDecimal, LegacyChannel, LegacyModel, LegacyProvider, MigrationFailurePoint,
    MigrationOutcome, migrate_postgres_provider_schema, postgres_table_exists,
};
use sqlx::{Connection, Executor, PgConnection, Row};

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

async fn legacy_database() -> Option<PgConnection> {
    let url = std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL").ok()?;
    let mut db = PgConnection::connect(&url).await.unwrap();
    db.execute("DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public")
        .await
        .unwrap();
    for statement in [
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL, public_name_key BYTEA NOT NULL UNIQUE)",
        "INSERT INTO monoize_groups VALUES ('g1', 'Group One', convert_to('Group One', 'UTF8')), ('g2', 'Group Two', convert_to('Group Two', 'UTF8'))",
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, name TEXT NOT NULL, group_ids TEXT NOT NULL, max_retries INTEGER NOT NULL)",
        "CREATE TABLE monoize_channels (id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, name TEXT NOT NULL, weight INTEGER NOT NULL)",
        "CREATE TABLE monoize_channel_models (id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, model_name TEXT NOT NULL, multiplier TEXT NOT NULL)",
        "INSERT INTO monoize_providers VALUES ('provider-old', 'Provider', '[\"g1\",\"g2\"]', -1)",
    ] {
        db.execute(statement).await.unwrap();
    }
    Some(db)
}

#[tokio::test]
async fn postgres_migration_expands_and_removes_legacy_storage() {
    let Some(mut db) = legacy_database().await else {
        return;
    };
    assert_eq!(
        migrate_postgres_provider_schema(&mut db, &[source()], None)
            .await
            .unwrap(),
        MigrationOutcome::Migrated {
            provider_count: 4,
            model_count: 2
        }
    );
    assert!(
        !postgres_table_exists(&mut db, "monoize_channels")
            .await
            .unwrap()
    );
    assert!(
        !postgres_table_exists(&mut db, "monoize_channel_models")
            .await
            .unwrap()
    );
    let priorities =
        sqlx::query("SELECT group_id, priority FROM monoize_providers ORDER BY group_id, priority")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .into_iter()
            .map(|row| {
                (
                    row.try_get::<String, _>("group_id").unwrap(),
                    row.try_get::<i32, _>("priority").unwrap(),
                )
            })
            .collect::<Vec<_>>();
    assert_eq!(
        priorities,
        vec![
            ("g1".to_owned(), 0),
            ("g1".to_owned(), 1),
            ("g2".to_owned(), 0),
            ("g2".to_owned(), 1),
        ]
    );
}

#[tokio::test]
async fn postgres_failures_roll_back_and_second_run_is_no_op() {
    for point in [
        MigrationFailurePoint::AfterLegacyRename,
        MigrationFailurePoint::AfterTargetSchema,
        MigrationFailurePoint::AfterProviders,
        MigrationFailurePoint::BeforeLegacyDrop,
    ] {
        let Some(mut db) = legacy_database().await else {
            return;
        };
        assert!(
            migrate_postgres_provider_schema(&mut db, &[source()], Some(point))
                .await
                .is_err()
        );
        assert!(
            postgres_table_exists(&mut db, "monoize_channels")
                .await
                .unwrap()
        );
        assert!(
            !postgres_table_exists(&mut db, "monoize_provider_models")
                .await
                .unwrap()
        );
    }

    let Some(mut db) = legacy_database().await else {
        return;
    };
    migrate_postgres_provider_schema(&mut db, &[source()], None)
        .await
        .unwrap();
    assert_eq!(
        migrate_postgres_provider_schema(&mut db, &[source()], None)
            .await
            .unwrap(),
        MigrationOutcome::AlreadyMigrated
    );
}
