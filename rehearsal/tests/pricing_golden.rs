use monoize_lynshen_rehearsal::pricing::{
    LineItem, PricingMapping, PricingScenario, charge, compare_snapshots, snapshot,
};
use monoize_lynshen_rehearsal::provider::CanonicalDecimal;

fn decimal(value: &str) -> CanonicalDecimal {
    CanonicalDecimal::parse(value).expect("fixture multiplier")
}

#[test]
fn aggregate_is_scaled_and_truncated_once() {
    let items = vec![LineItem::new("input", 1, 1), LineItem::new("output", 1, 1)];
    assert_eq!(charge(&items, &decimal("1.5")).unwrap(), 3);

    let fractional = vec![LineItem::new("input", 1, 1), LineItem::new("output", 1, 1)];
    assert_eq!(charge(&fractional, &decimal("1.4")).unwrap(), 2);
}

#[test]
fn charge_rejects_quantity_sum_and_scaling_overflow() {
    assert!(
        charge(
            &[
                LineItem::new("a", u64::MAX, u64::MAX),
                LineItem::new("b", u64::MAX, u64::MAX),
            ],
            &decimal("1")
        )
        .is_err()
    );
    assert!(
        charge(
            &[LineItem::new("input", u64::MAX, u64::MAX)],
            &decimal("9999999999999999999999999999")
        )
        .is_err()
    );
}

#[test]
fn pre_and_post_snapshots_are_byte_equal_for_equivalent_mappings() {
    let scenarios = vec![
        PricingScenario::new("one", vec![LineItem::new("input", 1, 1_001)]),
        PricingScenario::new(
            "mixed",
            vec![
                LineItem::new("input", 999, 1_001),
                LineItem::new("cache_read", 1_000_000, 7),
                LineItem::new("meter:image", 2, 125_000),
            ],
        ),
    ];
    let legacy = vec![PricingMapping::new(
        "provider-a",
        "group-a",
        "gpt-4o",
        "gpt-4o-upstream",
        "profile-s",
        decimal("1.2"),
    )];
    let target = legacy.clone();

    let before = snapshot(&legacy, &scenarios).unwrap();
    let after = snapshot(&target, &scenarios).unwrap();
    assert_eq!(before, after);
    assert!(compare_snapshots(&before, &after).unwrap().equal);
}

#[test]
fn comparison_detects_one_effective_multiplier_change() {
    let scenarios = vec![PricingScenario::new(
        "one",
        vec![LineItem::new("input", 1, 1_000)],
    )];
    let before = snapshot(
        &[PricingMapping::new("p", "g", "m", "u", "s", decimal("1.2"))],
        &scenarios,
    )
    .unwrap();
    let after = snapshot(
        &[PricingMapping::new("p", "g", "m", "u", "s", decimal("1.5"))],
        &scenarios,
    )
    .unwrap();

    let comparison = compare_snapshots(&before, &after).unwrap();
    assert!(!comparison.equal);
    assert_eq!(comparison.before_sha256.len(), 64);
    assert_eq!(comparison.after_sha256.len(), 64);
}
