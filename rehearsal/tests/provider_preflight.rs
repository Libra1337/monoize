use monoize_lynshen_rehearsal::provider::{
    LegacyChannel, LegacyProvider, PreflightSource, PublicName, build_preflight_report,
    canonical_json,
};

fn source(secret: &str) -> PreflightSource {
    PreflightSource {
        providers: vec![LegacyProvider {
            id: "provider-a".to_owned(),
            name: "Internal Provider".to_owned(),
            priority: 0,
            group_ids: vec!["group-a".to_owned(), "group-b".to_owned()],
            channels: vec![LegacyChannel {
                id: "channel-a".to_owned(),
                name: "Internal Channel".to_owned(),
                created_at: "2026-08-26T00:00:00Z".to_owned(),
                enabled: true,
                weight: 1,
                models: vec![],
            }],
        }],
        public_names: vec![PublicName {
            entity: "provider".to_owned(),
            source_id: "provider-a".to_owned(),
            normalized_name: "LynShen A".to_owned(),
        }],
        fingerprint_secrets: vec![secret.to_owned()],
    }
}

#[test]
fn report_contains_classification_and_no_secret_material() {
    let report = build_preflight_report(&source("sk-secret")).expect("valid preflight");
    let json = canonical_json(&report).expect("canonical report");
    let text = String::from_utf8(json).expect("UTF-8 report");

    assert!(text.ends_with('\n'));
    assert!(text.contains("source_fingerprint"));
    assert!(text.contains("semantic_change"));
    assert!(text.contains("LynShen A"));
    assert!(!text.contains("sk-secret"));
    assert!(!text.contains("Internal Provider"));
    assert!(!text.contains("Internal Channel"));
}

#[test]
fn secret_bytes_affect_fingerprint_without_being_serialized() {
    let left = build_preflight_report(&source("sk-one")).unwrap();
    let right = build_preflight_report(&source("sk-two")).unwrap();
    assert_ne!(left.source_fingerprint, right.source_fingerprint);
    assert!(
        !String::from_utf8(canonical_json(&left).unwrap())
            .unwrap()
            .contains("sk-one")
    );
}

#[test]
fn report_order_and_bytes_are_deterministic() {
    let input = source("sk-fixed");
    assert_eq!(
        canonical_json(&build_preflight_report(&input).unwrap()).unwrap(),
        canonical_json(&build_preflight_report(&input).unwrap()).unwrap()
    );
}
