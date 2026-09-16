use sea_orm::{DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = manager.get_database_backend();

        for column in ["max_input_tokens", "prompt_cache_incompatible_with_tools"] {
            let sql = match backend {
                DbBackend::Sqlite => {
                    format!(
                        "ALTER TABLE monoize_providers ADD COLUMN {column} INTEGER DEFAULT NULL"
                    )
                }
                DbBackend::Postgres => {
                    format!(
                        "ALTER TABLE monoize_providers ADD COLUMN IF NOT EXISTS {column} INTEGER DEFAULT NULL"
                    )
                }
                _ => continue,
            };
            conn.execute(Statement::from_string(backend, sql)).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = manager.get_database_backend();

        for column in ["max_input_tokens", "prompt_cache_incompatible_with_tools"] {
            let sql = match backend {
                DbBackend::Sqlite => {
                    format!("ALTER TABLE monoize_providers DROP COLUMN {column}")
                }
                DbBackend::Postgres => {
                    format!("ALTER TABLE monoize_providers DROP COLUMN IF EXISTS {column}")
                }
                _ => continue,
            };
            conn.execute(Statement::from_string(backend, sql)).await?;
        }
        Ok(())
    }
}
