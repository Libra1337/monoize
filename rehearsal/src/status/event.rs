use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    RateLimited,
    Transient,
    Persistent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventOutcome {
    Success,
    Failure {
        class: FailureClass,
        upstream_status: Option<u16>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpstreamCallEvent {
    pub id: String,
    pub group_id: String,
    pub provider_id: String,
    pub channel_id: String,
    pub outcome: String,
    pub failure_class: Option<FailureClass>,
    pub upstream_status: Option<u16>,
    pub occurred_at_unix_ms: u64,
    pub source_node_id: String,
    pub provider_generation: u64,
}

impl UpstreamCallEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_node_id: &str,
        lifecycle_id: Uuid,
        dispatch_index: u64,
        group_id: &str,
        provider_id: &str,
        channel_id: &str,
        outcome: EventOutcome,
        occurred_at_unix_ms: u64,
        provider_generation: u64,
    ) -> Result<Self, EventError> {
        if source_node_id.is_empty()
            || !source_node_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(EventError::InvalidSourceNode);
        }
        let (outcome, failure_class, upstream_status) = match outcome {
            EventOutcome::Success => ("success".to_owned(), None, None),
            EventOutcome::Failure {
                class,
                upstream_status,
            } => ("failure".to_owned(), Some(class), upstream_status),
        };
        Ok(Self {
            id: format!(
                "{source_node_id}.{}.{}",
                lifecycle_id.hyphenated(),
                dispatch_index
            ),
            group_id: group_id.to_owned(),
            provider_id: provider_id.to_owned(),
            channel_id: channel_id.to_owned(),
            outcome,
            failure_class,
            upstream_status,
            occurred_at_unix_ms,
            source_node_id: source_node_id.to_owned(),
            provider_generation,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventError {
    InvalidSourceNode,
}
