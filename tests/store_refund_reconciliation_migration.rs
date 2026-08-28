use monoize::migration::Migrator;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn migration_057_creates_refund_query_retry_table_and_due_index() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();

    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "INSERT INTO store_refund_query_retries
            (refund_id, attempt_count, next_attempt_at, last_error_category,
             alerted_at, updated_at)
         VALUES ('missing-refund', -1, '2026-08-28T00:00:00Z', NULL, NULL,
                 '2026-08-28T00:00:00Z')"
            .to_string(),
    ))
    .await
    .unwrap_err();

    let index = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT sql FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_store_refund_query_retries_due'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    let sql = index.try_get::<String>("", "sql").unwrap();
    assert!(sql.contains("next_attempt_at, refund_id"));
}

#[test]
fn migration_057_contains_postgres_table_constraints_and_due_index() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/migration/m20260828_000057_store_refund_query_retries.rs");
    let source = std::fs::read_to_string(path).expect("migration 057 source exists");
    for statement in [
        "CREATE TABLE store_refund_query_retries",
        "attempt_count BIGINT NOT NULL CHECK (attempt_count >= 0)",
        "FOREIGN KEY (refund_id) REFERENCES store_refunds(id) ON DELETE CASCADE",
        "CREATE INDEX idx_store_refund_query_retries_due",
        "(next_attempt_at, refund_id)",
    ] {
        assert!(source.contains(statement), "missing {statement}");
    }
}
