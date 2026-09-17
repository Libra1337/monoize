use sea_orm::DeriveRelation;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "admin_revenue_daily_user_model_rows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_type = "Text")]
    pub day: String,
    #[sea_orm(column_type = "Text")]
    pub user_id: String,
    #[sea_orm(column_type = "Text")]
    pub model: String,
    #[sea_orm(default_value = "0", column_type = "Text")]
    pub charge_nano_usd: String,
    #[sea_orm(default_value = 0)]
    pub calls: i64,
    #[sea_orm(default_value = 0)]
    pub input_tokens: i64,
    #[sea_orm(default_value = 0)]
    pub output_tokens: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
