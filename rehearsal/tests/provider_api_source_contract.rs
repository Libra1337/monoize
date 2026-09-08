const APP_SOURCE: &str = include_str!("../../src/app.rs");
const HANDLER_SOURCE: &str = include_str!("../../src/dashboard_handlers/providers.rs");
const FRONTEND_API_SOURCE: &str = include_str!("../../frontend/src/lib/api.ts");
const ROUTING_SOURCE: &str = include_str!("../../src/monoize_routing.rs");
const GROUP_STORE_SOURCE: &str = include_str!("../../src/users/groups.rs");
const GROUP_HANDLER_SOURCE: &str = include_str!("../../src/dashboard_handlers/groups.rs");
const PUBLIC_HANDLER_SOURCE: &str = include_str!("../../src/public_handlers.rs");
const PUBLIC_NAME_SOURCE: &str = include_str!("../../src/public_name.rs");
const ENTITY_MODULE_SOURCE: &str = include_str!("../../src/entity/mod.rs");

#[test]
fn channel_test_uses_only_the_singular_provider_route() {
    assert!(APP_SOURCE.contains("\"/dashboard/providers/{provider_id}/channel/test\""));
    assert!(
        !APP_SOURCE.contains("\"/dashboard/providers/{provider_id}/channels/{channel_id}/test\"")
    );
    assert!(HANDLER_SOURCE.contains("Path(provider_id): Path<String>"));
    assert!(!HANDLER_SOURCE.contains("Path((provider_id, channel_id)): Path<(String, String)>"));
    assert!(FRONTEND_API_SOURCE.contains("`/providers/${providerId}/channel/test`"));
    assert!(!FRONTEND_API_SOURCE.contains("`/providers/${providerId}/channels/${channelId}/test`"));
}

#[test]
fn provider_reorder_is_scoped_to_one_group() {
    let contract_start = ROUTING_SOURCE
        .find("pub struct ReorderProvidersInput")
        .expect("reorder contract exists");
    let contract = &ROUTING_SOURCE[contract_start..contract_start + 160];
    assert!(contract.contains("pub group_id: String"));
    assert!(contract.contains("pub provider_ids: Vec<String>"));
    assert!(
        FRONTEND_API_SOURCE
            .contains("body: JSON.stringify({ group_id: groupId, provider_ids: providerIds })")
    );
}

#[test]
fn provider_retry_field_is_removed_from_management_and_routing_contracts() {
    assert!(!ROUTING_SOURCE.contains("pub max_retries:"));
    assert!(!ROUTING_SOURCE.contains("input.max_retries"));
    assert!(!ROUTING_SOURCE.contains("{p}max_retries"));
    assert!(!FRONTEND_API_SOURCE.contains("\n  max_retries: number;"));
    assert!(!include_str!("../../src/handlers/mod.rs").contains("provider_budget_remaining"));
}

#[test]
fn provider_group_contract_is_singular() {
    for declaration in [
        "pub struct MonoizeProvider",
        "pub struct CreateMonoizeProviderInput",
        "pub struct UpdateMonoizeProviderInput",
    ] {
        let start = ROUTING_SOURCE
            .find(declaration)
            .expect("Provider declaration");
        let body = &ROUTING_SOURCE[start..start + 1_800];
        assert!(body.contains("pub group_id:"));
        assert!(!body.contains("pub group_ids:"));
    }
    let provider_start = FRONTEND_API_SOURCE
        .find("export interface Provider {")
        .expect("frontend Provider declaration");
    let provider = &FRONTEND_API_SOURCE[provider_start..provider_start + 1_200];
    assert!(provider.contains("group_id: string;"));
    assert!(!provider.contains("group_ids: string[];"));
}

#[test]
fn provider_runtime_uses_only_the_flattened_storage_contract() {
    for obsolete in [
        "monoize_channels",
        "monoize_channel_models",
        "uses_flattened_provider_schema",
        "load_flattened_channels_bulk",
    ] {
        assert!(
            !ROUTING_SOURCE.contains(obsolete),
            "runtime routing source still contains obsolete storage path: {obsolete}"
        );
    }
}

