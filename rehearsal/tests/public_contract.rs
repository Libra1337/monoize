use monoize_lynshen_rehearsal::public_contract::{
    MarketplaceItem, MarketplaceListResponse, PublicResponseError, SiteResponse,
    encode_marketplace_bounded, encode_public,
};
use serde_json::Value;

fn object_keys(value: &Value) -> Vec<&str> {
    let mut keys = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

#[test]
fn site_response_serializes_exact_allow_list() {
    let bytes = encode_public(&SiteResponse {
        site_name: "LynShen Console".to_owned(),
        site_description: "API service".to_owned(),
        api_base_url: "https://lynshen.org/v1".to_owned(),
    })
    .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        object_keys(&value),
        vec!["api_base_url", "site_description", "site_name"]
    );
}

#[test]
fn marketplace_response_omits_internal_and_secret_fields() {
    let bytes = encode_marketplace_bounded(MarketplaceListResponse {
        generated_at: "2026-08-26T00:00:00.000000Z".to_owned(),
        revision: "42".to_owned(),
        next_cursor: None,
        items: vec![MarketplaceItem {
            public_group_name: "Public".to_owned(),
            model: "gpt-4o".to_owned(),
            capabilities: vec!["chat".to_owned()],
            input_rate_range: None,
            output_rate_range: None,
            offer_count: 1,
        }],
    })
    .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for forbidden in [
        "api_key",
        "base_url",
        "proxy_url",
        "internal_id",
        "pricing_profile",
        "multiplier",
    ] {
        assert!(!text.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn response_at_one_mebibyte_passes_and_larger_response_fails() {
    let base = MarketplaceListResponse {
        generated_at: "2026-08-26T00:00:00.000000Z".to_owned(),
        revision: "1".to_owned(),
        next_cursor: None,
        items: vec![MarketplaceItem {
            public_group_name: String::new(),
            model: "m".to_owned(),
            capabilities: vec![],
            input_rate_range: None,
            output_rate_range: None,
            offer_count: 1,
        }],
    };
    let empty_len = encode_public(&base).unwrap().len();
    let mut exact = base.clone();
    exact.items[0].public_group_name = "a".repeat(1_048_576 - empty_len);
    assert_eq!(encode_marketplace_bounded(exact).unwrap().len(), 1_048_576);

    let mut too_large = base;
    too_large.items[0].public_group_name = "a".repeat(1_048_577 - empty_len);
    assert_eq!(
        encode_marketplace_bounded(too_large).unwrap_err(),
        PublicResponseError::TooLarge
    );
}
