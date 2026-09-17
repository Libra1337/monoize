use sea_orm::DeriveRelation;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "announcement_reads")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub user_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub announcement_id: String,
    #[sea_orm(column_type = "Text")]
    pub read_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