#[test]
fn referenced_provider_group_delete_returns_conflict() {
    assert!(GROUP_STORE_SOURCE.contains("GroupStoreError::GroupInUse"));
    assert!(GROUP_STORE_SOURCE.contains("WHERE group_id = $1 LIMIT 1"));
    assert!(!GROUP_STORE_SOURCE.contains("monoize_providers SET group_ids"));
    assert!(GROUP_HANDLER_SOURCE.contains("\"group_in_use\""));
    assert!(GROUP_HANDLER_SOURCE.contains("StatusCode::CONFLICT"));
}

#[test]
fn obsolete_channel_entities_are_not_exported() {
    assert!(!ENTITY_MODULE_SOURCE.contains("monoize_channels"));
    assert!(!ENTITY_MODULE_SOURCE.contains("monoize_channel_models"));
}

#[test]
fn provider_channel_contract_is_singular() {
    for declaration in [
        "pub struct MonoizeProvider",
        "pub struct CreateMonoizeProviderInput",
        "pub struct UpdateMonoizeProviderInput",
    ] {
        let start = ROUTING_SOURCE
            .find(declaration)
            .expect("Provider declaration");
        let body = &ROUTING_SOURCE[start..start + 1_800];
        assert!(body.contains("pub channel:"));
        assert!(!body.contains("pub channels:"));
    }
    let provider_start = FRONTEND_API_SOURCE
        .find("export interface Provider {")
        .expect("frontend Provider declaration");
    let provider = &FRONTEND_API_SOURCE[provider_start..provider_start + 1_200];
    assert!(provider.contains("channel: MonoizeChannel;"));
    assert!(!provider.contains("channels: MonoizeChannel[];"));
}

#[test]
fn channel_weight_is_removed_from_management_and_runtime_contracts() {
    for declaration in [
        "pub struct MonoizeChannel",
        "pub struct CreateMonoizeChannelInput",
    ] {
        let start = ROUTING_SOURCE
            .find(declaration)
            .expect("Channel declaration");
        let body = &ROUTING_SOURCE[start..start + 1_800];
        assert!(!body.contains("pub weight:"));
    }
    let frontend_start = FRONTEND_API_SOURCE
        .find("export interface MonoizeChannel {")
        .expect("frontend Channel declaration");
    let frontend = &FRONTEND_API_SOURCE[frontend_start..frontend_start + 1_800];
    assert!(!frontend.contains("weight:"));
    assert!(!frontend.contains("weight?:"));
}

#[test]
fn target_pricing_contract_is_exposed_without_legacy_model_multiplier() {
    let provider_start = ROUTING_SOURCE
        .find("pub struct MonoizeProvider")
        .expect("Provider declaration");
    let provider = &ROUTING_SOURCE[provider_start..provider_start + 1_800];
    assert!(provider.contains("pub pricing_profile: Option<String>"));
    assert!(provider.contains("pub multiplier: Multiplier"));

    let model_start = ROUTING_SOURCE
        .find("pub struct MonoizeModelEntry")
        .expect("model declaration");
    let model = &ROUTING_SOURCE[model_start..model_start + 700];
    let model_attributes = &ROUTING_SOURCE[model_start.saturating_sub(120)..model_start];
    assert!(model_attributes.contains("#[serde(deny_unknown_fields)]"));
    assert!(model.contains("pub pricing_profile_mode:"));
    assert!(model.contains("pub pricing_profile_override: Option<String>"));
    assert!(model.contains("pub multiplier_override: Option<Multiplier>"));
    assert!(!model.contains("pub multiplier: Multiplier"));
}

#[test]
fn provider_channel_allows_zero_model_mappings() {
    assert!(!ROUTING_SOURCE.contains("the channel must define at least one model"));
}

