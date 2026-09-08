use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityInput {
    pub peak_events_per_second: u64,
    pub max_outage_seconds: u64,
    pub safety_factor_milli: u64,
    pub max_in_flight_dispatches: u64,
    pub entry_max_bytes: u64,
    pub allocation_unit: u64,
    pub configured_quota_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedCapacity {
    pub peak_events_per_second: u64,
    pub max_in_flight_dispatches: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityResult {
    pub entry_reservation_bytes: u64,
    pub outage_event_slots: u64,
    pub minimum_spool_event_slots: u64,
    pub minimum_spool_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityError {
    InvalidPeak,
    InvalidOutage,
    InvalidSafetyFactor,
    InvalidInFlight,
    InvalidEntryMaximum,
    InvalidAllocationUnit,
    QuotaBelowMinimum,
    BelowApprovedNodeValue,
    Overflow,
}

pub fn calculate_capacity(input: CapacityInput) -> Result<CapacityResult, CapacityError> {
    validate_shape(input)?;
    let entry_reservation_bytes = round_up(input.entry_max_bytes, input.allocation_unit)?;
    let outage_numerator = input
        .peak_events_per_second
        .checked_mul(input.max_outage_seconds)
        .and_then(|value| value.checked_mul(input.safety_factor_milli))
        .ok_or(CapacityError::Overflow)?;
    let outage_event_slots = outage_numerator
        .checked_add(999)
        .ok_or(CapacityError::Overflow)?
        / 1_000;
    let minimum_spool_event_slots = outage_event_slots
        .checked_add(input.max_in_flight_dispatches)
        .ok_or(CapacityError::Overflow)?;
    let minimum_spool_bytes = minimum_spool_event_slots
        .checked_mul(entry_reservation_bytes)
        .ok_or(CapacityError::Overflow)?;
    if input.configured_quota_bytes < minimum_spool_bytes {
        return Err(CapacityError::QuotaBelowMinimum);
    }
    Ok(CapacityResult {
        entry_reservation_bytes,
        outage_event_slots,
        minimum_spool_event_slots,
        minimum_spool_bytes,
    })
}

pub fn validate_capacity(
    input: CapacityInput,
    approved: ApprovedCapacity,
) -> Result<CapacityResult, CapacityError> {
    if input.peak_events_per_second < approved.peak_events_per_second
        || input.max_in_flight_dispatches < approved.max_in_flight_dispatches
    {
        return Err(CapacityError::BelowApprovedNodeValue);
    }
    calculate_capacity(input)
}

fn validate_shape(input: CapacityInput) -> Result<(), CapacityError> {
    if input.peak_events_per_second == 0 {
        return Err(CapacityError::InvalidPeak);
    }
    if input.max_outage_seconds < 900 {
        return Err(CapacityError::InvalidOutage);
    }
    if input.safety_factor_milli < 1_200 {
        return Err(CapacityError::InvalidSafetyFactor);
    }
    if input.max_in_flight_dispatches == 0 {
        return Err(CapacityError::InvalidInFlight);
    }
    if input.entry_max_bytes < 1_024 {
        return Err(CapacityError::InvalidEntryMaximum);
    }
    if input.allocation_unit == 0 {
        return Err(CapacityError::InvalidAllocationUnit);
    }
    Ok(())
}

fn round_up(value: u64, unit: u64) -> Result<u64, CapacityError> {
    let units = value.checked_add(unit - 1).ok_or(CapacityError::Overflow)? / unit;
    units.checked_mul(unit).ok_or(CapacityError::Overflow)
}
