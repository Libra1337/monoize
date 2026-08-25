use monoize_lynshen_rehearsal::provider::{
    CanonicalDecimal, Classification, LegacyChannel, LegacyModel, LegacyProvider, PricingMode,
    deterministic_id, transform_provider,
};

fn decimal(value: &str) -> CanonicalDecimal {
    CanonicalDecimal::parse(value).expect("fixture decimal")
}

fn model(name: &str, profile: Option<&str>, multiplier: &str) -> LegacyModel {
    LegacyModel {
        name: name.to_owned(),
        redirect: None,
        resolved_profile: profile.map(str::to_owned),
        multiplier: decimal(multiplier),
    }
}

fn channel(id: &str, created_at: &str, models: Vec<LegacyModel>) -> LegacyChannel {
    LegacyChannel {
        id: id.to_owned(),
        name: format!("Channel {id}"),
        created_at: created_at.to_owned(),
        enabled: true,
        weight: 1,
        models,
    }
}

fn provider(groups: &[&str], channels: Vec<LegacyChannel>) -> LegacyProvider {
    LegacyProvider {
        id: "provider-old".to_owned(),
        name: "Provider A".to_owned(),
        priority: 7,
        group_ids: groups.iter().map(|value| (*value).to_owned()).collect(),
        channels,
    }
}

#[test]
fn expands_in_stored_group_and_sorted_channel_order() {
    let source = provider(
        &["group-b", "group-a"],
        vec![
            channel("channel-2", "2026-01-02T00:00:00Z", vec![]),
            channel("channel-1", "2026-01-01T00:00:00Z", vec![]),
        ],
    );
    let result = transform_provider(&source).expect("valid source");
    let identities = result
        .targets
        .iter()
        .map(|target| (target.group_id.clone(), target.channel.id.clone()))
        .collect::<Vec<_>>();

    assert_eq!(result.classification, Classification::SemanticChange);
    assert_eq!(
        identities,
        vec![
            ("group-b".to_owned(), "channel-1".to_owned()),
            ("group-b".to_owned(), "channel-2".to_owned()),
            (
                "group-a".to_owned(),
                deterministic_id("channel", "provider-old", "group-a", "channel-1")
            ),
            (
                "group-a".to_owned(),
                deterministic_id("channel", "provider-old", "group-a", "channel-2")
            ),
        ]
    );
    assert_eq!(result.targets[0].id, source.id);
}

#[test]
fn blocks_zero_group_and_zero_channel_sources() {
    assert_eq!(
        transform_provider(&provider(&[], vec![channel("c", "1", vec![])]))
            .expect_err("zero groups must fail")
            .code(),
        "provider_has_no_group"
    );
    assert_eq!(
        transform_provider(&provider(&["g"], vec![]))
            .expect_err("zero channels must fail")
            .code(),
        "provider_has_no_channel"
    );
}

#[test]
fn route_safe_requires_one_group_and_at_most_one_enabled_positive_channel() {
    let safe = provider(
        &["g"],
        vec![
            channel("disabled", "1", vec![]).with_route_state(false, 1),
            channel("live", "2", vec![]),
        ],
    );
    assert_eq!(
        transform_provider(&safe).unwrap().classification,
        Classification::RouteSafe
    );

    let changed = provider(
        &["g"],
        vec![channel("a", "1", vec![]), channel("b", "2", vec![])],
    );
    assert_eq!(
        transform_provider(&changed).unwrap().classification,
        Classification::SemanticChange
    );
}

#[test]
fn infers_profile_and_numeric_multiplier_ties_without_changing_effective_values() {
    let source = provider(
        &["g"],
        vec![channel(
            "c",
            "1",
            vec![
                model("m1", Some("Zulu"), "2"),
                model("m2", Some("Alpha"), "1.5"),
                model("m3", None, "2"),
                model("m4", Some("Alpha"), "1.5"),
            ],
        )],
    );
    let target = &transform_provider(&source).unwrap().targets[0];

    assert_eq!(target.pricing_profile.as_deref(), Some("Alpha"));
    assert_eq!(target.multiplier.as_str(), "1.5");
    assert_eq!(
        target.models[0].pricing,
        PricingMode::Override("Zulu".to_owned())
    );
    assert_eq!(
        target.models[0]
            .multiplier_override
            .as_ref()
            .unwrap()
            .as_str(),
        "2"
    );
    assert_eq!(target.models[1].pricing, PricingMode::Inherit);
    assert_eq!(target.models[1].multiplier_override, None);
    assert_eq!(target.models[2].pricing, PricingMode::Unpriced);
}

#[test]
fn deterministic_ids_are_stable_and_type_separated() {
    let provider_id = deterministic_id("provider", "p", "g", "c");
    assert_eq!(provider_id, deterministic_id("provider", "p", "g", "c"));
    assert!(provider_id.starts_with("p_"));
    assert_eq!(provider_id.len(), 34);
    assert_ne!(provider_id, deterministic_id("channel", "p", "g", "c"));
}

trait LegacyChannelFixture {
    fn with_route_state(self, enabled: bool, weight: i32) -> Self;
}

impl LegacyChannelFixture for LegacyChannel {
    fn with_route_state(mut self, enabled: bool, weight: i32) -> Self {
        self.enabled = enabled;
        self.weight = weight;
        self
    }
}
