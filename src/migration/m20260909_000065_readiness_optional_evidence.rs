use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait, TryGetable};
use sea_orm_migration::prelude::*;
use sea_orm_migration::SchemaManagerConnection;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Columns whose value is an externally issued attestation rather than a fact the deployment
/// can observe. SB-C-38 makes them optional so an adapter with no issuer can still hold a
/// readiness profile, which is what carries the currency, amount, and checkout-action metadata.
const OPTIONAL_DIGESTS: [&str; 3] = [
    "license_evidence_digest",
    "runtime_evidence_digest",
    "availability_evidence_digest",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.run(manager, true).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.run(manager, false).await
    }
}

impl Migration {
    /// Wraps the rebuild in the pragma dance migration 064 established. SQLite cannot drop a
    /// NOT NULL or alter a CHECK, so the table is recreated; with foreign keys enabled
    /// `ALTER TABLE ... RENAME` would rewrite every `REFERENCES` clause naming this table and
    /// `DROP TABLE` would cascade child rows away. Both pragmas must be set on the connection
    /// before the transaction opens, because SQLite ignores a `foreign_keys` change inside one.
    async fn run(&self, manager: &SchemaManager<'_>, forward: bool) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let connection = manager.get_connection();
        let restore_foreign_keys = if backend == DbBackend::Sqlite {
            sqlite_foreign_keys_enabled(connection).await?
        } else {
            false
        };
        if backend == DbBackend::Sqlite {
            connection
                .execute_unprepared("PRAGMA legacy_alter_table = ON")
                .await?;
        }
        if restore_foreign_keys {
            connection
                .execute_unprepared("PRAGMA foreign_keys = OFF")
                .await?;
        }
        let result = self.apply(manager, backend, forward).await;
        if restore_foreign_keys {
            connection
                .execute_unprepared("PRAGMA foreign_keys = ON")
                .await?;
        }
        if backend == DbBackend::Sqlite {
            connection
                .execute_unprepared("PRAGMA legacy_alter_table = OFF")
                .await?;
        }
        result
    }

    async fn apply(
        &self,
        manager: &SchemaManager<'_>,
        backend: DbBackend,
        forward: bool,
    ) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;

        // Reverting restores NOT NULL, so a profile that used the optional form cannot be
        // represented. Dropping those profiles is the only lossless-for-the-schema option; the
        // Channel falls back to `readiness_profile_missing` and the Admin re-enters it.
        if !forward {
            let predicate = OPTIONAL_DIGESTS
                .iter()
                .map(|column| format!("{column} IS NULL"))
                .chain(std::iter::once("privacy_record_id IS NULL".to_string()))
                .collect::<Vec<_>>()
                .join(" OR ");
            tx.execute(Statement::from_string(
                backend,
                format!("DELETE FROM store_channel_readiness_profiles WHERE {predicate}"),
            ))
            .await?;
        }

