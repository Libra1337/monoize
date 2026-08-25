use monoize_lynshen_rehearsal::provider::{CanonicalDecimal, ModelKeys};

#[test]
fn canonical_multiplier_normalizes_without_binary_float() {
    assert_eq!(
        CanonicalDecimal::parse("1.2000")
            .expect("valid decimal")
            .as_str(),
        "1.2"
    );
    assert_eq!(
        CanonicalDecimal::parse("0.000000001")
            .expect("nine fractional digits")
            .as_str(),
        "0.000000001"
    );
}

#[test]
fn canonical_multiplier_rejects_invalid_shapes() {
    for invalid in ["0", "-1", "+1", "1e0", "NaN", "1.0000000001"] {
        assert!(
            CanonicalDecimal::parse(invalid).is_err(),
            "accepted invalid decimal {invalid}"
        );
    }
}

#[test]
fn model_keys_are_trimmed_exact_and_ascii_folded() {
    let keys = ModelKeys::new("  GPT-4o-模型  ").expect("valid model");
    assert_eq!(keys.model_name, "GPT-4o-模型");
    assert_eq!(keys.name, "GPT-4o-模型".as_bytes());
    assert_eq!(keys.search, "gpt-4o-模型".as_bytes());
}

#[test]
fn model_keys_reject_controls_and_oversized_names() {
    assert!(ModelKeys::new("gpt\t4").is_err());
    assert!(ModelKeys::new("\u{7f}").is_err());
    assert!(ModelKeys::new(&"a".repeat(257)).is_err());
}
