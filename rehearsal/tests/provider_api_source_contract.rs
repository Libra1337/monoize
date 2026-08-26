const APP_SOURCE: &str = include_str!("../../src/app.rs");
const HANDLER_SOURCE: &str = include_str!("../../src/dashboard_handlers/providers.rs");
const FRONTEND_API_SOURCE: &str = include_str!("../../frontend/src/lib/api.ts");
const ROUTING_SOURCE: &str = include_str!("../../src/monoize_routing.rs");
const GROUP_STORE_SOURCE: &str = include_str!("../../src/users/groups.rs");
const GROUP_HANDLER_SOURCE: &str = include_str!("../../src/dashboard_handlers/groups.rs");
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
        let start = ROUTING_SOURCE.find(declaration).expect("Provider declaration");
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
        let start = ROUTING_SOURCE.find(declaration).expect("Provider declaration");
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
