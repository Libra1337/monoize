use monoize_lynshen_rehearsal::status::{
    EventOutcome, Spool, SpoolConfig, StatusSink, UpstreamCallEvent,
};
use sqlx::{Connection, SqliteConnection};
use tempfile::TempDir;
use uuid::Uuid;

fn event(index: u64) -> UpstreamCallEvent {
    UpstreamCallEvent::new(
        "primary",
        Uuid::parse_str("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        index,
        "g",
        "p",
        "c",
        EventOutcome::Success,
        1_777_000_000_000 + index,
        1,
    )
    .unwrap()
}

#[tokio::test]
async fn replay_is_idempotent_by_event_id() {
    let mut db = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    let sink = StatusSink::create_sqlite(&mut db).await.unwrap();
    let events = vec![event(0), event(1), event(0)];
    assert_eq!(sink.insert_sqlite(&mut db, &events).await.unwrap(), 2);
    assert_eq!(sink.insert_sqlite(&mut db, &events).await.unwrap(), 0);
    assert_eq!(sink.count_sqlite(&mut db).await.unwrap(), 2);
}

#[tokio::test]
async fn committed_drain_deletes_files_and_failed_drain_retains_them() {
    let directory = TempDir::new().unwrap();
    let spool = Spool::open(
        directory.path(),
        SpoolConfig {
            entry_max_bytes: 4_096,
            state_max_bytes: 1_024,
        },
    )
    .unwrap();
    spool.publish(&event(0)).unwrap();
    spool.publish(&event(1)).unwrap();

    let mut db = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    let sink = StatusSink::create_sqlite(&mut db).await.unwrap();
    sink.drain_sqlite(&mut db, &spool, 1).await.unwrap();
    assert_eq!(spool.pending_events().unwrap().len(), 1);

    sqlx::query("DROP TABLE upstream_call_events")
        .execute(&mut db)
        .await
        .unwrap();
    assert!(sink.drain_sqlite(&mut db, &spool, 100).await.is_err());
    assert_eq!(spool.pending_events().unwrap().len(), 1);
}
