use monoize::migration::Migrator;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn migration_055_preserves_grants_adds_refund_scope_and_recreates_indexes() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    Migrator::down(&db, Some(1)).await.unwrap();

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
         VALUES ('legacy-grant', 'legacy-admin', 'session-digest', 'token-digest',
                 'compliance_confirm', '2026-08-28T00:00:00Z', '2026-08-28T00:05:00Z')"
            .to_string(),
    ))
    .await
    .unwrap();

    Migrator::up(&db, Some(1)).await.unwrap();
    let legacy = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT scope FROM store_reauth_grants WHERE id = 'legacy-grant'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(legacy.try_get::<String>("", "scope").unwrap(), "compliance_confirm");

    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "INSERT INTO store_reauth_grants
            (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
         VALUES ('refund-grant', 'legacy-admin', 'refund-session', 'refund-token',
                 'refund', '2026-08-28T00:00:00Z', '2026-08-28T00:05:00Z')"
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
