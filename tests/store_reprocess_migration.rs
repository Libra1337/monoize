use monoize::migration::Migrator;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn migration_056_preserves_grants_adds_reprocess_scope_and_recreates_indexes() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    Migrator::down(&db, Some(3)).await.unwrap();

    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "PRAGMA foreign_keys = OFF".to_string(),
    ))
    .await
    .unwrap();
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "INSERT INTO store_reauth_grants
            (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
         VALUES ('refund-grant', 'legacy-admin', 'session-digest', 'token-digest',
                 'refund', '2026-08-28T00:00:00Z', '2026-08-28T00:05:00Z')"
            .to_string(),
    ))
    .await
    .unwrap();

    Migrator::up(&db, Some(1)).await.unwrap();
    let legacy = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT scope FROM store_reauth_grants WHERE id = 'refund-grant'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(legacy.try_get::<String>("", "scope").unwrap(), "refund");

    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "INSERT INTO store_reauth_grants
            (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
         VALUES ('reprocess-grant', 'legacy-admin', 'reprocess-session', 'reprocess-token',
                 'reprocess', '2026-08-28T00:00:00Z', '2026-08-28T00:05:00Z')"
            .to_string(),
    ))
    .await
    .unwrap();

    let indexes = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master
             WHERE type = 'index' AND name IN
               ('uq_store_reauth_token_digest', 'idx_store_reauth_expiry')
             ORDER BY name"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        indexes,
        ["idx_store_reauth_expiry", "uq_store_reauth_token_digest"]
    );
}

#[test]
fn migration_056_postgres_branch_recreates_reauth_indexes() {
    let source = include_str!("../src/migration/m20260828_000056_store_reprocess_reauth.rs");
    let postgres_branch = source
        .split("} else {")
        .next()
        .expect("migration has a PostgreSQL branch");
    for statement in [
        "DROP INDEX IF EXISTS uq_store_reauth_token_digest",
        "DROP INDEX IF EXISTS idx_store_reauth_expiry",
        "CREATE UNIQUE INDEX uq_store_reauth_token_digest",
        "CREATE INDEX idx_store_reauth_expiry",
    ] {
        assert!(postgres_branch.contains(statement), "missing {statement}");
    }
}
