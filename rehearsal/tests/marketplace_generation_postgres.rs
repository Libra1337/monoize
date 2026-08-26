use monoize_lynshen_rehearsal::marketplace::{
    create_postgres_generation_schema, postgres_generation_revision,
};
use sqlx::{Connection, Executor, PgConnection};

async fn database() -> Option<PgConnection> {
    let url = std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL").ok()?;
    let mut db = PgConnection::connect(&url).await.unwrap();
    db.execute("DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public")
        .await
        .unwrap();
    for statement in [
        "CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT)",
        "CREATE TABLE monoize_providers (id TEXT PRIMARY KEY, public_name TEXT)",
        "CREATE TABLE monoize_provider_models (provider_id TEXT, model_name TEXT)",
        "CREATE TABLE billing_rate_records (id TEXT PRIMARY KEY, unit_price TEXT)",
        "CREATE TABLE model_metadata_records (model_id TEXT PRIMARY KEY, mode TEXT)",
        "CREATE TABLE system_settings (key TEXT PRIMARY KEY, value TEXT, updated_at TEXT)",
    ] {
        db.execute(statement).await.unwrap();
    }
    create_postgres_generation_schema(&mut db, 1_000_000)
        .await
        .unwrap();
    Some(db)
}

#[tokio::test]
async fn statement_triggers_advance_once_and_truncate_advances_once() {
    let Some(mut db) = database().await else {
        return;
    };
    let initial = postgres_generation_revision(&mut db).await.unwrap();
    db.execute("INSERT INTO monoize_groups VALUES ('g1', 'A'), ('g2', 'B')")
        .await
        .unwrap();
    assert_eq!(
        postgres_generation_revision(&mut db).await.unwrap(),
        initial + 1
    );
    db.execute("TRUNCATE monoize_groups").await.unwrap();
    assert_eq!(
        postgres_generation_revision(&mut db).await.unwrap(),
        initial + 2
    );
}

#[tokio::test]
async fn filtered_settings_and_rollback_match_contract() {
    let Some(mut db) = database().await else {
        return;
    };
    let initial = postgres_generation_revision(&mut db).await.unwrap();
    db.execute("INSERT INTO system_settings VALUES ('other', '1', 'a')")
        .await
        .unwrap();
    assert_eq!(
        postgres_generation_revision(&mut db).await.unwrap(),
        initial
    );
    db.execute("INSERT INTO system_settings VALUES ('reasoning_suffix_map', '{}', 'a')")
        .await
        .unwrap();
    assert_eq!(
        postgres_generation_revision(&mut db).await.unwrap(),
        initial + 1
    );
    db.execute("UPDATE system_settings SET updated_at = 'b'")
        .await
        .unwrap();
    assert_eq!(
        postgres_generation_revision(&mut db).await.unwrap(),
        initial + 1
    );
    db.execute("BEGIN").await.unwrap();
    db.execute("UPDATE system_settings SET value = '{\"x\":1}' WHERE key = 'reasoning_suffix_map'")
        .await
        .unwrap();
    db.execute("ROLLBACK").await.unwrap();
    assert_eq!(
        postgres_generation_revision(&mut db).await.unwrap(),
        initial + 1
    );
}

#[tokio::test]
async fn singleton_guards_delete_skip_and_truncate() {
    let Some(mut db) = database().await else {
        return;
    };
    assert!(
        db.execute("DELETE FROM marketplace_generation")
            .await
            .is_err()
    );
    assert!(db.execute("TRUNCATE marketplace_generation").await.is_err());
    assert!(
        db.execute("UPDATE marketplace_generation SET revision = revision + 2, generated_at_unix_us = generated_at_unix_us + 1")
            .await
            .is_err()
    );
}
