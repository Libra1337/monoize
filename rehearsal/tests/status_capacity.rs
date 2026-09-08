use monoize_lynshen_rehearsal::status::{
    ApprovedCapacity, CapacityError, CapacityInput, calculate_capacity, validate_capacity,
};

fn input() -> CapacityInput {
    CapacityInput {
        peak_events_per_second: 100,
        max_outage_seconds: 900,
        safety_factor_milli: 1_200,
        max_in_flight_dispatches: 1_024,
        entry_max_bytes: 4_096,
        allocation_unit: 4_096,
        configured_quota_bytes: 446_562_304,
    }
}

#[test]
fn computes_checked_minimum_at_exact_boundary() {
    let result = calculate_capacity(input()).expect("valid capacity");
    assert_eq!(result.entry_reservation_bytes, 4_096);
    assert_eq!(result.outage_event_slots, 108_000);
    assert_eq!(result.minimum_spool_event_slots, 109_024);
    assert_eq!(result.minimum_spool_bytes, 446_562_304);
}

#[test]
fn rejects_one_byte_below_minimum() {
    let mut value = input();
    value.configured_quota_bytes -= 1;
    assert_eq!(
        calculate_capacity(value).unwrap_err(),
        CapacityError::QuotaBelowMinimum
    );
}

#[test]
fn rounds_entry_reservation_to_allocation_unit() {
    let mut value = input();
    value.entry_max_bytes = 4_097;
    value.configured_quota_bytes = u64::MAX;
    assert_eq!(
        calculate_capacity(value).unwrap().entry_reservation_bytes,
        8_192
    );
}

#[test]
fn rejects_invalid_and_overflowing_inputs() {
    for mutate in [
        |value: &mut CapacityInput| value.peak_events_per_second = 0,
        |value: &mut CapacityInput| value.max_outage_seconds = 899,
        |value: &mut CapacityInput| value.safety_factor_milli = 1_199,
        |value: &mut CapacityInput| value.max_in_flight_dispatches = 0,
        |value: &mut CapacityInput| value.entry_max_bytes = 1_023,
        |value: &mut CapacityInput| value.allocation_unit = 0,
    ] {
        let mut value = input();
        mutate(&mut value);
        assert!(calculate_capacity(value).is_err());
    }

    let mut overflow = input();
    overflow.peak_events_per_second = u64::MAX;
    overflow.configured_quota_bytes = u64::MAX;
    assert_eq!(
        calculate_capacity(overflow).unwrap_err(),
        CapacityError::Overflow
    );
}

#[test]
fn deployment_values_must_cover_approved_node_evidence() {
    let approved = ApprovedCapacity {
        peak_events_per_second: 101,
        max_in_flight_dispatches: 1_025,
    };
    assert_eq!(
        validate_capacity(input(), approved).unwrap_err(),
        CapacityError::BelowApprovedNodeValue
    );
}
