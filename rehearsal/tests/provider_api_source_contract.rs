const APP_SOURCE: &str = include_str!("../../src/app.rs");
const HANDLER_SOURCE: &str = include_str!("../../src/dashboard_handlers/providers.rs");
const FRONTEND_API_SOURCE: &str = include_str!("../../frontend/src/lib/api.ts");
const ROUTING_SOURCE: &str = include_str!("../../src/monoize_routing.rs");

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
