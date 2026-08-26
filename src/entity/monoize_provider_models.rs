use sea_orm::DeriveRelation;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "monoize_provider_models")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub provider_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    #[sea_orm(column_type = "Blob")]
    pub model_name_key: Vec<u8>,
    #[sea_orm(column_type = "Text")]
    pub model_name: String,
    #[sea_orm(column_type = "Blob")]
    pub model_search_key: Vec<u8>,
    #[sea_orm(column_type = "Text")]
    pub redirect: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub pricing_profile_mode: String,
    #[sea_orm(column_type = "Text")]
    pub pricing_profile_override: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub multiplier_override: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl sea_orm::ActiveModelBehavior for ActiveModel {}