        match backend {
            DbBackend::Sqlite => {
                tx.execute_unprepared(
                    "ALTER TABLE store_channel_readiness_profiles
                     RENAME TO store_channel_readiness_profiles_old",
                )
                .await?;
                tx.execute_unprepared(&readiness_table_sql(backend, forward))
                    .await?;
                tx.execute_unprepared(
                    "INSERT INTO store_channel_readiness_profiles
                        (channel_id, active_credential_digest, privacy_record_id,
                         callback_verification_passed, supported_currencies_json,
                         amount_limits_json, checkout_action_kinds_json,
                         license_evidence_digest, runtime_evidence_digest,
                         availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
                     SELECT channel_id, active_credential_digest, privacy_record_id,
                            callback_verification_passed, supported_currencies_json,
                            amount_limits_json, checkout_action_kinds_json,
                            license_evidence_digest, runtime_evidence_digest,
                            availability_evidence_digest, verifier_admin_id, verified_at,
                            expires_at
                     FROM store_channel_readiness_profiles_old",
                )
                .await?;
                tx.execute_unprepared("DROP TABLE store_channel_readiness_profiles_old")
                    .await?;
            }
            DbBackend::Postgres => {
                for column in OPTIONAL_DIGESTS
                    .iter()
                    .copied()
                    .chain(std::iter::once("privacy_record_id"))
                {
                    let action = if forward { "DROP" } else { "SET" };
                    tx.execute(Statement::from_string(
                        backend,
                        format!(
                            "ALTER TABLE store_channel_readiness_profiles
                             ALTER COLUMN {column} {action} NOT NULL"
                        ),
                    ))
                    .await?;
                }
                for column in OPTIONAL_DIGESTS {
                    // The migration-054 constraints are unnamed, so they are located by the
                    // column their definition mentions rather than by a known name.
                    let rows = tx
                        .query_all(Statement::from_string(
                            backend,
                            format!(
                                "SELECT conname AS name FROM pg_constraint
                                 WHERE conrelid = 'store_channel_readiness_profiles'::regclass
                                   AND contype = 'c'
                                   AND pg_get_constraintdef(oid) LIKE '%{column}%'"
                            ),
                        ))
                        .await?;
                    for row in rows {
                        let name = String::try_get(&row, "", "name")?;
                        tx.execute(Statement::from_string(
                            backend,
                            format!(
                                "ALTER TABLE store_channel_readiness_profiles
                                 DROP CONSTRAINT {}",
                                quote_identifier(&name)
                            ),
                        ))
                        .await?;
                    }
                    tx.execute(Statement::from_string(
                        backend,
                        format!(
                            "ALTER TABLE store_channel_readiness_profiles
                             ADD CONSTRAINT ck_readiness_{column}
                             CHECK ({})",
                            digest_check(backend, column, forward)
                        ),
                    ))
                    .await?;
                }
            }
            _ => {}
        }

        commit_with_foreign_key_check(tx, backend).await
    }
}

fn digest_check(backend: DbBackend, column: &str, nullable: bool) -> String {
    let pattern = if backend == DbBackend::Postgres {
        format!("{column} ~ '^[0-9a-f]{{64}}$'")
    } else {
        format!("length({column}) = 64 AND {column} NOT GLOB '*[^0-9a-f]*'")
    };
    if nullable {
        format!("{column} IS NULL OR ({pattern})")
    } else {
        pattern
    }
}

fn readiness_table_sql(backend: DbBackend, nullable: bool) -> String {
    let null = if nullable { "" } else { " NOT NULL" };
    format!(
        "CREATE TABLE store_channel_readiness_profiles (
            channel_id TEXT NOT NULL PRIMARY KEY,
            active_credential_digest TEXT NOT NULL,
            privacy_record_id TEXT{null},
            callback_verification_passed INTEGER NOT NULL
                CHECK (callback_verification_passed IN (0, 1)),
            supported_currencies_json TEXT NOT NULL,
            amount_limits_json TEXT NOT NULL,
            checkout_action_kinds_json TEXT NOT NULL,
            license_evidence_digest TEXT{null},
            runtime_evidence_digest TEXT{null},
            availability_evidence_digest TEXT{null},
            verifier_admin_id TEXT NOT NULL,
            verified_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            CHECK ({}), CHECK ({}), CHECK ({}), CHECK ({}),
            FOREIGN KEY (channel_id) REFERENCES store_payment_channels(id) ON DELETE CASCADE,
            FOREIGN KEY (privacy_record_id)
                REFERENCES store_privacy_records(id) ON DELETE RESTRICT
        )",
        digest_check(backend, "active_credential_digest", false),
        digest_check(backend, "license_evidence_digest", nullable),
        digest_check(backend, "runtime_evidence_digest", nullable),
        digest_check(backend, "availability_evidence_digest", nullable),
    )
}

async fn sqlite_foreign_keys_enabled(
    connection: &SchemaManagerConnection<'_>,
) -> Result<bool, DbErr> {
    let row = connection
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA foreign_keys".to_string(),
        ))
        .await?;
    match row {
        Some(row) => Ok(i32::try_get(&row, "", "foreign_keys")? != 0),
        None => Ok(false),
    }
}

