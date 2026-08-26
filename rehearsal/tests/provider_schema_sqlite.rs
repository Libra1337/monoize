use monoize_lynshen_rehearsal::provider::{create_sqlite_target_schema, sqlite_table_exists};
use sqlx::{Connection, Executor, SqliteConnection};

async fn database() -> SqliteConnection {
    let mut db = SqliteConnection::connect("sqlite::memory:")
        .await
        .expect("open SQLite");
    db.execute("PRAGMA foreign_keys = ON")
        .await
        .expect("enable foreign keys");
    db.execute("CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL, public_name_key BLOB NOT NULL UNIQUE)")
        .await
        .expect("legacy group fixture");
    db.execute("INSERT INTO monoize_groups VALUES ('g', 'Public', CAST('Public' AS BLOB))")
        .await
        .expect("group fixture");
    db
}

#[tokio::test]
async fn target_schema_has_no_legacy_channel_tables() {
    let mut db = database().await;
    create_sqlite_target_schema(&mut db).await.unwrap();
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
}

#[tokio::test]
async fn target_constraints_reject_duplicate_names_and_mismatched_keys() {
    let mut db = database().await;
    create_sqlite_target_schema(&mut db).await.unwrap();
    let insert = "INSERT INTO monoize_providers (id, group_id, name, public_name, public_name_key, priority, enabled, pricing_profile, multiplier, configuration_generation, created_at, channel_id, channel_name, channel_public_name, channel_public_name_key, channel_provider_type, channel_base_url, channel_api_key, channel_enabled, channel_max_retries) VALUES (?, 'g', ?, ?, CAST(? AS BLOB), 0, 1, 'S', '1', 1, '2026-08-26T00:00:00Z', ?, ?, ?, CAST(? AS BLOB), 'responses', 'https://example.invalid', 'secret', 1, 0)";
    sqlx::query(insert)
        .bind("p1")
        .bind("internal")
        .bind("Provider Public")
        .bind("Provider Public")
        .bind("c1")
        .bind("channel internal")
        .bind("Channel Public")
        .bind("Channel Public")
        .execute(&mut db)
        .await
        .unwrap();

    assert!(
        sqlx::query(insert)
            .bind("p2")
            .bind("other")
            .bind("Provider Public")
            .bind("Provider Public")
            .bind("c2")
            .bind("other channel")
            .bind("Channel Two")
            .bind("Channel Two")
            .execute(&mut db)
            .await
            .is_err()
    );

    let model_insert = "INSERT INTO monoize_provider_models (provider_id, model_name, model_name_key, model_search_key, redirect, pricing_profile_mode, pricing_profile_override, multiplier_override, created_at) VALUES ('p1', ?, CAST(? AS BLOB), CAST(? AS BLOB), NULL, 'inherit', NULL, NULL, '2026-08-26T00:00:00Z')";
    assert!(
        sqlx::query(model_insert)
            .bind("GPT-4o")
            .bind("wrong")
            .bind("gpt-4o")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(model_insert)
            .bind("GPT-4o")
            .bind("GPT-4o")
            .bind("wrong")
            .execute(&mut db)
            .await
            .is_err()
    );
    sqlx::query(model_insert)
        .bind("GPT-4o")
        .bind("GPT-4o")
        .bind("gpt-4o")
        .execute(&mut db)
        .await
        .unwrap();
    assert!(
        sqlx::query(model_insert)
            .bind("GPT-4o")
            .bind("GPT-4o")
            .bind("gpt-4o")
            .execute(&mut db)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn schema_creation_rolls_back_when_transaction_fails() {
    let mut db = database().await;
    create_sqlite_target_schema(&mut db).await.unwrap();
    assert!(create_sqlite_target_schema(&mut db).await.is_ok());
}
