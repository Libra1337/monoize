use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Rates whose price was typed by an Admin rather than ingested from an upstream catalog.
///
/// `models_dev` and `catalog` rows carry upstream list prices, which are published in USD.
/// `manual` rows were entered by hand against a form labelled USD but denominated in the
/// operator's own currency, so they are the rows this migration re-labels.
const MANUAL_SOURCE: &str = "manual";

/// Id prefix of a rate mirrored from `model_metadata_records`.
///
/// A mirror row inherits the metadata row's `source`, so it reads as `manual` after an Admin
/// edits the metadata. Its price is still nano-USD from the Models.dev catalogue, so it is
/// excluded from the CNY re-labelling (MB-D3d).
const METADATA_MIRROR_ID_PREFIX: &str = "model_metadata:";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let tx = manager.get_connection().begin().await?;

        // The stored integer is nano-units per token. It was never USD-specific arithmetic —
        // only the name asserted a currency — so the column is renamed rather than rebuilt.
        // `RENAME COLUMN` rewrites the migration-019 lookup index in place on both backends.
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE billing_rate_records
             RENAME COLUMN unit_price_nano_usd TO unit_price_nano"
                .to_string(),
        ))
        .await?;

        // The default exists so the NOT NULL column can be added to a populated table, and it
        // encodes the correct reading for the only bulk ingest path: upstream catalogues quote
        // USD. Every application write sets the column explicitly.
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE billing_rate_records
             ADD COLUMN unit_price_currency TEXT NOT NULL DEFAULT 'USD'
             CHECK (unit_price_currency IN ('USD', 'CNY'))"
                .to_string(),
        ))
        .await?;

        // MB-D3e: a manual rate keeps its exact numeric value and only gains its true
        // currency. No exchange rate is applied, because the number was already entered as CNY.
        tx.execute(Statement::from_string(
            backend,
            format!(
                "UPDATE billing_rate_records SET unit_price_currency = 'CNY'
                 WHERE source = '{MANUAL_SOURCE}'
                   AND id NOT LIKE '{METADATA_MIRROR_ID_PREFIX}%'"
            ),
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

        // Reverting destroys the currency distinction: a CNY rate becomes a USD rate carrying
        // the same number, which is exactly the mislabelling this migration corrected. The
        // numeric values are preserved, so re-applying the migration restores the correct
        // state without any data loss.
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE billing_rate_records DROP COLUMN unit_price_currency".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE billing_rate_records
             RENAME COLUMN unit_price_nano TO unit_price_nano_usd"
                .to_string(),
        ))
        .await?;

        tx.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::Migration;
    use sea_orm::{
        ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, TryGetable,
    };
    use sea_orm_migration::prelude::*;

    async fn database_before_066() -> DatabaseConnection {
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

    async fn seed(db: &DatabaseConnection, id: &str, source: &str, price: &str) {
        db.execute_unprepared(&format!(
            "INSERT INTO billing_rate_records
                (id, source, pricing_profile, rate_kind, usage_class, unit,
                 unit_price_nano_usd, match_json, priority, enabled, raw_json, updated_at)
             VALUES ('{id}', '{source}', 'profile', 'token', 'input_uncached', 'token',
                     '{price}', '{{}}', 0, 1, '{{}}', '2026-09-09T00:00:00Z')"
        ))
        .await
        .expect("seed rate");
    }

    async fn rows(db: &DatabaseConnection, sql: &str) -> Vec<(String, String, String)> {
        db.query_all(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .expect("query")
            .iter()
            .map(|row| {
                (
                    String::try_get(row, "", "id").expect("id"),
                    String::try_get(row, "", "unit_price_nano").expect("price"),
                    String::try_get(row, "", "unit_price_currency").expect("currency"),
                )
            })
            .collect()
    }

    /// MB-D3e: a manual rate keeps its exact number and becomes CNY. An ingested rate keeps
    /// its number and stays USD. No exchange rate is applied to either.
    ///
    /// MB-D3d: a `model_metadata:` mirror row reads as `manual` once an Admin edits the
    /// metadata, but its price came from the Models.dev USD catalogue, so re-labelling it
    /// would silently multiply the operator's price by the exchange rate.
    #[tokio::test]
    async fn manual_rates_become_cny_at_the_same_number() {
        let db = database_before_066().await;
        seed(&db, "manual-1", "manual", "9000").await;
        seed(&db, "models-1", "models_dev", "3000").await;
        seed(&db, "catalog-1", "catalog", "15000").await;
        seed(&db, "model_metadata:gpt-5:output", "manual", "60000").await;

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 066");

        let mut observed = rows(
            &db,
            "SELECT id, unit_price_nano, unit_price_currency
             FROM billing_rate_records ORDER BY id",
        )
        .await;
        observed.sort();
        assert_eq!(
            observed,
            vec![
                ("catalog-1".to_string(), "15000".to_string(), "USD".to_string()),
                ("manual-1".to_string(), "9000".to_string(), "CNY".to_string()),
                (
                    "model_metadata:gpt-5:output".to_string(),
                    "60000".to_string(),
                    "USD".to_string()
                ),
                ("models-1".to_string(), "3000".to_string(), "USD".to_string()),
            ]
        );
    }

    /// The rename must carry the migration-019 lookup index across, or every rate resolution
    /// degrades to a full table scan.
    #[tokio::test]
    async fn rename_preserves_the_lookup_index_and_rejects_a_bad_currency() {
        let db = database_before_066().await;
        seed(&db, "manual-1", "manual", "9000").await;

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 066");

        let indexes = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'index'
                 AND tbl_name = 'billing_rate_records'"
                    .to_string(),
            ))
            .await
            .expect("query indexes")
            .iter()
            .map(|row| String::try_get(row, "", "name").expect("index name"))
            .collect::<Vec<_>>();
        assert!(
            indexes
                .iter()
                .any(|name| name == "idx_billing_rate_records_lookup"),
            "lookup index missing after rename: {indexes:?}"
        );

        db.execute_unprepared(
            "UPDATE billing_rate_records SET unit_price_currency = 'EUR' WHERE id = 'manual-1'",
        )
        .await
        .expect_err("an unsupported currency must fail the CHECK");
    }

    /// Reverting restores the original column name and preserves every numeric value, so the
    /// migration can be re-applied to recover the currency labels.
    #[tokio::test]
    async fn revert_restores_the_column_name_and_keeps_values() {
        let db = database_before_066().await;
        seed(&db, "manual-1", "manual", "9000").await;
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 066");

        Migration
            .down(&SchemaManager::new(&db))
            .await
            .expect("revert 066");

        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT unit_price_nano_usd AS p FROM billing_rate_records WHERE id = 'manual-1'"
                    .to_string(),
            ))
            .await
            .expect("query")
            .expect("one row");
        assert_eq!(String::try_get(&row, "", "p").expect("price"), "9000");

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("re-apply 066");
        assert_eq!(
            rows(
                &db,
                "SELECT id, unit_price_nano, unit_price_currency FROM billing_rate_records"
            )
            .await,
            vec![("manual-1".to_string(), "9000".to_string(), "CNY".to_string())]
        );
    }
}
