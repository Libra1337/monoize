use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// SB-Q-1B-1: a `custom` 86400-second quota window is a rolling 24-hour window that
/// must behave as the Asia/Shanghai calendar day of the `day` kind. Existing rules are
/// normalized in place; already-expired rolling buckets need no rewrite because
/// `quota_window` derives every new window from the rule kind.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "UPDATE store_plan_quotas SET window_kind = 'day'
             WHERE window_kind = 'custom' AND window_seconds = 86400",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "UPDATE store_plan_quotas SET window_kind = 'custom'
             WHERE window_kind = 'day' AND window_seconds = 86400",
        )
        .await?;
        Ok(())
    }
}
