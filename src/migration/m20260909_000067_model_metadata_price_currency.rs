use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Metadata rows whose prices were typed by an Admin rather than ingested from Models.dev.
///
/// Models.dev sync skips a `manual` row entirely (`model-metadata-dashboard.spec.md` SP1),
/// so a row carrying this source has had its prices set by hand against a form that was
/// labelled USD while the operator was pricing in CNY. Those are the rows relabelled here.
const MANUAL_SOURCE: &str = "manual";

/// Id prefix of a `billing_rate_records` row mirrored from `model_metadata_records`.
const METADATA_MIRROR_ID_PREFIX: &str = "model_metadata:";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let tx = manager.get_connection().begin().await?;

        // The default covers the bulk ingest path, which is Models.dev and therefore USD
        // (`user-billing-and-model-metadata.spec.md` S4). Every application write is explicit.
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE model_metadata_records
             ADD COLUMN price_currency TEXT NOT NULL DEFAULT 'USD'
             CHECK (price_currency IN ('USD', 'CNY'))"
                .to_string(),
        ))
        .await?;

        // MD10a: a manual row keeps every price digit and only gains its true currency. No
        // exchange rate is applied, because the number was already entered as CNY.
        tx.execute(Statement::from_string(
            backend,
            format!(
                "UPDATE model_metadata_records SET price_currency = 'CNY'
                 WHERE source = '{MANUAL_SOURCE}'"
            ),
        ))
        .await?;

        // MB-D3f: migration 066 left every mirror row as USD because the metadata layer had no
        // currency to follow. It has one now, so each mirror adopts the denomination of the row
        // it copied its digits from. A mirror whose metadata row has since been deleted keeps
        // its current value rather than being guessed at, hence COALESCE.
        tx.execute(Statement::from_string(
            backend,
            format!(
                "UPDATE billing_rate_records
                 SET unit_price_currency = COALESCE((
                     SELECT m.price_currency FROM model_metadata_records m
                     WHERE billing_rate_records.id =
                         '{METADATA_MIRROR_ID_PREFIX}' || m.model_id || ':'
                         || billing_rate_records.usage_class
                 ), unit_price_currency)
                 WHERE id LIKE '{METADATA_MIRROR_ID_PREFIX}%'"
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

        // Restore the post-066 reading of a mirror row before the currency source disappears.
        tx.execute(Statement::from_string(
            backend,
            format!(
                "UPDATE billing_rate_records SET unit_price_currency = 'USD'
                 WHERE id LIKE '{METADATA_MIRROR_ID_PREFIX}%'"
            ),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE model_metadata_records DROP COLUMN price_currency".to_string(),
        ))
        .await?;

        tx.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::Migration;
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, TryGetable};
    use sea_orm_migration::prelude::*;

    async fn database_before_067() -> DatabaseConnection {
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

    async fn seed_metadata(db: &DatabaseConnection, model_id: &str, source: &str, input: &str) {
        db.execute_unprepared(&format!(
            "INSERT INTO model_metadata_records
                (model_id, input_cost_per_token_nano, output_cost_per_token_nano,
                 raw_json, source, updated_at)
             VALUES ('{model_id}', '{input}', '{input}', '{{}}', '{source}',
                     '2026-09-09T00:00:00Z')"
        ))
        .await
        .expect("seed metadata");
    }

    async fn seed_rate(db: &DatabaseConnection, id: &str, source: &str, currency: &str) {
        db.execute_unprepared(&format!(
            "INSERT INTO billing_rate_records
                (id, source, pricing_profile, rate_kind, usage_class, unit,
                 unit_price_nano, unit_price_currency, match_json, priority, enabled,
                 raw_json, updated_at)
             VALUES ('{id}', '{source}', 'profile', 'token', 'input_uncached', 'token',
                     '5000', '{currency}', '{{}}', 0, 1, '{{}}', '2026-09-09T00:00:00Z')"
        ))
        .await
        .expect("seed rate");
    }

    async fn metadata_currencies(db: &DatabaseConnection) -> Vec<(String, String, String)> {
        db.query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT model_id, input_cost_per_token_nano AS price, price_currency
             FROM model_metadata_records ORDER BY model_id"
                .to_string(),
        ))
        .await
        .expect("query metadata")
        .iter()
        .map(|row| {
            (
                String::try_get(row, "", "model_id").expect("model_id"),
                String::try_get(row, "", "price").expect("price"),
                String::try_get(row, "", "price_currency").expect("currency"),
            )
        })
        .collect()
    }

    async fn rate_currencies(db: &DatabaseConnection) -> Vec<(String, String, String)> {
        db.query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id, unit_price_nano AS price, unit_price_currency
             FROM billing_rate_records ORDER BY id"
                .to_string(),
        ))
        .await
        .expect("query rates")
        .iter()
        .map(|row| {
            (
                String::try_get(row, "", "id").expect("id"),
                String::try_get(row, "", "price").expect("price"),
                String::try_get(row, "", "unit_price_currency").expect("currency"),
            )
        })
        .collect()
    }

    /// MD10a: a manually priced metadata row becomes CNY at the same number, a synced row
    /// stays USD, and each mirrored rate adopts the currency of its own metadata row.
    #[tokio::test]
    async fn manual_metadata_becomes_cny_and_mirrors_follow() {
        let db = database_before_067().await;
        seed_metadata(&db, "kimi-k3", "manual", "20000").await;
        seed_metadata(&db, "gpt-4o", "models_dev", "2500").await;
        // Migration 066 left every mirror as USD, which is the state this one corrects.
        seed_rate(&db, "model_metadata:kimi-k3:input_uncached", "manual", "USD").await;
        seed_rate(&db, "model_metadata:gpt-4o:input_uncached", "models_dev", "USD").await;
        // A mirror whose metadata row no longer exists must keep its stored value.
        seed_rate(&db, "model_metadata:removed:input_uncached", "manual", "USD").await;
        // A rate outside the mirror namespace must not be touched at all.
        seed_rate(&db, "manual:openai:gpt-6:input_uncached", "manual", "CNY").await;

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 067");

        assert_eq!(
            metadata_currencies(&db).await,
            vec![
                ("gpt-4o".to_string(), "2500".to_string(), "USD".to_string()),
                ("kimi-k3".to_string(), "20000".to_string(), "CNY".to_string()),
            ]
        );
        assert_eq!(
            rate_currencies(&db).await,
            vec![
                (
                    "manual:openai:gpt-6:input_uncached".to_string(),
                    "5000".to_string(),
                    "CNY".to_string()
                ),
                (
                    "model_metadata:gpt-4o:input_uncached".to_string(),
                    "5000".to_string(),
                    "USD".to_string()
                ),
                (
                    "model_metadata:kimi-k3:input_uncached".to_string(),
                    "5000".to_string(),
                    "CNY".to_string()
                ),
                (
                    "model_metadata:removed:input_uncached".to_string(),
                    "5000".to_string(),
                    "USD".to_string()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn metadata_currency_check_rejects_an_unsupported_value() {
        let db = database_before_067().await;
        seed_metadata(&db, "kimi-k3", "manual", "20000").await;
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 067");

        db.execute_unprepared(
            "UPDATE model_metadata_records SET price_currency = 'EUR' WHERE model_id = 'kimi-k3'",
        )
        .await
        .expect_err("an unsupported currency must fail the CHECK");
    }

    /// Reverting drops the currency column, so a mirror row has nothing to follow and returns
    /// to the post-066 reading. Every price digit survives both directions.
    #[tokio::test]
    async fn revert_restores_the_post_066_state() {
        let db = database_before_067().await;
        seed_metadata(&db, "kimi-k3", "manual", "20000").await;
        seed_rate(&db, "model_metadata:kimi-k3:input_uncached", "manual", "USD").await;

        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("apply 067");
        Migration.down(&manager).await.expect("revert 067");

        assert_eq!(
            rate_currencies(&db).await,
            vec![(
                "model_metadata:kimi-k3:input_uncached".to_string(),
                "5000".to_string(),
                "USD".to_string()
            )]
        );

        Migration.up(&manager).await.expect("re-apply 067");
        assert_eq!(
            rate_currencies(&db).await,
            vec![(
                "model_metadata:kimi-k3:input_uncached".to_string(),
                "5000".to_string(),
                "CNY".to_string()
            )]
        );
    }
}
