use monoize_lynshen_rehearsal::status::{
    HealthState, ProviderWindow, SourceCompleteness, aggregate_group, data_complete,
    provider_state, success_rate_basis_points,
};

#[test]
fn provider_thresholds_use_integer_cross_products() {
    assert_eq!(provider_state(9, 0), HealthState::InsufficientData);
    assert_eq!(provider_state(95, 5), HealthState::Operational);
    assert_eq!(provider_state(94, 6), HealthState::MinorDegradation);
    assert_eq!(provider_state(80, 20), HealthState::MinorDegradation);
    assert_eq!(provider_state(79, 21), HealthState::MajorDegradation);
    assert_eq!(provider_state(50, 50), HealthState::MajorDegradation);
    assert_eq!(provider_state(49, 51), HealthState::Unavailable);
}

#[test]
fn success_rate_is_floor_basis_points_and_zero_is_null() {
    assert_eq!(success_rate_basis_points(0, 0), None);
    assert_eq!(success_rate_basis_points(2, 1), Some(6_666));
    assert_eq!(success_rate_basis_points(1, 0), Some(10_000));
}

#[test]
fn group_uses_worst_known_state_and_counts_insufficient_providers() {
    let aggregate = aggregate_group(&[
        ProviderWindow::new(HealthState::Operational),
        ProviderWindow::new(HealthState::InsufficientData),
        ProviderWindow::new(HealthState::MajorDegradation),
    ]);
    assert_eq!(aggregate.state, HealthState::MajorDegradation);
    assert_eq!(aggregate.insufficient_provider_count, 1);

    let all_insufficient = aggregate_group(&[
        ProviderWindow::new(HealthState::InsufficientData),
        ProviderWindow::new(HealthState::InsufficientData),
    ]);
    assert_eq!(all_insufficient.state, HealthState::InsufficientData);
    assert_eq!(all_insufficient.insufficient_provider_count, 2);
}

#[test]
fn pending_old_events_clock_skew_and_latch_make_data_incomplete() {
    let through = 1_000_000;
    assert!(data_complete(&[SourceCompleteness::complete()], through));
    assert!(!data_complete(
        &[SourceCompleteness {
            oldest_pending_event_unix_ms: Some(through),
            ..SourceCompleteness::complete()
        }],
        through
    ));
    assert!(!data_complete(
        &[SourceCompleteness {
            clock_synchronized: false,
            ..SourceCompleteness::complete()
        }],
        through
    ));
    assert!(!data_complete(
        &[SourceCompleteness {
            incomplete_until_unix_ms: Some(through + 1),
            ..SourceCompleteness::complete()
        }],
        through
    ));
}
