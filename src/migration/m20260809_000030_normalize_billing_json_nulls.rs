use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let Some(sql) = normalization_sql(backend) else {
            return Ok(());
        };

        manager
            .get_connection()
            .execute(Statement::from_string(backend, sql))
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

pub(crate) fn normalization_sql(backend: DbBackend) -> Option<String> {
    let (trim_function, character_function) = match backend {
        DbBackend::Sqlite => ("trim", "char"),
        DbBackend::Postgres => ("btrim", "chr"),
        _ => return None,
    };
    let whitespace = format!(
        "{character_function}(9) || {character_function}(10) || {character_function}(13) || {character_function}(32)"
    );
    let normalized_match = format!("{trim_function}(match_json, {whitespace}) = 'null'");
    let normalized_raw = format!("{trim_function}(raw_json, {whitespace}) = 'null'");

    Some(format!(
        "UPDATE billing_rate_records \
         SET match_json = CASE WHEN {normalized_match} THEN '{{}}' ELSE match_json END, \
             raw_json = CASE WHEN {normalized_raw} THEN '{{}}' ELSE raw_json END \
         WHERE {normalized_match} OR {normalized_raw}"
    ))
}
