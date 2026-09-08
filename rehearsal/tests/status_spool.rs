use monoize_lynshen_rehearsal::status::{
    EventOutcome, FailureClass, NodeState, Spool, SpoolConfig, SpoolError, UpstreamCallEvent,
};
use tempfile::TempDir;
use uuid::Uuid;

fn event(index: u64) -> UpstreamCallEvent {
    UpstreamCallEvent::new(
        "primary",
        Uuid::parse_str("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        index,
        "group-a",
        "provider-a",
        "channel-a",
        EventOutcome::Failure {
            class: FailureClass::Transient,
            upstream_status: Some(503),
        },
        1_777_000_000_000,
        7,
    )
    .unwrap()
}

fn spool(directory: &TempDir, entry_max_bytes: usize) -> Spool {
    Spool::open(
        directory.path(),
        SpoolConfig {
            entry_max_bytes,
            state_max_bytes: 1_024,
        },
    )
    .unwrap()
}

#[test]
fn event_id_is_stable_and_contains_no_request_data() {
    let value = event(3);
    assert_eq!(value.id, "primary.123e4567-e89b-42d3-a456-426614174000.3");
    let encoded = serde_json::to_string(&value).unwrap();
    for forbidden in ["user_id", "api_key", "request_body", "error_text", "url"] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn publish_survives_reopen_and_replay_uses_same_id() {
    let directory = TempDir::new().unwrap();
    let event = event(0);
    spool(&directory, 4_096).publish(&event).unwrap();

    let reopened = spool(&directory, 4_096);
    let pending = reopened.pending_events().unwrap();
    assert_eq!(pending, vec![event]);
}

#[test]
fn temporary_event_is_loss_evidence_and_is_cleaned() {
    let directory = TempDir::new().unwrap();
    std::fs::write(
        directory.path().join("primary.test.0.event.tmp"),
        b"partial",
    )
    .unwrap();
    let reopened = spool(&directory, 4_096);

    assert!(reopened.state().incomplete_since_unix_ms.is_some());
    assert!(!directory.path().join("primary.test.0.event.tmp").exists());
}

#[test]
fn oversized_event_sets_latch_without_truncation() {
    let directory = TempDir::new().unwrap();
    let spool = spool(&directory, 128);
    assert_eq!(
        spool.publish(&event(0)).unwrap_err(),
        SpoolError::EntryTooLarge
    );
    assert!(spool.pending_events().unwrap().is_empty());
    assert!(spool.state().incomplete_since_unix_ms.is_some());
}

#[test]
fn state_publication_round_trips_clean_shutdown_and_loss_times() {
    let directory = TempDir::new().unwrap();
    let opened = spool(&directory, 4_096);
    opened
        .write_state(NodeState {
            clean_shutdown: false,
            incomplete_since_unix_ms: Some(100),
            incomplete_until_unix_ms: Some(86_400_100),
            most_recent_loss_unix_ms: Some(100),
        })
        .unwrap();
    assert!(!spool(&directory, 4_096).state().clean_shutdown);
    assert_eq!(
        spool(&directory, 4_096).state().incomplete_since_unix_ms,
        Some(100)
    );
}
