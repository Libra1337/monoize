use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let tx = manager.get_connection().begin().await?;
        for sql in reauth_scope_upgrade_sql(backend) {
            tx.execute(Statement::from_string(backend, sql)).await?;
        }
        tx.execute(Statement::from_string(
            backend,
            readiness_table_sql(backend),
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP TABLE IF EXISTS store_channel_readiness_profiles".to_string(),
        ))
        .await?;
        for sql in reauth_scope_downgrade_sql(backend) {
            tx.execute(Statement::from_string(backend, sql)).await?;
        }
        tx.commit().await
    }
}

fn reauth_scope_upgrade_sql(backend: DbBackend) -> Vec<String> {
    if backend == DbBackend::Postgres {
        return vec![
            "ALTER TABLE store_reauth_grants DROP CONSTRAINT IF EXISTS store_reauth_grants_scope_check"
                .to_string(),
            "ALTER TABLE store_reauth_grants ADD CONSTRAINT store_reauth_grants_scope_check CHECK (scope IN ('credential_update', 'redemption_access', 'compliance_confirm'))"
                .to_string(),
        ];
    }
    vec![
        "DROP INDEX IF EXISTS uq_store_reauth_token_digest".to_string(),
        "DROP INDEX IF EXISTS idx_store_reauth_expiry".to_string(),
        "ALTER TABLE store_reauth_grants RENAME TO store_reauth_grants_v052".to_string(),
        reauth_table_sql("store_reauth_grants", true),
        "INSERT INTO store_reauth_grants
            (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
         SELECT id, user_id, session_token_digest, token_digest, scope, created_at, expires_at
         FROM store_reauth_grants_v052"
            .to_string(),
        "DROP TABLE store_reauth_grants_v052".to_string(),
        "CREATE UNIQUE INDEX uq_store_reauth_token_digest ON store_reauth_grants (token_digest)"
            .to_string(),
        "CREATE INDEX idx_store_reauth_expiry ON store_reauth_grants (expires_at, id)".to_string(),
    ]
}

fn reauth_scope_downgrade_sql(backend: DbBackend) -> Vec<String> {
    if backend == DbBackend::Postgres {
        return vec![
            "DELETE FROM store_reauth_grants WHERE scope = 'compliance_confirm'".to_string(),
            "ALTER TABLE store_reauth_grants DROP CONSTRAINT IF EXISTS store_reauth_grants_scope_check"
                .to_string(),
            "ALTER TABLE store_reauth_grants ADD CONSTRAINT store_reauth_grants_scope_check CHECK (scope IN ('credential_update', 'redemption_access'))"
                .to_string(),
        ];
    }
    vec![
        "DELETE FROM store_reauth_grants WHERE scope = 'compliance_confirm'".to_string(),
        "DROP INDEX IF EXISTS uq_store_reauth_token_digest".to_string(),
        "DROP INDEX IF EXISTS idx_store_reauth_expiry".to_string(),
        "ALTER TABLE store_reauth_grants RENAME TO store_reauth_grants_v054".to_string(),
        reauth_table_sql("store_reauth_grants", false),
        "INSERT INTO store_reauth_grants
            (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
         SELECT id, user_id, session_token_digest, token_digest, scope, created_at, expires_at
         FROM store_reauth_grants_v054"
            .to_string(),
        "DROP TABLE store_reauth_grants_v054".to_string(),
        "CREATE UNIQUE INDEX uq_store_reauth_token_digest ON store_reauth_grants (token_digest)"
            .to_string(),
        "CREATE INDEX idx_store_reauth_expiry ON store_reauth_grants (expires_at, id)".to_string(),
    ]
}

fn reauth_table_sql(name: &str, compliance_scope: bool) -> String {
    let scopes = if compliance_scope {
        "'credential_update', 'redemption_access', 'compliance_confirm'"
    } else {
        "'credential_update', 'redemption_access'"
    };
    format!(
        "CREATE TABLE {name} (
            id TEXT NOT NULL PRIMARY KEY, user_id TEXT NOT NULL,
            session_token_digest TEXT NOT NULL, token_digest TEXT NOT NULL,
            scope TEXT NOT NULL CHECK (scope IN ({scopes})),
            created_at TEXT NOT NULL, expires_at TEXT NOT NULL
        )"
    )
}

fn readiness_table_sql(backend: DbBackend) -> String {
    let digest = if backend == DbBackend::Postgres {
        "{column} ~ '^[0-9a-f]{64}$'"
    } else {
        "length({column}) = 64 AND {column} NOT GLOB '*[^0-9a-f]*'"
    };
    let check = |column: &str| digest.replace("{column}", column);
    format!(
        "CREATE TABLE store_channel_readiness_profiles (
            channel_id TEXT NOT NULL PRIMARY KEY,
            active_credential_digest TEXT NOT NULL,
            privacy_record_id TEXT NOT NULL,
            callback_verification_passed INTEGER NOT NULL
                CHECK (callback_verification_passed IN (0, 1)),
            supported_currencies_json TEXT NOT NULL,
            amount_limits_json TEXT NOT NULL,
            checkout_action_kinds_json TEXT NOT NULL,
            license_evidence_digest TEXT NOT NULL,
            runtime_evidence_digest TEXT NOT NULL,
            availability_evidence_digest TEXT NOT NULL,
            verifier_admin_id TEXT NOT NULL,
            verified_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            CHECK ({}), CHECK ({}), CHECK ({}), CHECK ({}),
            FOREIGN KEY (channel_id) REFERENCES store_payment_channels(id) ON DELETE CASCADE,
            FOREIGN KEY (privacy_record_id) REFERENCES store_privacy_records(id) ON DELETE RESTRICT
        )",
        check("active_credential_digest"),
        check("license_evidence_digest"),
        check("runtime_evidence_digest"),
        check("availability_evidence_digest")
    )
}

#[cfg(test)]
mod tests {
    use super::{readiness_table_sql, reauth_scope_upgrade_sql};
    use sea_orm::DbBackend;

    #[test]
    fn postgres_scope_upgrade_and_readiness_constraints_are_explicit() {
        let scope = reauth_scope_upgrade_sql(DbBackend::Postgres).join("\n");
        assert!(scope.contains("DROP CONSTRAINT IF EXISTS store_reauth_grants_scope_check"));
        assert!(scope.contains("'compliance_confirm'"));
        let readiness = readiness_table_sql(DbBackend::Postgres);
        for column in [
            "active_credential_digest",
            "license_evidence_digest",
            "runtime_evidence_digest",
            "availability_evidence_digest",
        ] {
            assert!(readiness.contains(&format!("{column} ~ '^[0-9a-f]{{64}}$'")));
        }
    }
}