/// Commits the rebuild only when no dangling reference remains. `PRAGMA foreign_key_check`
/// replaces the enforcement that stayed disabled for the rebuild.
async fn commit_with_foreign_key_check(
    tx: sea_orm::DatabaseTransaction,
    backend: DbBackend,
) -> Result<(), DbErr> {
    if backend == DbBackend::Sqlite {
        let violations = tx
            .query_all(Statement::from_string(
                backend,
                "PRAGMA foreign_key_check".to_string(),
            ))
            .await?;
        if !violations.is_empty() {
            tx.rollback().await?;
            return Err(DbErr::Migration(format!(
                "readiness rebuild left {} dangling foreign key reference(s)",
                violations.len()
            )));
        }
    }
    tx.commit().await
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::{digest_check, readiness_table_sql, Migration};
    use sea_orm::{
        ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, TryGetable,
    };
    use sea_orm_migration::prelude::*;

    const DIGEST_A: &str = "aa11223344556677889900aabbccddeeff00112233445566778899aabbccddee";
    const DIGEST_B: &str = "bb11223344556677889900aabbccddeeff00112233445566778899aabbccddee";

    /// Builds the schema through migration 064 so the rebuild runs against the real table
    /// shape, including the `store_privacy_records` reference that must survive it.
    async fn database_before_065() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect SQLite");
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .expect("enable foreign keys");
        let manager = SchemaManager::new(&db);
        for migration in crate::migration::Migrator::migrations() {
            if migration.name() == Migration.name() {
                break;
            }
            migration.up(&manager).await.expect(migration.name());
        }
        db
    }

    async fn seed_privacy_and_channel(db: &DatabaseConnection) {
        db.execute_unprepared(&format!(
            "INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('privacy-1', 'v1', 'CN', '[]', '{{}}', 'contract', 'admin-1', '{DIGEST_A}',
                     '2026-01-01T00:00:00.000000Z', '2027-01-01T00:00:00.000000Z', 1)"
        ))
        .await
        .expect("seed privacy record");
        db.execute_unprepared(
            "INSERT INTO store_payment_channels
                (id, adapter_kind, name, icon_kind, icon_value, sort_order, enabled, revision,
                 created_at, updated_at)
             VALUES ('ch-1', 'epay', 'EPay', 'builtin', NULL, 0, 0, 1,
                     '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z')",
        )
        .await
        .expect("seed channel");
    }

    async fn insert_readiness(
        db: &DatabaseConnection,
        privacy: &str,
        digests: &str,
    ) -> Result<(), DbErr> {
        db.execute_unprepared(&format!(
            "INSERT INTO store_channel_readiness_profiles
                (channel_id, active_credential_digest, privacy_record_id,
                 callback_verification_passed, supported_currencies_json, amount_limits_json,
                 checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                 availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES ('ch-1', '{DIGEST_B}', {privacy}, 1, '[\"CNY\"]', '{{}}', '[\"qr\"]',
                     {digests}, {digests}, {digests}, 'admin-1',
                     '2026-01-01T00:00:00.000000Z', '2026-02-01T00:00:00.000000Z')"
        ))
        .await
        .map(|_| ())
    }

    async fn readiness_count(db: &DatabaseConnection) -> i64 {
        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM store_channel_readiness_profiles".to_string(),
            ))
            .await
            .expect("count readiness")
            .expect("one row");
        i64::try_get(&row, "", "n").expect("count value")
    }

    async fn foreign_keys_intact(db: &DatabaseConnection) -> bool {
        db.query_all(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA foreign_key_check".to_string(),
        ))
        .await
        .expect("foreign key check")
        .is_empty()
    }

    /// The rebuild must carry an existing fully attested profile across unchanged and leave no
    /// dangling reference, because the table is dropped and recreated underneath a RESTRICT
    /// foreign key into `store_privacy_records`.
    #[tokio::test]
    async fn rebuild_preserves_an_existing_attested_profile() {
        let db = database_before_065().await;
        seed_privacy_and_channel(&db).await;
        insert_readiness(&db, "'privacy-1'", &format!("'{DIGEST_A}'"))
            .await
            .expect("seed readiness");

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 065");

        assert_eq!(readiness_count(&db).await, 1);
        assert!(foreign_keys_intact(&db).await);
        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT privacy_record_id AS p, license_evidence_digest AS l
                 FROM store_channel_readiness_profiles WHERE channel_id = 'ch-1'"
                    .to_string(),
            ))
            .await
            .expect("read profile")
            .expect("one row");
        assert_eq!(
            String::try_get(&row, "", "p").expect("privacy id"),
            "privacy-1"
        );
        assert_eq!(
            String::try_get(&row, "", "l").expect("license digest"),
            DIGEST_A
        );
    }

    /// SB-C-38: after the migration an exempt adapter can store a profile with no privacy
    /// record and no evidence digests. This is the row shape the pre-065 schema rejected.
    #[tokio::test]
    async fn migrated_schema_accepts_a_profile_with_no_attestations() {
        let db = database_before_065().await;
        seed_privacy_and_channel(&db).await;
        insert_readiness(&db, "NULL", "NULL")
            .await
            .expect_err("pre-065 schema must reject NULL attestations");

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 065");

        insert_readiness(&db, "NULL", "NULL")
            .await
            .expect("post-065 schema accepts NULL attestations");
        assert_eq!(readiness_count(&db).await, 1);
        assert!(foreign_keys_intact(&db).await);
    }

    /// A digest that is present must still be a 64-character lowercase hex string; making the
    /// column nullable must not turn it into a free-text field.
    #[tokio::test]
    async fn migrated_schema_still_rejects_a_malformed_digest() {
        let db = database_before_065().await;
        seed_privacy_and_channel(&db).await;
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 065");

        insert_readiness(&db, "NULL", "'not-a-digest'")
            .await
            .expect_err("short non-hex digest must fail the CHECK");
        assert_eq!(readiness_count(&db).await, 0);
    }

    /// Reverting restores NOT NULL, so a profile that used the optional form cannot be
    /// represented and must be dropped rather than block the revert.
    #[tokio::test]
    async fn revert_drops_profiles_that_have_no_attestations() {
        let db = database_before_065().await;
        seed_privacy_and_channel(&db).await;
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 065");
        insert_readiness(&db, "NULL", "NULL")
            .await
            .expect("store exempt profile");

        Migration
            .down(&SchemaManager::new(&db))
            .await
            .expect("revert 065");

        assert_eq!(readiness_count(&db).await, 0);
        assert!(foreign_keys_intact(&db).await);
        insert_readiness(&db, "NULL", "NULL")
            .await
            .expect_err("reverted schema must reject NULL attestations again");
    }

    /// A revert must keep a fully attested profile, since the restored schema can express it.
    #[tokio::test]
    async fn revert_keeps_a_fully_attested_profile() {
        let db = database_before_065().await;
        seed_privacy_and_channel(&db).await;
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 065");
        insert_readiness(&db, "'privacy-1'", &format!("'{DIGEST_A}'"))
            .await
            .expect("store attested profile");

        Migration
            .down(&SchemaManager::new(&db))
            .await
            .expect("revert 065");

        assert_eq!(readiness_count(&db).await, 1);
        assert!(foreign_keys_intact(&db).await);
    }

    #[test]
    fn nullable_sqlite_check_admits_null_and_rejects_short_hex() {
        let check = digest_check(DbBackend::Sqlite, "license_evidence_digest", true);
        assert!(check.starts_with("license_evidence_digest IS NULL OR ("));
        assert!(check.contains("length(license_evidence_digest) = 64"));
    }

    #[test]
    fn strict_check_has_no_null_escape() {
        let check = digest_check(DbBackend::Sqlite, "active_credential_digest", false);
        assert!(!check.contains("IS NULL"));
    }

    #[test]
    fn forward_table_keeps_credential_digest_and_channel_required() {
        let sql = readiness_table_sql(DbBackend::Sqlite, true);
        assert!(sql.contains("channel_id TEXT NOT NULL PRIMARY KEY"));
        assert!(sql.contains("active_credential_digest TEXT NOT NULL"));
        assert!(sql.contains("privacy_record_id TEXT,"));
        assert!(sql.contains("license_evidence_digest TEXT,"));
        assert!(!sql.contains("license_evidence_digest TEXT NOT NULL"));
    }

    #[test]
    fn reverted_table_restores_every_not_null() {
        let sql = readiness_table_sql(DbBackend::Sqlite, false);
        assert!(sql.contains("privacy_record_id TEXT NOT NULL"));
        assert!(sql.contains("license_evidence_digest TEXT NOT NULL"));
        assert!(sql.contains("runtime_evidence_digest TEXT NOT NULL"));
        assert!(sql.contains("availability_evidence_digest TEXT NOT NULL"));
        assert!(!sql.contains("IS NULL OR"));
    }
}
