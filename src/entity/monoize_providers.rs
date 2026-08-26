use sea_orm::DeriveRelation;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "monoize_providers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    #[sea_orm(column_type = "Text")]
    pub group_id: String,
    #[sea_orm(column_type = "Text")]
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub public_name: String,
    #[sea_orm(column_type = "Blob")]
    pub public_name_key: Vec<u8>,
    pub priority: i32,
    pub enabled: i32,
    #[sea_orm(column_type = "Text")]
    pub pricing_profile: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub multiplier: String,
    pub configuration_generation: i64,
    #[sea_orm(column_type = "Text")]
    pub created_at: String,
    #[sea_orm(unique)]
    #[sea_orm(column_type = "Text")]
    pub channel_id: String,
    #[sea_orm(column_type = "Text")]
    pub channel_name: String,
    #[sea_orm(column_type = "Text")]
    pub channel_public_name: String,
    #[sea_orm(column_type = "Blob")]
    pub channel_public_name_key: Vec<u8>,
    #[sea_orm(column_type = "Text")]
    pub channel_provider_type: String,
    #[sea_orm(column_type = "Text")]
    pub channel_base_url: String,
    #[sea_orm(column_type = "Text")]
    pub channel_api_key: String,
    pub channel_enabled: i32,
    pub channel_max_retries: i32,
    pub transforms: String,
    pub api_type_overrides: String,
    pub active_probe_enabled_override: Option<i32>,
    pub active_probe_interval_seconds_override: Option<i32>,
    pub active_probe_success_threshold_override: Option<i32>,
    pub active_probe_model_override: Option<String>,
    pub request_timeout_ms_override: Option<i32>,
    pub extra_fields_whitelist: Option<String>,
    pub strip_cross_protocol_nested_extra: Option<i32>,
    pub circuit_breaker_enabled: i32,
    pub per_model_circuit_break: i32,
    pub channel_retry_interval_ms: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl sea_orm::ActiveModelBehavior for ActiveModel {}
