use sea_orm::DeriveRelation;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "firewall_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_type = "Text")]
    pub user_id: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub username: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub api_key_id: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub api_key_name: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub endpoint: String,
    #[sea_orm(column_type = "Text")]
    pub model: String,
    #[sea_orm(column_type = "Text")]
    pub term: String,
    #[sea_orm(column_type = "Text")]
    pub content: String,
    #[sea_orm(default_value = "blocked", column_type = "Text")]
    pub action: String,
    #[sea_orm(column_type = "Text")]
    pub created_at: String,
    pub created_at_unix_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl sea_orm::ActiveModelBehavior for ActiveModel {}
