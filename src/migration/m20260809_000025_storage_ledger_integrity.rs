use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let conn = manager.get_connection();

        match backend {
            DbBackend::Sqlite => {
                let tx = conn.begin().await?;
                let result: Result<(), DbErr> = async {
                    for sql in [
                        "DROP TABLE IF EXISTS billing_ledger_without_user_fk",
                        "CREATE TABLE billing_ledger_without_user_fk (id TEXT NOT NULL PRIMARY KEY, user_id TEXT NOT NULL, kind TEXT NOT NULL, delta_nano_usd TEXT NOT NULL, balance_after_nano_usd TEXT, meta_json TEXT NOT NULL, created_at TEXT NOT NULL)",
                        "INSERT INTO billing_ledger_without_user_fk (id, user_id, kind, delta_nano_usd, balance_after_nano_usd, meta_json, created_at) SELECT id, user_id, kind, delta_nano_usd, balance_after_nano_usd, meta_json, created_at FROM billing_ledger",
                        "DROP TABLE billing_ledger",
                        "ALTER TABLE billing_ledger_without_user_fk RENAME TO billing_ledger",
                        "CREATE INDEX IF NOT EXISTS idx_billing_ledger_user_id ON billing_ledger (user_id)",
                    ] {
                        tx.execute(Statement::from_string(backend, sql.to_owned()))
                            .await?;
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    let _ = tx.rollback().await;
                    return Err(error);
                }
                tx.commit().await?;
            }
            DbBackend::Postgres => {
                let tx = conn.begin().await?;
                let result: Result<(), DbErr> = async {
                    tx.execute(Statement::from_string(
                        backend,
                        "ALTER TABLE billing_ledger DROP CONSTRAINT IF EXISTS fk_billing_ledger_user_id"
                            .to_string(),
                    ))
                    .await?;

                    for column in [
                        "input_tokens",
                        "output_tokens",
                        "cache_read_tokens",
                        "cache_creation_tokens",
                        "tool_prompt_tokens",
                        "reasoning_tokens",
                        "accepted_prediction_tokens",
                        "rejected_prediction_tokens",
                        "error_http_status",
                        "duration_ms",
                        "ttfb_ms",
                        "first_visible_output_ms",
                        "last_visible_output_ms",
                        "visible_generation_ms",
                        "visible_output_tokens",
                    ] {
                        tx.execute(Statement::from_string(
                            backend,
                            format!(
                                "ALTER TABLE request_logs ALTER COLUMN {column} TYPE BIGINT USING {column}::BIGINT"
                            ),
                        ))
                        .await?;
                    }

                    for column in ["max_input_tokens", "max_output_tokens", "max_tokens"] {
                        tx.execute(Statement::from_string(
                            backend,
                            format!(
                                "ALTER TABLE model_metadata_records ALTER COLUMN {column} TYPE BIGINT USING {column}::BIGINT"
                            ),
                        ))
                        .await?;
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    let _ = tx.rollback().await;
                    return Err(error);
                }
                tx.commit().await?;
            }
            _ => {}
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
