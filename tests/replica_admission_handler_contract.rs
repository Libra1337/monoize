fn assert_every_dispatch_is_guarded(name: &str, source: &str, expected: usize) {
    let needle = "upstream::call_upstream";
    let mut cursor = 0usize;
    let mut count = 0usize;
    while let Some(relative) = source[cursor..].find(needle) {
        let dispatch = cursor + relative;
        let prefix_start = dispatch.saturating_sub(900);
        let prefix = &source[prefix_start..dispatch];
        assert!(
            prefix.contains("mark_plan_routed_before_dispatch"),
            "{name} dispatch #{count} lacks the durable Plan route guard"
        );
        count += 1;
        cursor = dispatch + needle.len();
    }
    assert_eq!(count, expected, "{name} physical dispatch count changed");
}

#[test]
fn every_handler_physical_dispatch_is_guarded() {
    assert_every_dispatch_is_guarded("embeddings", include_str!("../src/handlers/mod.rs"), 1);
    assert_every_dispatch_is_guarded("nonstream", include_str!("../src/handlers/nonstream.rs"), 3);
    assert_every_dispatch_is_guarded("streaming", include_str!("../src/handlers/streaming.rs"), 2);
    assert_every_dispatch_is_guarded("compact", include_str!("../src/handlers/compact.rs"), 1);
    assert_every_dispatch_is_guarded("image API", include_str!("../src/handlers/image_api.rs"), 1);
}

#[test]
fn handlers_do_not_return_the_removed_replica_plan_placeholder() {
    let sources = [
        include_str!("../src/handlers/mod.rs"),
        include_str!("../src/handlers/billing.rs"),
        include_str!("../src/handlers/nonstream.rs"),
        include_str!("../src/handlers/streaming.rs"),
        include_str!("../src/handlers/compact.rs"),
        include_str!("../src/handlers/image_api.rs"),
    ];
    assert!(
        sources
            .iter()
            .all(|source| !source.contains("plan_admission_token_required"))
    );
}
