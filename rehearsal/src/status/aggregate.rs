use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Operational,
    MinorDegradation,
    MajorDegradation,
    Unavailable,
    InsufficientData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderWindow {
    pub state: HealthState,
}

impl ProviderWindow {
    pub fn new(state: HealthState) -> Self {
        Self { state }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupAggregate {
    pub state: HealthState,
    pub insufficient_provider_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceCompleteness {
    pub active: bool,
    pub heartbeat_current: bool,
    pub clock_synchronized: bool,
    pub oldest_pending_event_unix_ms: Option<u64>,
    pub incomplete_until_unix_ms: Option<u64>,
}

impl SourceCompleteness {
    pub fn complete() -> Self {
        Self {
            active: true,
            heartbeat_current: true,
            clock_synchronized: true,
            oldest_pending_event_unix_ms: None,
            incomplete_until_unix_ms: None,
        }
    }
}

pub fn provider_state(successes: u64, failures: u64) -> HealthState {
    let Some(attempts) = successes.checked_add(failures) else {
        return HealthState::Unavailable;
    };
    if attempts < 10 {
        return HealthState::InsufficientData;
    }
    let successes = u128::from(successes);
    let attempts = u128::from(attempts);
    if successes * 100 >= attempts * 95 {
        HealthState::Operational
    } else if successes * 100 >= attempts * 80 {
        HealthState::MinorDegradation
    } else if successes * 100 >= attempts * 50 {
        HealthState::MajorDegradation
    } else {
        HealthState::Unavailable
    }
}

pub fn success_rate_basis_points(successes: u64, failures: u64) -> Option<u16> {
    let attempts = successes.checked_add(failures)?;
    if attempts == 0 {
        return None;
    }
    let basis_points = u128::from(successes) * 10_000 / u128::from(attempts);
    u16::try_from(basis_points).ok()
}

pub fn aggregate_group(providers: &[ProviderWindow]) -> GroupAggregate {
    let insufficient_provider_count = providers
        .iter()
        .filter(|provider| provider.state == HealthState::InsufficientData)
        .count() as u64;
    let state = providers
        .iter()
        .filter_map(|provider| severity(provider.state).map(|severity| (severity, provider.state)))
        .max_by_key(|(severity, _)| *severity)
        .map_or(HealthState::InsufficientData, |(_, state)| state);
    GroupAggregate {
        state,
        insufficient_provider_count,
    }
}

pub fn data_complete(sources: &[SourceCompleteness], data_through_unix_ms: u64) -> bool {
    sources.iter().filter(|source| source.active).all(|source| {
        source.heartbeat_current
            && source.clock_synchronized
            && source
                .oldest_pending_event_unix_ms
                .is_none_or(|oldest| oldest > data_through_unix_ms)
            && source
                .incomplete_until_unix_ms
                .is_none_or(|until| until <= data_through_unix_ms)
    })
}

fn severity(state: HealthState) -> Option<u8> {
    match state {
        HealthState::Operational => Some(0),
        HealthState::MinorDegradation => Some(1),
        HealthState::MajorDegradation => Some(2),
        HealthState::Unavailable => Some(3),
        HealthState::InsufficientData => None,
    }
}
