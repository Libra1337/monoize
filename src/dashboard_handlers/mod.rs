mod admin;
mod analytics_request_logs;
mod api_keys;
mod auth;
mod billing_plans;
mod billing_rates;
mod captcha;
mod groups;
mod model_registry;
mod providers;
mod request_captures;
pub(crate) mod session_helpers;
mod settings;
mod store_billing;
mod users;

#[cfg(test)]
mod tests;

pub use admin::{get_admin_overview, get_metrics};
pub use analytics_request_logs::{
    AnalyticsQuery, RequestLogsQuery, get_dashboard_analytics, get_my_live_usage,
    list_my_request_logs, stream_request_logs,
};
pub use api_keys::{
    ApiKeyCreatedResponse, ApiKeyResponse, BatchDeleteApiKeysRequest, CreateApiKeyRequest,
    TransferToSubAccountRequest, UpdateApiKeyRequest, batch_delete_api_keys, create_api_key,
    delete_api_key, get_api_key, get_apikey_presets, list_my_api_keys, transfer_to_sub_account,
    update_api_key,
};
pub use auth::{
    AuthResponse, ChangePasswordRequest, LoginRequest, RegisterRequest, UpdateMeRequest,
    UserBillingPlanResponse, UserResponse, change_password, get_me, login, logout, register,
    update_me, user_response_from_store,
};
pub use billing_plans::{
    BillingPlanResponse, CreateBillingPlanRequest, UpdateBillingPlanRequest, create_billing_plan,
    delete_billing_plan, list_billing_plans, reset_billing_plan, update_billing_plan,
};
pub use billing_rates::{
    PricingProfilePatternsResponse, UpdatePricingProfilePatternsRequest, delete_billing_rate,
    get_pricing_profile_patterns, list_billing_rates, sync_billing_rates_catalog,
    update_pricing_profile_patterns, upsert_billing_rate,
};
pub use captcha::{create_captcha_challenge, redeem_captcha_challenge};
pub use groups::{
    DashboardGroupsResponse, create_group, delete_group, list_dashboard_groups, reorder_groups,
    update_group,
};
pub use model_registry::{
    create_model, delete_model, delete_model_metadata, get_model, get_model_metadata,
    list_marketplace_models, list_model_metadata, list_models, sync_model_metadata_models_dev,
    update_model, upsert_model_metadata,
};
pub(crate) use providers::provider_dashboard_rate_matrix_is_complete;
pub use request_captures::{RequestCaptureQuery, get_request_capture};

pub use providers::{
    FetchChannelModelsRequest, TestChannelRequest, create_provider, delete_provider,
    fetch_channel_models, fetch_provider_models, get_provider, get_provider_presets,
    get_transform_registry, list_providers, reorder_providers, test_channel, update_provider,
};
pub use settings::{
    UpdateSettingsRequest, get_config_overview, get_dashboard_stats, get_public_settings,
    get_public_site_settings, get_settings, update_settings,
};
pub use store_billing::{
    create_store_order, create_store_payment_attempt, create_store_payment_channel_admin,
    create_store_product_admin, create_store_reauth_grant, delete_store_payment_channel_admin,
    delete_store_product_admin, export_store_redemption_codes_admin,
    generate_store_redemption_codes_admin, get_store_catalog, get_store_entitlement,
    get_store_exchange_rate, get_store_order, get_store_payment_icon, get_store_settings_admin,
    list_all_store_orders_admin, list_store_orders, list_store_payment_channels_admin,
    list_store_products_admin, list_store_redemption_codes_admin, redeem_store_code,
    replace_store_payment_credential_admin, reveal_store_redemption_codes_admin,
    revoke_store_redemption_code_admin, update_store_payment_channel_admin,
    update_store_product_admin, update_store_settings_admin, upload_store_payment_icon_admin,
};
pub use users::{
    CreateUserRequest, UpdateUserRequest, create_user, delete_user, get_user, list_users,
    update_user,
};
