use monoize_lynshen_rehearsal::marketplace::{
    create_sqlite_generation_schema, generation_revision,
};
use sqlx::{Connection, Executor, SqliteConnection};

async fn database() -> SqliteConnection {
    let mut db = SqliteConnection::connect("sqlite::memory:").await.unwrap();
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
    create_sqlite_generation_schema(&mut db, 1_000_000)
        .await
        .unwrap();
    db
}

#[tokio::test]
async fn full_sources_advance_once_per_affected_row_and_rollback_restores_revision() {
    let mut db = database().await;
    let before = generation_revision(&mut db).await.unwrap();
    db.execute("INSERT INTO monoize_groups VALUES ('g1', 'A'), ('g2', 'B')")
        .await
        .unwrap();
    assert_eq!(generation_revision(&mut db).await.unwrap(), before + 2);

    db.execute("BEGIN").await.unwrap();
    db.execute("UPDATE monoize_groups SET public_name = 'C'")
        .await
        .unwrap();
    db.execute("ROLLBACK").await.unwrap();
    assert_eq!(generation_revision(&mut db).await.unwrap(), before + 2);
}

#[tokio::test]
async fn filtered_setting_advances_only_for_reasoning_suffix_semantics() {
    let mut db = database().await;
    let initial = generation_revision(&mut db).await.unwrap();
    db.execute("INSERT INTO system_settings VALUES ('other', '1', 'a')")
        .await
        .unwrap();
    assert_eq!(generation_revision(&mut db).await.unwrap(), initial);

    db.execute("INSERT INTO system_settings VALUES ('reasoning_suffix_map', '{}', 'a')")
        .await
        .unwrap();
    assert_eq!(generation_revision(&mut db).await.unwrap(), initial + 1);
    db.execute("UPDATE system_settings SET updated_at = 'b' WHERE key = 'reasoning_suffix_map'")
        .await
        .unwrap();
    assert_eq!(generation_revision(&mut db).await.unwrap(), initial + 1);
    db.execute("UPDATE system_settings SET value = value WHERE key = 'reasoning_suffix_map'")
        .await
        .unwrap();
    assert_eq!(generation_revision(&mut db).await.unwrap(), initial + 1);
    db.execute("UPDATE system_settings SET value = '{\"x\":1}' WHERE key = 'reasoning_suffix_map'")
        .await
        .unwrap();
    assert_eq!(generation_revision(&mut db).await.unwrap(), initial + 2);
}

#[tokio::test]
async fn singleton_cannot_be_deleted_or_updated_out_of_sequence() {
    let mut db = database().await;
    assert!(
        db.execute("DELETE FROM marketplace_generation")
            .await
            .is_err()
    );
    assert!(
        db.execute("UPDATE marketplace_generation SET revision = revision + 2, generated_at_unix_us = generated_at_unix_us + 1")
            .await
            .is_err()
    );
}
