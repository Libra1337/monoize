use monoize_lynshen_rehearsal::status::{DispatchGate, DispatchPath};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_gate_bounds_all_physical_dispatch_paths() {
    let gate = Arc::new(DispatchGate::new(8).unwrap());
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let paths = [
        DispatchPath::HttpInitial,
        DispatchPath::HttpRetry,
        DispatchPath::ProviderFailForward,
        DispatchPath::WebSocket,
    ];
    let mut tasks = Vec::new();
    for index in 0..64 {
        let gate = gate.clone();
        let active = active.clone();
        let maximum = maximum.clone();
        let path = paths[index % paths.len()];
        tasks.push(tokio::spawn(async move {
            gate.dispatch(path, || async {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(2)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            })
            .await
            .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    assert_eq!(maximum.load(Ordering::SeqCst), 8);
    assert_eq!(gate.maximum_observed(), 8);
    assert_eq!(gate.current(), 0);
}

#[tokio::test]
async fn cancellation_while_waiting_does_not_consume_a_permit() {
    let gate = Arc::new(DispatchGate::new(1).unwrap());
    let held = gate.acquire(DispatchPath::HttpInitial).await.unwrap();
    let waiting = {
        let gate = gate.clone();
        tokio::spawn(async move { gate.acquire(DispatchPath::HttpRetry).await })
    };
    tokio::task::yield_now().await;
    waiting.abort();
    drop(held);

    let next = gate.acquire(DispatchPath::WebSocket).await.unwrap();
    assert_eq!(gate.current(), 1);
    drop(next);
    assert_eq!(gate.current(), 0);
}

#[test]
fn zero_permits_are_invalid() {
    assert!(DispatchGate::new(0).is_err());
}
