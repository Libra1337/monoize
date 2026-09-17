use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// AN-1..AN-3: dashboard announcements and per-user read state.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Announcements::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Announcements::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Announcements::Title).text().not_null())
                    .col(ColumnDef::new(Announcements::Content).text().not_null())
                    .col(
                        ColumnDef::new(Announcements::Type)
                            .text()
                            .not_null()
                            .default("info"),
                    )
                    .col(
                        ColumnDef::new(Announcements::Pinned)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Announcements::Enabled)
                            .integer()
                            .not_null()
                            .default(1),
                    )
                    .col(ColumnDef::new(Announcements::CreatedAt).text().not_null())
                    .col(ColumnDef::new(Announcements::CreatedBy).text().not_null())
                    .to_owned(),
            )
            .await?;

        // SQLite CHECK constraints and DESC indexes need raw SQL on both
        // backends for identical semantics.
        let db = manager.get_connection();
        if db.get_database_backend() == DbBackend::Sqlite {
            db.execute_unprepared(
                "CREATE INDEX idx_announcements_enabled_created \
                 ON announcements (enabled, created_at DESC)",
            )
            .await?;
        } else {
            db.execute_unprepared(
                "ALTER TABLE announcements ADD CONSTRAINT chk_announcements_type \
                 CHECK (type IN ('info','success','warning','error'))",
            )
            .await?;
            db.execute_unprepared(
                "CREATE INDEX idx_announcements_enabled_created \
                 ON announcements (enabled, created_at)",
            )
            .await?;
        }

        manager
            .create_table(
                Table::create()
                    .table(AnnouncementReads::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(AnnouncementReads::UserId).text().not_null())
                    .col(
                        ColumnDef::new(AnnouncementReads::AnnouncementId)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AnnouncementReads::ReadAt).text().not_null())
                    .primary_key(
                        Index::create()
                            .col(AnnouncementReads::UserId)
                            .col(AnnouncementReads::AnnouncementId),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AnnouncementReads::Table).to_owned())
            .await?;
        let db = manager.get_connection();
        db.execute_unprepared("DROP INDEX IF EXISTS idx_announcements_enabled_created")
            .await?;
        manager
            .drop_table(Table::drop().table(Announcements::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Announcements {
    Table,
    Id,
    Title,
    Content,
    Type,
    Pinned,
    Enabled,
    CreatedAt,
    CreatedBy,
}

#[derive(DeriveIden)]
enum AnnouncementReads {
    Table,
    UserId,
    AnnouncementId,
    ReadAt,
}