#[test]
fn production_model_writes_use_the_validated_model_name_contract() {
    assert!(ROUTING_SOURCE.contains("fn canonical_model_name("));
    assert!(ROUTING_SOURCE.contains("canonical_model_name(model)?"));
    assert!(ROUTING_SOURCE.contains("canonicalize_models(&channel.models)?"));
}

#[test]
fn per_model_active_probe_pricing_snapshot_covers_every_model() {
    let start = APP_SOURCE
        .find("async fn build_active_probe_pricing_snapshot")
        .expect("active probe snapshot builder");
    let body = &APP_SOURCE[start..start + 6_000];
    assert!(body.contains("for (logical_model, model_entry) in &channel.models"));
}

#[test]
fn management_writes_require_confirmed_canonical_public_names() {
    assert!(
        ROUTING_SOURCE
            .matches("pub confirm_public_exposure: bool")
            .count()
            >= 2
    );
    assert!(
        GROUP_STORE_SOURCE
            .matches("pub confirm_public_exposure: bool")
            .count()
            >= 2
    );
    assert!(ROUTING_SOURCE.contains("canonicalize_public_name"));
    assert!(GROUP_STORE_SOURCE.contains("canonicalize_public_name"));
    assert!(PUBLIC_NAME_SOURCE.contains(".nfc()"));
    assert!(PUBLIC_NAME_SOURCE.contains("1..=64"));
}

#[test]
fn public_surfaces_read_only_persisted_public_names() {
    assert!(PUBLIC_HANDLER_SOURCE.contains("public_provider_names_by_id"));
    assert!(PUBLIC_HANDLER_SOURCE.contains("public_name AS group_public_name"));
    assert!(!PUBLIC_HANDLER_SOURCE.contains("public_provider_name: provider.name"));
    assert!(!PUBLIC_HANDLER_SOURCE.contains("public_channel_name: channel.name"));
    assert!(!PUBLIC_HANDLER_SOURCE.contains("public_name: provider.name"));
}

#[test]
fn public_source_errors_are_sanitized_before_serialization() {
    assert!(PUBLIC_HANDLER_SOURCE.contains("fn marketplace_source_error("));
    assert!(PUBLIC_HANDLER_SOURCE.contains("fn status_source_error("));
    assert!(
        !PUBLIC_HANDLER_SOURCE.contains(
            "AppError::new(StatusCode::INTERNAL_SERVER_ERROR, \"internal_error\", error)"
        )
    );
    assert!(!PUBLIC_HANDLER_SOURCE.contains(
        "AppError::new(StatusCode::INTERNAL_SERVER_ERROR, \"internal_error\", error.to_string())"
    ));
}

#[test]
fn provider_write_responses_include_structured_pricing_warnings() {
    assert!(HANDLER_SOURCE.contains("struct PricingWarning"));
    assert!(HANDLER_SOURCE.contains("logical_model: String"));
    assert!(HANDLER_SOURCE.contains("missing_usage_classes: Vec<String>"));
    assert!(HANDLER_SOURCE.contains("\"pricing_warnings\""));
    assert!(FRONTEND_API_SOURCE.contains("pricing_warnings?: ProviderPricingWarning[]"));
}

#[test]
fn new_provider_and_channel_ids_use_uuid_v4_and_channel_input_has_no_id() {
    assert!(ROUTING_SOURCE.contains("fn generate_provider_id()"));
    assert!(ROUTING_SOURCE.contains("let id = generate_provider_id();"));
    assert!(ROUTING_SOURCE.contains("fn generate_channel_id()"));
    assert!(
        ROUTING_SOURCE
            .matches("uuid::Uuid::new_v4().to_string()")
            .count()
            >= 2
    );
    assert!(!ROUTING_SOURCE.contains("fn generate_short_id()"));
    let channel_input_start = ROUTING_SOURCE
        .find("pub struct CreateMonoizeChannelInput")
        .expect("Channel input declaration");
    let channel_input = &ROUTING_SOURCE[channel_input_start..channel_input_start + 1_800];
    assert!(!channel_input.contains("pub id:"));
    assert!(!channel_input.contains("skip_deserializing"));
}
