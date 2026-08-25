use monoize_lynshen_rehearsal::public_cache::{if_none_match_matches, weak_etag};

#[test]
fn weak_etag_hashes_exact_uncompressed_bytes() {
    assert_eq!(
        weak_etag(br#"{"site_name":"LynShen Console"}"#),
        "W/\"84550d7c0b19571fd44efd1a34b30f89928333d15586ca6e34826a5ccb99f761\""
    );
}

#[test]
fn entity_tag_list_uses_weak_comparison_and_wildcard() {
    let current = "W/\"abc\"";
    assert!(if_none_match_matches("\"def\", W/\"abc\"", current));
    assert!(if_none_match_matches("\"abc\"", current));
    assert!(if_none_match_matches("*", current));
}

#[test]
fn malformed_entity_tag_list_is_ignored() {
    for malformed in ["abc", "W/abc", "\"unterminated", "*, \"abc\""] {
        assert!(!if_none_match_matches(malformed, "W/\"abc\""));
    }
}
