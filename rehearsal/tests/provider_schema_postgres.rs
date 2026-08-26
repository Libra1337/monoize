use monoize_lynshen_rehearsal::provider::{create_postgres_target_schema, postgres_table_exists};
use sqlx::{Connection, Executor, PgConnection};

async fn database() -> Option<PgConnection> {
    let url = std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL").ok()?;
    let mut db = PgConnection::connect(&url)
        .await
        .expect("connect PostgreSQL");
    db.execute("DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public")
        .await
        .unwrap();
    db.execute("CREATE TABLE monoize_groups (id TEXT PRIMARY KEY, public_name TEXT NOT NULL, public_name_key BYTEA NOT NULL UNIQUE)")
        .await
        .unwrap();
    db.execute("INSERT INTO monoize_groups VALUES ('g', 'Public', convert_to('Public', 'UTF8'))")
        .await
        .unwrap();
    Some(db)
}

#[tokio::test]
async fn postgres_target_schema_enforces_binary_keys_and_uniqueness() {
    let Some(mut db) = database().await else {
        return;
    };
    create_postgres_target_schema(&mut db).await.unwrap();
    assert!(
        postgres_table_exists(&mut db, "monoize_provider_models")
            .await
            .unwrap()
    );
    assert!(
        !postgres_table_exists(&mut db, "monoize_channels")
            .await
            .unwrap()
    );

    let insert = "INSERT INTO monoize_providers (id, group_id, name, public_name, public_name_key, priority, enabled, pricing_profile, multiplier, configuration_generation, created_at, channel_id, channel_name, channel_public_name, channel_public_name_key, channel_provider_type, channel_base_url, channel_api_key, channel_enabled, channel_max_retries) VALUES ($1, 'g', $2, $3, convert_to($3, 'UTF8'), 0, 1, 'S', '1', 1, '2026-08-26T00:00:00Z', $4, $5, $6, convert_to($6, 'UTF8'), 'responses', 'https://example.invalid', 'secret', 1, 0)";
    sqlx::query(insert)
        .bind("p1")
        .bind("internal")
        .bind("Provider Public")
        .bind("c1")
        .bind("channel internal")
        .bind("Channel Public")
        .execute(&mut db)
        .await
        .unwrap();
    assert!(
        sqlx::query(insert)
            .bind("p2")
            .bind("other")
            .bind("Provider Public")
            .bind("c2")
            .bind("other channel")
            .bind("Channel Two")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO monoize_provider_models (provider_id, model_name, model_name_key, model_search_key, redirect, pricing_profile_mode, pricing_profile_override, multiplier_override, created_at) VALUES ('p1', 'GPT-4o', convert_to('wrong', 'UTF8'), convert_to('gpt-4o', 'UTF8'), NULL, 'inherit', NULL, NULL, '2026-08-26T00:00:00Z')")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO monoize_provider_models (provider_id, model_name, model_name_key, model_search_key, redirect, pricing_profile_mode, pricing_profile_override, multiplier_override, created_at) VALUES ('p1', 'GPT-4o', convert_to('GPT-4o', 'UTF8'), convert_to('wrong', 'UTF8'), NULL, 'inherit', NULL, NULL, '2026-08-26T00:00:00Z')")
            .execute(&mut db)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn postgres_schema_creation_is_idempotent() {
    let Some(mut db) = database().await else {
        return;
    };
    create_postgres_target_schema(&mut db).await.unwrap();
    create_postgres_target_schema(&mut db).await.unwrap();
}
