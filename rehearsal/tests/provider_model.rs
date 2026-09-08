use monoize_lynshen_rehearsal::provider::{CanonicalDecimal, ModelKeys, PublicNameKey};

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

#[test]
fn public_names_trim_and_normalize_to_nfc_bytes() {
    let composed = PublicNameKey::new("  Cafe\u{301}  ").unwrap();
    assert_eq!(composed.name, "Café");
    assert_eq!(composed.key, "Café".as_bytes());
    assert_eq!(composed, PublicNameKey::new("Café").unwrap());
}

#[test]
fn public_names_reject_controls_empty_and_more_than_64_scalars() {
    for invalid in ["", " \t ", "Na\nme", &"a".repeat(65)] {
        assert!(PublicNameKey::new(invalid).is_err());
    }
}
