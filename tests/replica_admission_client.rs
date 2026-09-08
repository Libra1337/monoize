use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use ed25519_dalek::SigningKey;
use futures_util::{StreamExt as _, stream};
use monoize::replica::admission_client::{
    AdmissionClient, ReplicaFundingDecision, ReplicaIssueInput,
};
use monoize::replica::metering::MeteringSpoolCapacity;
use monoize::store_billing::admission_token::{
    ADMISSION_ISSUER, AdmissionClaimStore, AdmissionKeyRing, AdmissionSigningKey,
    AdmissionTokenInput, AdmissionVerifierKey, AdmissionVerifierRing, TerminalKind,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;

const TOKEN: &str = "replica-cluster-token-with-at-least-32-bytes";
const REPLICA_ID: &str = "6f8b7f54-1833-4dc8-9e93-24482e870c22";
const OVERSIZED_KEYSET: usize = 1;
const OVERSIZED_ISSUE: usize = 2;
const OVERSIZED_CONFIRM: usize = 3;

#[derive(Clone, Copy)]
enum ConfirmMode {
    Success,
    FirstResponseLost,
    BlockFirst,
    FirstResponseLostBlockSecond,
    Reject,
}

#[derive(Clone)]
struct MockPrimary {
    keyset_calls: Arc<AtomicUsize>,
    issue_calls: Arc<AtomicUsize>,
    confirm_calls: Arc<AtomicUsize>,
    keyset_delay: Duration,
    first_key: AdmissionVerifierKey,
    issue_key: AdmissionVerifierKey,
    issue_ring: AdmissionKeyRing,
    confirm_mode: ConfirmMode,
    confirm_gate: Arc<tokio::sync::Notify>,
    return_balance: Arc<AtomicBool>,
    retain_first_keyset: Arc<AtomicBool>,
    keyset_failures_remaining: Arc<AtomicUsize>,
    oversized_response: Arc<AtomicUsize>,
}

#[derive(Deserialize)]
struct IssueWire {
    audience: String,
    user_id: String,
    request_id: String,
    effective_groups: Vec<String>,
    maximum_nano_usd: String,
    pricing_revision: String,
}

async fn keyset(State(state): State<MockPrimary>, headers: HeaderMap) -> Response {
    assert_cluster_headers(&headers);
    if !state.keyset_delay.is_zero() {
        tokio::time::sleep(state.keyset_delay).await;
    }
    let call = state.keyset_calls.fetch_add(1, Ordering::SeqCst);
    if state.oversized_response.load(Ordering::SeqCst) == OVERSIZED_KEYSET {
        return oversized_pending_response();
    }
    if state
        .keyset_failures_remaining
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let key = if call == 0 || state.retain_first_keyset.load(Ordering::SeqCst) {
        &state.first_key
    } else {
        &state.issue_key
    };
    Json(json!({
        "keys": [{
            "key_id": key.key_id,
            "public_key_base64": key.public_key_base64,
            "state": key.state,
            "activated_at": key.activated_at,
            "verify_until": key.verify_until,
        }]
    }))
    .into_response()
}

async fn issue(
    State(state): State<MockPrimary>,
    headers: HeaderMap,
    Json(body): Json<IssueWire>,
) -> Response {
    assert_cluster_headers(&headers);
    state.issue_calls.fetch_add(1, Ordering::SeqCst);
    if state.oversized_response.load(Ordering::SeqCst) == OVERSIZED_ISSUE {
        return oversized_pending_response();
    }
    assert_eq!(body.audience, REPLICA_ID);
    assert_eq!(body.user_id, "plan-user");
    assert_eq!(body.effective_groups, vec!["default"]);
    if state.return_balance.load(Ordering::SeqCst) {
        return Json(json!({ "funding": "balance" })).into_response();
    }
    let issued_at = chrono::DateTime::from_timestamp(Utc::now().timestamp(), 0).unwrap();
    let token_id = "admission-token";
    let reservation_id = "admission-reservation";
    let maximum_nano_usd = body.maximum_nano_usd.parse::<i128>().unwrap();
    let compact_jws = state
        .issue_ring
        .issue(AdmissionTokenInput {
            audience: body.audience,
            token_id: token_id.to_string(),
            reservation_id: reservation_id.to_string(),
            request_id: body.request_id,
            entitlement_id: "plan-entitlement".to_string(),
            generation: 1,
            maximum_nano_usd,
            reserved_fen_cny: 1,
            pricing_revision: body.pricing_revision,
            issued_at,
        })
        .unwrap();
    Json(json!({
        "funding": "plan",
        "token_id": token_id,
        "reservation_id": reservation_id,
        "compact_jws": compact_jws,
        "issued_at": issued_at,
        "expires_at": issued_at + chrono::Duration::seconds(30),
        "duplicate": false,
    }))
    .into_response()
}

async fn confirm(
    State(state): State<MockPrimary>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_cluster_headers(&headers);
    assert_eq!(body["audience"], REPLICA_ID);
    assert_eq!(body["token_id"], "admission-token");
    assert_eq!(body["reservation_id"], "admission-reservation");
    assert_eq!(body["request_id"], "external-request");
    let call = state.confirm_calls.fetch_add(1, Ordering::SeqCst);
    if state.oversized_response.load(Ordering::SeqCst) == OVERSIZED_CONFIRM {
        return oversized_pending_response();
    }
    match state.confirm_mode {
        ConfirmMode::Success => {
            Json(json!({ "confirmed": true, "duplicate": call > 0 })).into_response()
        }
        ConfirmMode::FirstResponseLost if call == 0 => {
            tokio::time::sleep(Duration::from_millis(250)).await;
            Json(json!({ "confirmed": true, "duplicate": false })).into_response()
        }
        ConfirmMode::FirstResponseLost => {
            Json(json!({ "confirmed": true, "duplicate": true })).into_response()
        }
        ConfirmMode::BlockFirst if call == 0 => {
            state.confirm_gate.notified().await;
            Json(json!({ "confirmed": true, "duplicate": false })).into_response()
        }
        ConfirmMode::BlockFirst => {
            Json(json!({ "confirmed": true, "duplicate": true })).into_response()
        }
        ConfirmMode::FirstResponseLostBlockSecond if call == 0 => {
            tokio::time::sleep(Duration::from_millis(250)).await;
            Json(json!({ "confirmed": true, "duplicate": false })).into_response()
        }
        ConfirmMode::FirstResponseLostBlockSecond => {
            state.confirm_gate.notified().await;
            Json(json!({ "confirmed": true, "duplicate": true })).into_response()
        }
        ConfirmMode::Reject => StatusCode::CONFLICT.into_response(),
    }
}

fn oversized_pending_response() -> Response {
    let first = stream::once(async { Ok::<_, Infallible>(bytes::Bytes::from(vec![b'x'; 65_537])) });
    let pending = stream::pending::<Result<bytes::Bytes, Infallible>>();
    Body::from_stream(first.chain(pending)).into_response()
}

fn assert_cluster_headers(headers: &HeaderMap) {
    assert_eq!(
        headers.get("authorization").unwrap(),
        &format!("Bearer {TOKEN}")
    );
    assert_eq!(headers.get("x-monoize-replica-id").unwrap(), REPLICA_ID);
}

fn verifier_key(id: &str, seed: [u8; 32]) -> AdmissionVerifierKey {
    AdmissionVerifierKey {
        key_id: id.to_string(),
        public_key_base64: URL_SAFE_NO_PAD
            .encode(SigningKey::from_bytes(&seed).verifying_key().as_bytes()),
        state: "active".to_string(),
        activated_at: Utc::now() - chrono::Duration::minutes(1),
        verify_until: None,
    }
}

async fn fixture(
    first_key: AdmissionVerifierKey,
    issue_key: AdmissionVerifierKey,
    issue_seed: [u8; 32],
    keyset_delay: Duration,
    confirm_mode: ConfirmMode,
    request_timeout: Duration,
) -> (TempDir, AdmissionClient, MockPrimary) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let issue_ring = AdmissionKeyRing::new(
        ADMISSION_ISSUER,
        AdmissionSigningKey::from_seed(
            issue_key.key_id.clone(),
            issue_seed,
            Utc::now() - chrono::Duration::minutes(1),
        )
        .unwrap(),
        vec![],
    )
    .unwrap();
    let state = MockPrimary {
        keyset_calls: Arc::new(AtomicUsize::new(0)),
        issue_calls: Arc::new(AtomicUsize::new(0)),
        confirm_calls: Arc::new(AtomicUsize::new(0)),
        keyset_delay,
        first_key,
        issue_key,
        issue_ring,
        confirm_mode,
        confirm_gate: Arc::new(tokio::sync::Notify::new()),
        return_balance: Arc::new(AtomicBool::new(false)),
        retain_first_keyset: Arc::new(AtomicBool::new(false)),
        keyset_failures_remaining: Arc::new(AtomicUsize::new(0)),
        oversized_response: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new()
        .route("/internal/replica/admission/keyset", get(keyset))
        .route("/internal/replica/admission/issue", post(issue))
        .route("/internal/replica/admission/confirm", post(confirm))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let temp = TempDir::new().unwrap();
    let capacity = Arc::new(MeteringSpoolCapacity::new(1024 * 1024));
    let claims = Arc::new(
        AdmissionClaimStore::new_with_capacity(temp.path(), capacity)
            .await
            .unwrap(),
    );
    let client = AdmissionClient::new(
        reqwest::Client::builder()
            .no_proxy()
            .timeout(request_timeout)
            .build()
            .unwrap(),
        format!("http://{address}"),
        TOKEN.to_string(),
        REPLICA_ID.to_string(),
        Duration::from_secs(60),
        AdmissionVerifierRing::new(),
        claims,
        Arc::new(tokio::sync::Notify::new()),
    )
    .unwrap();
    (temp, client, state)
}

fn issue_input() -> ReplicaIssueInput {
    ReplicaIssueInput {
        user_id: "plan-user".to_string(),
        request_id: "external-request".to_string(),
        effective_groups: vec!["default".to_string()],
        maximum_nano_usd: 1_000_000,
        pricing_revision: "pricing-v1".to_string(),
    }
}

fn only_claim_json(temp: &TempDir) -> Value {
    let paths = std::fs::read_dir(temp.path().join("claims"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    serde_json::from_slice(&std::fs::read(&paths[0]).unwrap()).unwrap()
}

async fn assert_cancelled_issue_released(
    temp: &TempDir,
    client: &AdmissionClient,
    primary: &MockPrimary,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let terminals = client.claims().load_pending_terminals(10).await.unwrap();
            if terminals.len() == 1 && !client.has_active("external-request") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let claim = only_claim_json(temp);
    assert_eq!(claim["state"], "release_pending");
    let terminals = client.claims().load_pending_terminals(10).await.unwrap();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].input.kind, TerminalKind::Release);
    assert!(primary.confirm_calls.load(Ordering::SeqCst) >= 1);

    let visible_bytes = std::fs::read_dir(temp.path().join("claims"))
        .unwrap()
        .chain(std::fs::read_dir(temp.path().join("terminal")).unwrap())
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum::<u64>();
    assert_eq!(client.claims().capacity().accounted_bytes(), visible_bytes);
}

#[tokio::test]
async fn cancelled_issue_during_first_confirmation_releases_in_process() {
    let key = verifier_key("key-a", [12; 32]);
    let (temp, client, primary) = fixture(
        key.clone(),
        key,
        [12; 32],
        Duration::ZERO,
        ConfirmMode::BlockFirst,
        Duration::from_secs(2),
    )
    .await;
    let owner = tokio::spawn({
        let client = client.clone();
        async move { client.issue(issue_input()).await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while primary.confirm_calls.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    owner.abort();
    assert!(owner.await.unwrap_err().is_cancelled());
    primary.confirm_gate.notify_one();

    assert_cancelled_issue_released(&temp, &client, &primary).await;
}

#[tokio::test]
async fn cancelled_issue_after_confirmation_response_loss_releases_in_process() {
    let key = verifier_key("key-a", [13; 32]);
    let (temp, client, primary) = fixture(
        key.clone(),
        key,
        [13; 32],
        Duration::ZERO,
        ConfirmMode::FirstResponseLostBlockSecond,
        Duration::from_millis(75),
    )
    .await;
    let owner = tokio::spawn({
        let client = client.clone();
        async move { client.issue(issue_input()).await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while primary.confirm_calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    owner.abort();
    assert!(owner.await.unwrap_err().is_cancelled());
    primary.confirm_gate.notify_one();

    assert_cancelled_issue_released(&temp, &client, &primary).await;
}

#[tokio::test]
async fn last_unfinished_handler_scope_drop_publishes_one_release() {
    let key = verifier_key("key-a", [14; 32]);
    let (temp, client, _primary) = fixture(
        key.clone(),
        key,
        [14; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    assert!(matches!(
        client.issue(issue_input()).await.unwrap(),
        ReplicaFundingDecision::Plan(_)
    ));
    let scope = client
        .handler_scope("external-request")
        .expect("active handler scope");
    let final_scope = scope.clone();
    drop(scope);
    tokio::task::yield_now().await;
    assert!(client.has_active("external-request"));
    assert!(
        client
            .claims()
            .load_pending_terminals(10)
            .await
            .unwrap()
            .is_empty()
    );

    drop(final_scope);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !client.has_active("external-request")
                && client
                    .claims()
                    .load_pending_terminals(10)
                    .await
                    .unwrap()
                    .len()
                    == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(only_claim_json(&temp)["state"], "release_pending");
    let terminals = client.claims().load_pending_terminals(10).await.unwrap();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].input.kind, TerminalKind::Release);
}

#[tokio::test]
async fn oversized_streaming_keyset_stops_at_limit_and_keeps_snapshot() {
    let old_key = verifier_key("old-key", [15; 32]);
    let next_key = verifier_key("next-key", [16; 32]);
    let (_temp, client, primary) = fixture(
        next_key.clone(),
        next_key,
        [16; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    client
        .verifier()
        .replace_snapshot(vec![old_key], Utc::now() - chrono::Duration::seconds(61))
        .unwrap();
    primary
        .oversized_response
        .store(OVERSIZED_KEYSET, Ordering::SeqCst);

    let error = tokio::time::timeout(
        Duration::from_millis(500),
        client.refresh_if_due(Utc::now()),
    )
    .await
    .expect("keyset reader must stop after byte 65537")
    .unwrap_err();
    assert_eq!(error.code(), "plan_admission_verification_unavailable");
    assert_eq!(client.verifier().key_ids(Utc::now()), vec!["old-key"]);
}

#[tokio::test]
async fn oversized_streaming_issue_stops_at_limit_without_claim() {
    let key = verifier_key("key-a", [17; 32]);
    let (temp, client, primary) = fixture(
        key.clone(),
        key.clone(),
        [17; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    client
        .verifier()
        .replace_snapshot(vec![key], Utc::now())
        .unwrap();
    primary
        .oversized_response
        .store(OVERSIZED_ISSUE, Ordering::SeqCst);

    let error = tokio::time::timeout(Duration::from_millis(500), client.issue(issue_input()))
        .await
        .expect("issue reader must stop after byte 65537")
        .unwrap_err();
    assert_eq!(error.code(), "plan_admission_issue_unavailable");
    assert!(
        temp.path()
            .join("claims")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn oversized_streaming_confirmation_stops_at_limit_and_releases_claim() {
    let key = verifier_key("key-a", [18; 32]);
    let (temp, client, primary) = fixture(
        key.clone(),
        key,
        [18; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    primary
        .oversized_response
        .store(OVERSIZED_CONFIRM, Ordering::SeqCst);

    let error = tokio::time::timeout(Duration::from_millis(500), client.issue(issue_input()))
        .await
        .expect("confirmation reader must stop after byte 65537")
        .unwrap_err();
    assert_eq!(error.code(), "plan_admission_confirmation_failed");
    assert_cancelled_issue_released(&temp, &client, &primary).await;
}

#[tokio::test]
async fn due_keyset_refresh_is_single_flight_and_reuses_fresh_snapshot() {
    let key = verifier_key("key-a", [1; 32]);
    let (_temp, client, primary) = fixture(
        key.clone(),
        key,
        [1; 32],
        Duration::from_millis(75),
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    let now = Utc::now();
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            client.refresh_if_due(now).await.unwrap()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    client
        .refresh_if_due(now + chrono::Duration::seconds(59))
        .await
        .unwrap();
    assert_eq!(primary.keyset_calls.load(Ordering::SeqCst), 1);
    client
        .refresh_if_due(now + chrono::Duration::seconds(60))
        .await
        .unwrap();
    assert_eq!(primary.keyset_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn scheduler_retries_failed_refresh_preserves_snapshot_and_stops_with_shutdown() {
    let old_key = verifier_key("old-key", [9; 32]);
    let next_key = verifier_key("next-key", [10; 32]);
    let (_temp, client, primary) = fixture(
        next_key.clone(),
        next_key,
        [10; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    client
        .verifier()
        .replace_snapshot(vec![old_key], Utc::now() - chrono::Duration::seconds(1))
        .unwrap();
    primary.keyset_failures_remaining.store(1, Ordering::SeqCst);
    let shutdown = Arc::new(AtomicBool::new(false));
    let handle = client
        .clone()
        .with_refresh_interval(Duration::from_millis(100))
        .spawn_keyset_refresh_loop(shutdown.clone());

    tokio::time::timeout(Duration::from_millis(80), async {
        while primary.keyset_calls.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(client.verifier().key_ids(Utc::now()), vec!["old-key"]);

    tokio::time::timeout(Duration::from_millis(250), async {
        while client.verifier().key_ids(Utc::now()) != vec!["next-key"] {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(primary.keyset_calls.load(Ordering::SeqCst) >= 2);
    assert_eq!(client.verifier().key_ids(Utc::now()), vec!["next-key"]);

    shutdown.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_millis(150), handle)
        .await
        .unwrap()
        .unwrap();
    let stopped_at = primary.keyset_calls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(primary.keyset_calls.load(Ordering::SeqCst), stopped_at);
}

#[tokio::test]
async fn balance_issue_creates_no_claim_and_sends_no_confirmation() {
    let key = verifier_key("key-a", [6; 32]);
    let (temp, client, primary) = fixture(
        key.clone(),
        key,
        [6; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    primary.return_balance.store(true, Ordering::SeqCst);
    assert_eq!(
        client.issue(issue_input()).await.unwrap(),
        ReplicaFundingDecision::Balance
    );
    assert_eq!(primary.confirm_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read_dir(temp.path().join("claims"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn unknown_kid_refreshes_only_once_then_fails_before_claim_and_confirmation() {
    let first = verifier_key("key-a", [7; 32]);
    let issue_key = verifier_key("key-b", [8; 32]);
    let (temp, client, primary) = fixture(
        first,
        issue_key,
        [8; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    primary.retain_first_keyset.store(true, Ordering::SeqCst);
    let error = client.issue(issue_input()).await.unwrap_err();
    assert_eq!(error.code(), "plan_admission_verification_unavailable");
    assert_eq!(primary.keyset_calls.load(Ordering::SeqCst), 2);
    assert_eq!(primary.confirm_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read_dir(temp.path().join("claims"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn unknown_kid_refreshes_once_then_claims_confirms_routes_and_settles_after_expiry() {
    let first = verifier_key("key-a", [1; 32]);
    let issue_key = verifier_key("key-b", [2; 32]);
    let (temp, client, primary) = fixture(
        first,
        issue_key,
        [2; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;

    let decision = client.issue(issue_input()).await.unwrap();
    let admission = match decision {
        ReplicaFundingDecision::Plan(admission) => admission,
        ReplicaFundingDecision::Balance => panic!("expected Plan"),
    };
    assert_eq!(primary.keyset_calls.load(Ordering::SeqCst), 2);
    assert_eq!(primary.issue_calls.load(Ordering::SeqCst), 1);
    assert_eq!(primary.confirm_calls.load(Ordering::SeqCst), 1);
    assert_eq!(only_claim_json(&temp)["state"], "confirmed");

    let routed_at = admission.expires_at - chrono::Duration::seconds(1);
    client
        .mark_routed(&admission.request_id, routed_at)
        .await
        .unwrap();
    assert_eq!(only_claim_json(&temp)["state"], "routed");
    client
        .settle(
            &admission.request_id,
            750_000,
            admission.expires_at + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();
    let terminals = client.claims().load_pending_terminals(10).await.unwrap();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].input.kind, TerminalKind::Settlement);
    assert_eq!(terminals[0].input.actual_nano_usd, Some(750_000));
}

#[tokio::test]
async fn confirmation_response_loss_retries_exactly_once_before_confirming() {
    let key = verifier_key("key-a", [3; 32]);
    let (temp, client, primary) = fixture(
        key.clone(),
        key,
        [3; 32],
        Duration::ZERO,
        ConfirmMode::FirstResponseLost,
        Duration::from_millis(100),
    )
    .await;
    assert!(matches!(
        client.issue(issue_input()).await.unwrap(),
        ReplicaFundingDecision::Plan(_)
    ));
    assert_eq!(primary.confirm_calls.load(Ordering::SeqCst), 2);
    assert_eq!(only_claim_json(&temp)["state"], "confirmed");
}

#[tokio::test]
async fn confirmation_rejection_enters_irreversible_release_pending_and_publishes_release() {
    let key = verifier_key("key-a", [4; 32]);
    let (temp, client, primary) = fixture(
        key.clone(),
        key,
        [4; 32],
        Duration::ZERO,
        ConfirmMode::Reject,
        Duration::from_secs(2),
    )
    .await;
    let error = client.issue(issue_input()).await.unwrap_err();
    assert_eq!(error.code(), "plan_admission_confirmation_failed");
    assert_eq!(primary.confirm_calls.load(Ordering::SeqCst), 1);
    assert_eq!(only_claim_json(&temp)["state"], "release_pending");
    let terminals = client.claims().load_pending_terminals(10).await.unwrap();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].input.kind, TerminalKind::Release);
    assert!(!client.has_active("external-request"));
}

#[tokio::test]
async fn failed_route_marker_prevents_dispatch_and_finishes_with_release() {
    let key = verifier_key("key-a", [5; 32]);
    let (temp, client, _primary) = fixture(
        key.clone(),
        key,
        [5; 32],
        Duration::ZERO,
        ConfirmMode::Success,
        Duration::from_secs(2),
    )
    .await;
    let admission = match client.issue(issue_input()).await.unwrap() {
        ReplicaFundingDecision::Plan(admission) => admission,
        ReplicaFundingDecision::Balance => panic!("expected Plan"),
    };
    client
        .claims()
        .mark_release_pending(&admission.token_id)
        .await
        .unwrap();
    let error = client
        .mark_routed(&admission.request_id, Utc::now())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "plan_admission_dispatch_unavailable");
    assert_eq!(only_claim_json(&temp)["state"], "release_pending");
    let terminals = client.claims().load_pending_terminals(10).await.unwrap();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].input.kind, TerminalKind::Release);
}
