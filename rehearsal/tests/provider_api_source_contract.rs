const APP_SOURCE: &str = include_str!("../../src/app.rs");
const HANDLER_SOURCE: &str = include_str!("../../src/dashboard_handlers/providers.rs");
const FRONTEND_API_SOURCE: &str = include_str!("../../frontend/src/lib/api.ts");

#[test]
fn channel_test_uses_only_the_singular_provider_route() {
    assert!(APP_SOURCE.contains(
        "\"/dashboard/providers/{provider_id}/channel/test\""
    ));
    assert!(!APP_SOURCE.contains(
        "\"/dashboard/providers/{provider_id}/channels/{channel_id}/test\""
    ));
    assert!(HANDLER_SOURCE.contains("Path(provider_id): Path<String>"));
    assert!(!HANDLER_SOURCE.contains(
        "Path((provider_id, channel_id)): Path<(String, String)>"
    ));
    assert!(FRONTEND_API_SOURCE.contains(
        "`/providers/${providerId}/channel/test`"
    ));
    assert!(!FRONTEND_API_SOURCE.contains(
        "`/providers/${providerId}/channels/${channelId}/test`"
    ));
}
