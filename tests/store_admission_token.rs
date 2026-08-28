use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeZone, Utc};
use ed25519_dalek::SigningKey;
use monoize::replica::metering::MeteringSpoolCapacity;
use monoize::store_billing::admission_token::{
    AdmissionBinding, AdmissionClaimStore, AdmissionError, AdmissionKeyRing, AdmissionSigningKey,
    AdmissionTokenInput, AdmissionVerificationBinding, AdmissionVerifierKey, AdmissionVerifierRing,
    PlanTerminalAcknowledgement, PriorAdmissionSigningKey, TerminalAcknowledgementResult,
    TerminalSpoolInput, validate_terminal_acknowledgements,
};
use tempfile::tempdir;

fn signing_key(id: &str, seed: u8, activated_at: chrono::DateTime<Utc>) -> AdmissionSigningKey {
    AdmissionSigningKey::from_seed(id, [seed; 32], activated_at).unwrap()
}

fn token_input(now: chrono::DateTime<Utc>) -> AdmissionTokenInput {
    AdmissionTokenInput {
        audience: "replica-a".to_string(),
        token_id: "token-1".to_string(),
        reservation_id: "reservation-1".to_string(),
        request_id: "request-1".to_string(),
        entitlement_id: "entitlement-1".to_string(),
        generation: 7,
        maximum_nano_usd: 50_000_000,
        reserved_fen_cny: 30,
        pricing_revision: "pricing-v3".to_string(),
        issued_at: now,
    }
}

fn binding() -> AdmissionBinding {
    AdmissionBinding {
        audience: "replica-a".to_string(),
        reservation_id: "reservation-1".to_string(),
        request_id: "request-1".to_string(),
        entitlement_id: "entitlement-1".to_string(),
        generation: 7,
        maximum_nano_usd: 50_000_000,
        reserved_fen_cny: 30,
        pricing_revision: "pricing-v3".to_string(),
    }
}

fn verifier_key(
    id: &str,
    seed: u8,
    state: &str,
    activated_at: chrono::DateTime<Utc>,
    verify_until: Option<chrono::DateTime<Utc>>,
) -> AdmissionVerifierKey {
    AdmissionVerifierKey {
        key_id: id.to_string(),
        public_key_base64: URL_SAFE_NO_PAD.encode(
            SigningKey::from_bytes(&[seed; 32])
                .verifying_key()
                .as_bytes(),
        ),
        state: state.to_string(),
        activated_at,
        verify_until,
    }
}

fn verification_binding(now: chrono::DateTime<Utc>) -> AdmissionVerificationBinding {
    AdmissionVerificationBinding {
        audience: "replica-a".to_string(),
        token_id: "token-1".to_string(),
        reservation_id: "reservation-1".to_string(),
        request_id: "request-1".to_string(),
        maximum_nano_usd: 50_000_000,
        pricing_revision: "pricing-v3".to_string(),
        issued_at: now,
        expires_at: now + chrono::Duration::seconds(30),
    }
}

#[test]
fn compact_jws_binds_every_admission_field() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 6, 0, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 41, now),
        vec![],
    )
    .unwrap();
    let token = ring.issue(token_input(now)).unwrap();
    assert_eq!(token.split('.').count(), 3);

    let verified = ring.verify(&token, &binding(), now).unwrap();
    assert_eq!(verified.issuer, "lynshen-primary");
    assert_eq!(verified.key_id, "admission-key-1");
    assert_eq!(verified.token_id, "token-1");
    assert_eq!(verified.expires_at, now + chrono::Duration::seconds(30));

    let mut wrong_request = binding();
    wrong_request.request_id = "request-other".to_string();
    assert_eq!(
        ring.verify(&token, &wrong_request, now).unwrap_err(),
        AdmissionError::BindingMismatch
    );
    let mut wrong_audience = binding();
    wrong_audience.audience = "replica-b".to_string();
    assert_eq!(
        ring.verify(&token, &wrong_audience, now).unwrap_err(),
        AdmissionError::WrongAudience
    );
}

#[test]
fn verification_uses_exactly_five_seconds_of_clock_skew() {
    let issued_at = Utc.with_ymd_and_hms(2026, 8, 28, 7, 0, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 42, issued_at),
        vec![],
    )
    .unwrap();
    let token = ring.issue(token_input(issued_at)).unwrap();

    ring.verify(&token, &binding(), issued_at - chrono::Duration::seconds(5))
        .unwrap();
    assert_eq!(
        ring.verify(&token, &binding(), issued_at - chrono::Duration::seconds(6),)
            .unwrap_err(),
        AdmissionError::NotYetValid
    );
    ring.verify(
        &token,
        &binding(),
        issued_at + chrono::Duration::seconds(34),
    )
    .unwrap();
    assert_eq!(
        ring.verify(
            &token,
            &binding(),
            issued_at + chrono::Duration::seconds(35),
        )
        .unwrap_err(),
        AdmissionError::Expired
    );
    assert_eq!(
        ring.verify(&token, &binding(), issued_at - chrono::Duration::minutes(2),)
            .unwrap_err(),
        AdmissionError::NotYetValid
    );
    assert_eq!(
        ring.verify(&token, &binding(), issued_at + chrono::Duration::minutes(2),)
            .unwrap_err(),
        AdmissionError::Expired
    );
}

#[test]
fn rotation_retains_the_prior_verifier_for_at_least_five_minutes() {
    let activated = Utc.with_ymd_and_hms(2026, 8, 28, 8, 0, 0).unwrap();
    let ring = AdmissionKeyRing::new("lynshen-primary", signing_key("old", 43, activated), vec![])
        .unwrap();
    let rotated_at = activated + chrono::Duration::minutes(1);
    ring.publish(signing_key("next", 44, rotated_at)).unwrap();
    ring.activate("next", rotated_at).unwrap();

    assert!(
        ring.verification_key_ids(rotated_at + chrono::Duration::seconds(299))
            .iter()
            .any(|id| id == "old")
    );
    ring.prune_retired(rotated_at + chrono::Duration::seconds(300));
    assert!(
        !ring
            .verification_key_ids(rotated_at + chrono::Duration::seconds(300))
            .iter()
            .any(|id| id == "old")
    );
    assert_eq!(ring.active_key_id(), "next");
}

#[test]
fn restart_retains_a_prior_key_from_its_deactivation_time() {
    let deactivated_at = Utc.with_ymd_and_hms(2026, 8, 28, 8, 0, 0).unwrap();
    let prior = PriorAdmissionSigningKey {
        key: signing_key("old", 55, deactivated_at - chrono::Duration::hours(1)),
        deactivated_at,
        last_issued_expires_at: None,
        verify_until: Some(deactivated_at + chrono::Duration::seconds(10)),
    };
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("current", 56, deactivated_at),
        vec![prior],
    )
    .unwrap();

    assert!(
        ring.verification_key_ids(deactivated_at + chrono::Duration::seconds(299))
            .iter()
            .any(|id| id == "old")
    );
    assert!(
        !ring
            .verification_key_ids(deactivated_at + chrono::Duration::seconds(300))
            .iter()
            .any(|id| id == "old")
    );
}

#[test]
fn restart_extends_prior_key_retention_through_the_last_token_skew() {
    let deactivated_at = Utc.with_ymd_and_hms(2026, 8, 28, 9, 0, 0).unwrap();
    let last_issued_expires_at = deactivated_at + chrono::Duration::seconds(360);
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("current", 57, deactivated_at),
        vec![PriorAdmissionSigningKey {
            key: signing_key("old", 58, deactivated_at - chrono::Duration::hours(1)),
            deactivated_at,
            last_issued_expires_at: Some(last_issued_expires_at),
            verify_until: None,
        }],
    )
    .unwrap();

    assert!(
        ring.verification_key_ids(last_issued_expires_at + chrono::Duration::seconds(4))
            .iter()
            .any(|id| id == "old")
    );
    assert!(
        !ring
            .verification_key_ids(last_issued_expires_at + chrono::Duration::seconds(5))
            .iter()
            .any(|id| id == "old")
    );
}

#[test]
fn restart_honors_a_persisted_later_prior_key_deadline() {
    let deactivated_at = Utc.with_ymd_and_hms(2026, 8, 28, 10, 0, 0).unwrap();
    let persisted = deactivated_at + chrono::Duration::minutes(12);
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("current", 59, deactivated_at),
        vec![PriorAdmissionSigningKey {
            key: signing_key("old", 60, deactivated_at - chrono::Duration::hours(1)),
            deactivated_at,
            last_issued_expires_at: None,
            verify_until: Some(persisted),
        }],
    )
    .unwrap();

    assert!(
        ring.verification_key_ids(persisted - chrono::Duration::seconds(1))
            .iter()
            .any(|id| id == "old")
    );
    assert!(
        !ring
            .verification_key_ids(persisted)
            .iter()
            .any(|id| id == "old")
    );
}

#[test]
fn tamper_unknown_key_and_noncanonical_base64url_are_rejected() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 8, 30, 0).unwrap();
    let ring =
        AdmissionKeyRing::new("lynshen-primary", signing_key("known", 50, now), vec![]).unwrap();
    let token = ring.issue(token_input(now)).unwrap();
    let mut segments = token.split('.').map(str::to_string).collect::<Vec<_>>();
    let last = segments[1].pop().unwrap();
    segments[1].push(if last == 'A' { 'B' } else { 'A' });
    assert_eq!(
        ring.verify(&segments.join("."), &binding(), now)
            .unwrap_err(),
        AdmissionError::InvalidSignature
    );

    let other_ring =
        AdmissionKeyRing::new("lynshen-primary", signing_key("other", 51, now), vec![]).unwrap();
    assert_eq!(
        other_ring.verify(&token, &binding(), now).unwrap_err(),
        AdmissionError::UnknownKey
    );

    let mut padded = token.split('.').map(str::to_string).collect::<Vec<_>>();
    padded[0].push('=');
    assert_eq!(
        ring.verify(&padded.join("."), &binding(), now).unwrap_err(),
        AdmissionError::NonCanonicalToken
    );
}

#[test]
fn prior_key_verifies_tokens_after_rotation_within_the_token_window() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 8, 45, 0).unwrap();
    let ring =
        AdmissionKeyRing::new("lynshen-primary", signing_key("old", 52, now), vec![]).unwrap();
    let token = ring.issue(token_input(now)).unwrap();
    ring.publish(signing_key("next", 53, now + chrono::Duration::seconds(1)))
        .unwrap();
    ring.activate("next", now + chrono::Duration::seconds(1))
        .unwrap();
    ring.verify(&token, &binding(), now + chrono::Duration::seconds(10))
        .unwrap();
}

#[test]
fn verifier_snapshot_rejects_every_invalid_keyset_shape() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 8, 50, 0).unwrap();
    let active = verifier_key("active", 71, "active", now, None);
    let retired = verifier_key(
        "retired",
        72,
        "retired",
        now - chrono::Duration::hours(1),
        Some(now + chrono::Duration::minutes(5)),
    );
    let mut padded = active.clone();
    padded.public_key_base64.push('=');
    let mut wrong_size = active.clone();
    wrong_size.public_key_base64 = URL_SAFE_NO_PAD.encode([1_u8; 31]);
    let mut invalid_state = active.clone();
    invalid_state.state = "published".to_string();
    let mut active_with_deadline = active.clone();
    active_with_deadline.verify_until = Some(now + chrono::Duration::minutes(5));
    let mut retired_without_deadline = retired.clone();
    retired_without_deadline.verify_until = None;
    let mut retired_with_expired_deadline = retired.clone();
    retired_with_expired_deadline.verify_until = Some(now);

    let cases = vec![
        vec![active.clone(), active.clone()],
        vec![padded],
        vec![wrong_size],
        vec![invalid_state],
        vec![
            active.clone(),
            verifier_key("active-2", 73, "active", now, None),
        ],
        vec![active_with_deadline],
        vec![retired_without_deadline],
        vec![retired_with_expired_deadline],
    ];
    for keys in cases {
        let ring = AdmissionVerifierRing::new();
        assert_eq!(
            ring.replace_snapshot(keys, now).unwrap_err(),
            AdmissionError::VerifierSnapshotInvalid
        );
        assert!(ring.key_ids(now).is_empty());
    }
}

#[test]
fn verifier_snapshot_replacement_is_atomic_and_removes_absent_keys() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 8, 55, 0).unwrap();
    let signer =
        AdmissionKeyRing::new("lynshen-primary", signing_key("old", 74, now), vec![]).unwrap();
    let token = signer.issue(token_input(now)).unwrap();
    let ring = AdmissionVerifierRing::new();
    ring.replace_snapshot(vec![verifier_key("old", 74, "active", now, None)], now)
        .unwrap();
    ring.verify(&token, &verification_binding(now), now)
        .unwrap();

    let duplicate = verifier_key("duplicate", 75, "active", now, None);
    assert_eq!(
        ring.replace_snapshot(vec![duplicate.clone(), duplicate], now)
            .unwrap_err(),
        AdmissionError::VerifierSnapshotInvalid
    );
    ring.verify(&token, &verification_binding(now), now)
        .unwrap();

    ring.replace_snapshot(vec![verifier_key("next", 76, "active", now, None)], now)
        .unwrap();
    assert_eq!(
        ring.verify(&token, &verification_binding(now), now)
            .unwrap_err(),
        AdmissionError::UnknownKey
    );
    assert_eq!(ring.key_ids(now), vec!["next"]);
}

#[test]
fn verifier_binds_compact_jws_and_uses_the_fixed_issuer() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 8, 58, 0).unwrap();
    let signer =
        AdmissionKeyRing::new("lynshen-primary", signing_key("verify", 77, now), vec![]).unwrap();
    let token = signer.issue(token_input(now)).unwrap();
    let ring = AdmissionVerifierRing::new();
    ring.replace_snapshot(vec![verifier_key("verify", 77, "active", now, None)], now)
        .unwrap();
    let verified = ring
        .verify(&token, &verification_binding(now), now)
        .unwrap();
    assert_eq!(verified.entitlement_id, "entitlement-1");
    assert_eq!(verified.generation, 7);
    assert_eq!(verified.reserved_fen_cny, 30);

    let mut changed = verification_binding(now);
    changed.token_id = "token-other".to_string();
    assert_eq!(
        ring.verify(&token, &changed, now).unwrap_err(),
        AdmissionError::BindingMismatch
    );
    changed = verification_binding(now);
    changed.expires_at += chrono::Duration::seconds(1);
    assert_eq!(
        ring.verify(&token, &changed, now).unwrap_err(),
        AdmissionError::BindingMismatch
    );

    let wrong_issuer =
        AdmissionKeyRing::new("other-primary", signing_key("verify", 77, now), vec![])
            .unwrap()
            .issue(token_input(now))
            .unwrap();
    assert_eq!(
        ring.verify(&wrong_issuer, &verification_binding(now), now)
            .unwrap_err(),
        AdmissionError::WrongIssuer
    );
}

#[test]
fn terminal_acknowledgement_requires_exactly_one_matching_valid_entry() {
    let expected_token = "token-1";
    let expected_digest = "a".repeat(64);
    for result in [
        TerminalAcknowledgementResult::Applied,
        TerminalAcknowledgementResult::Duplicate,
    ] {
        assert!(validate_terminal_acknowledgements(
            &[PlanTerminalAcknowledgement {
                token_id: expected_token.to_string(),
                canonical_digest: expected_digest.clone(),
                result,
            }],
            expected_token,
            &expected_digest,
        ));
    }
    let matching = PlanTerminalAcknowledgement {
        token_id: expected_token.to_string(),
        canonical_digest: expected_digest.clone(),
        result: TerminalAcknowledgementResult::Applied,
    };
    assert!(!validate_terminal_acknowledgements(
        &[],
        expected_token,
        &expected_digest
    ));
    assert!(!validate_terminal_acknowledgements(
        &[matching.clone(), matching.clone()],
        expected_token,
        &expected_digest,
    ));
    let mut mismatch = matching.clone();
    mismatch.canonical_digest = "b".repeat(64);
    assert!(!validate_terminal_acknowledgements(
        &[mismatch],
        expected_token,
        &expected_digest,
    ));
    let mut unexpected = matching.clone();
    unexpected.token_id = "unexpected-token".to_string();
    assert!(!validate_terminal_acknowledgements(
        &[matching, unexpected],
        expected_token,
        &expected_digest,
    ));
}

#[tokio::test]
async fn claim_marker_and_terminal_spool_are_durable_and_replay_safe() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 0, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 45, now),
        vec![],
    )
    .unwrap();
    let token = ring.issue(token_input(now)).unwrap();
    let verified = ring.verify(&token, &binding(), now).unwrap();
    let temp = tempdir().unwrap();
    let claims = AdmissionClaimStore::new(temp.path()).await.unwrap();

    claims.claim(&verified).await.unwrap();
    assert!(claims.marker_exists("token-1").await.unwrap());
    assert_eq!(
        claims.claim(&verified).await.unwrap_err(),
        AdmissionError::Replay
    );

    let terminal = claims
        .spool_terminal(TerminalSpoolInput::settlement(
            &verified,
            40_000_000,
            now + chrono::Duration::seconds(10),
        ))
        .await
        .unwrap();
    assert!(terminal.path.exists());
    let terminal_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&terminal.path).unwrap()).unwrap();
    assert_eq!(terminal_json["version"], 1);
    assert_eq!(terminal_json["actual_nano_usd"], "40000000");
    assert_eq!(terminal_json["canonical_digest"], terminal.canonical_digest);
    assert_eq!(
        claims
            .acknowledge_terminal(
                "token-1",
                "0".repeat(64).as_str(),
                now + chrono::Duration::minutes(4),
            )
            .await
            .unwrap_err(),
        AdmissionError::TerminalAcknowledgementMismatch
    );
    assert!(terminal.path.exists());
    claims
        .acknowledge_terminal(
            "token-1",
            &terminal.canonical_digest,
            now + chrono::Duration::minutes(4),
        )
        .await
        .unwrap();
    assert!(!terminal.path.exists());
    assert!(claims.marker_exists("token-1").await.unwrap());
    assert!(
        claims
            .cleanup_acknowledged(verified.expires_at + chrono::Duration::seconds(299))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        claims
            .cleanup_acknowledged(verified.expires_at + chrono::Duration::seconds(300))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(!claims.marker_exists("token-1").await.unwrap());
}

#[tokio::test]
async fn admission_files_ignore_temp_residue_and_acknowledgement_replaces_complete_json() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 30, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 61, now),
        vec![],
    )
    .unwrap();
    let verified = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let claims = AdmissionClaimStore::new(temp.path()).await.unwrap();
    std::fs::write(temp.path().join("claims").join("claim-orphan.tmp"), b"{").unwrap();
    std::fs::write(
        temp.path().join("terminal").join("terminal-orphan.tmp"),
        b"{",
    )
    .unwrap();

    assert!(!claims.marker_exists("orphan").await.unwrap());
    assert_eq!(
        claims
            .spool_terminal(TerminalSpoolInput::release(&verified, now))
            .await
            .unwrap_err(),
        AdmissionError::ClaimMissing
    );

    let marker_path = claims.claim(&verified).await.unwrap();
    assert_eq!(
        claims.claim(&verified).await.unwrap_err(),
        AdmissionError::Replay
    );
    let terminal =
        TerminalSpoolInput::settlement(&verified, 40_000_000, now + chrono::Duration::seconds(1));
    let first = claims.spool_terminal(terminal.clone()).await.unwrap();
    assert_eq!(claims.spool_terminal(terminal).await.unwrap(), first);
    assert_eq!(
        claims
            .spool_terminal(TerminalSpoolInput::release(
                &verified,
                now + chrono::Duration::seconds(1),
            ))
            .await
            .unwrap_err(),
        AdmissionError::TerminalConflict
    );

    let acknowledged_at = now + chrono::Duration::minutes(1);
    claims
        .acknowledge_terminal("token-1", &first.canonical_digest, acknowledged_at)
        .await
        .unwrap();
    assert!(!first.path.exists());
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(marker_path).unwrap()).unwrap();
    assert_eq!(marker["token_id"], "token-1");
    assert_eq!(marker["reservation_id"], "reservation-1");
    assert_eq!(marker["request_id"], "request-1");
    assert_eq!(marker["audience"], "replica-a");
    assert_eq!(marker["version"], 1);
    assert_eq!(marker["maximum_nano_usd"], "50000000");
    assert_eq!(marker["state"], "claimed");
    assert!(marker["routed_at"].is_null());
    assert_eq!(marker["terminal_reserved_bytes"], 4096);
    assert_eq!(
        DateTime::parse_from_rfc3339(marker["acknowledged_at"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc),
        acknowledged_at
    );
    for (directory, permitted_residue) in [
        ("claims", "claim-orphan.tmp"),
        ("terminal", "terminal-orphan.tmp"),
    ] {
        let temp_names = std::fs::read_dir(temp.path().join(directory))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect::<Vec<_>>();
        assert_eq!(temp_names, vec![permitted_residue]);
    }
}

#[tokio::test]
async fn admission_store_startup_removes_only_strict_residual_temp_names() {
    let temp = tempdir().unwrap();
    let claims_dir = temp.path().join("claims");
    let terminal_dir = temp.path().join("terminal");
    std::fs::create_dir_all(&claims_dir).unwrap();
    std::fs::create_dir_all(&terminal_dir).unwrap();
    let digest = "a".repeat(64);
    let uuid = "123e4567-e89b-42d3-a456-426614174000";
    let claim_residue = format!(".claim-{digest}.json.{uuid}.tmp");
    let terminal_residue = format!(".terminal-{digest}.json.{uuid}.tmp");
    std::fs::write(claims_dir.join(&claim_residue), b"partial").unwrap();
    std::fs::write(terminal_dir.join(&terminal_residue), b"partial").unwrap();

    let preserved = [
        format!("unrelated-{digest}.json"),
        format!(".claim-{}.json.{uuid}.tmp", "B".repeat(64)),
        format!(".claim-{digest}.json.123e4567-e89b-12d3-a456-426614174000.tmp"),
        format!(".terminal-{digest}.json.{uuid}.tmp"),
        "claim-orphan.tmp".to_string(),
    ];
    for name in &preserved {
        std::fs::write(claims_dir.join(name), b"preserve").unwrap();
    }
    let matching_directory = claims_dir.join(format!(
        ".claim-{digest}.json.223e4567-e89b-42d3-a456-426614174000.tmp"
    ));
    std::fs::create_dir(&matching_directory).unwrap();

    AdmissionClaimStore::new(temp.path()).await.unwrap();

    assert!(!claims_dir.join(claim_residue).exists());
    assert!(!terminal_dir.join(terminal_residue).exists());
    for name in preserved {
        assert!(claims_dir.join(&name).exists(), "startup removed {name}");
    }
    assert!(matching_directory.is_dir());
}

#[tokio::test]
async fn claim_lifecycle_transitions_are_durable_and_release_pending_is_irreversible() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 40, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 78, now),
        vec![],
    )
    .unwrap();
    let verified = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let claims = AdmissionClaimStore::new(temp.path()).await.unwrap();
    let marker_path = claims.claim(&verified).await.unwrap();

    claims.mark_confirmed("token-1").await.unwrap();
    let confirmed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
    assert_eq!(confirmed["state"], "confirmed");
    assert!(confirmed["routed_at"].is_null());

    let routed_at = now + chrono::Duration::seconds(2);
    claims.mark_routed("token-1", routed_at).await.unwrap();
    let routed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
    assert_eq!(routed["state"], "routed");
    assert_eq!(
        DateTime::parse_from_rfc3339(routed["routed_at"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc),
        routed_at
    );

    claims.mark_release_pending("token-1").await.unwrap();
    let release_pending: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
    assert_eq!(release_pending["state"], "release_pending");
    assert!(release_pending["routed_at"].is_null());
    assert_eq!(
        claims.mark_confirmed("token-1").await.unwrap_err(),
        AdmissionError::InvalidClaimTransition
    );
}

#[tokio::test]
async fn startup_recovers_abandoned_claims_by_their_durable_state() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 45, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 79, now),
        vec![],
    )
    .unwrap();
    let base = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let claims = AdmissionClaimStore::new(temp.path()).await.unwrap();
    for token_id in ["claimed", "confirmed", "routed", "release-pending"] {
        let mut admission = base.clone();
        admission.token_id = token_id.to_string();
        claims.claim(&admission).await.unwrap();
    }
    claims.mark_confirmed("confirmed").await.unwrap();
    claims.mark_confirmed("routed").await.unwrap();
    claims
        .mark_routed("routed", now + chrono::Duration::seconds(1))
        .await
        .unwrap();
    claims
        .mark_release_pending("release-pending")
        .await
        .unwrap();
    drop(claims);

    let reopened = AdmissionClaimStore::new(temp.path()).await.unwrap();
    let pending = reopened.load_pending_terminals(10).await.unwrap();
    assert_eq!(pending.len(), 4);
    for terminal in pending {
        if terminal.input.token_id == "routed" {
            assert_eq!(terminal.input.actual_nano_usd, Some(50_000_000));
        } else {
            assert_eq!(terminal.input.actual_nano_usd, None);
        }
    }
}

#[tokio::test]
async fn pending_terminals_use_deterministic_order_and_preserve_the_limit_tail() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 50, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 80, now),
        vec![],
    )
    .unwrap();
    let base = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let claims = AdmissionClaimStore::new(temp.path()).await.unwrap();
    for (token_id, created_at) in [
        ("z-token", now),
        ("old-token", now - chrono::Duration::seconds(1)),
        ("a-token", now),
    ] {
        let mut admission = base.clone();
        admission.token_id = token_id.to_string();
        claims.claim(&admission).await.unwrap();
        claims
            .spool_terminal(TerminalSpoolInput::release(&admission, created_at))
            .await
            .unwrap();
    }

    let selected = claims.load_pending_terminals(2).await.unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|record| record.input.token_id.as_str())
            .collect::<Vec<_>>(),
        vec!["old-token", "a-token"]
    );
    assert_eq!(
        std::fs::read_dir(temp.path().join("terminal"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(
                |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json")
            )
            .count(),
        3
    );
}

#[tokio::test]
async fn startup_rejects_corrupt_claim_and_terminal_final_files() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 55, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 81, now),
        vec![],
    )
    .unwrap();
    let verified = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();

    let unknown_version = tempdir().unwrap();
    let store = AdmissionClaimStore::new(unknown_version.path())
        .await
        .unwrap();
    let path = store.claim(&verified).await.unwrap();
    let mut marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    marker["version"] = 2.into();
    std::fs::write(&path, serde_json::to_vec(&marker).unwrap()).unwrap();
    drop(store);
    assert_eq!(
        AdmissionClaimStore::new(unknown_version.path())
            .await
            .unwrap_err(),
        AdmissionError::SpoolCorrupt
    );

    let wrong_name = tempdir().unwrap();
    let store = AdmissionClaimStore::new(wrong_name.path()).await.unwrap();
    let path = store.claim(&verified).await.unwrap();
    std::fs::rename(
        &path,
        wrong_name
            .path()
            .join("claims")
            .join(format!("claim-{}.json", "a".repeat(64))),
    )
    .unwrap();
    drop(store);
    assert_eq!(
        AdmissionClaimStore::new(wrong_name.path())
            .await
            .unwrap_err(),
        AdmissionError::SpoolCorrupt
    );

    let malformed = tempdir().unwrap();
    std::fs::create_dir_all(malformed.path().join("claims")).unwrap();
    std::fs::create_dir_all(malformed.path().join("terminal")).unwrap();
    std::fs::write(
        malformed
            .path()
            .join("claims")
            .join(format!("claim-{}.json", "b".repeat(64))),
        b"{",
    )
    .unwrap();
    assert_eq!(
        AdmissionClaimStore::new(malformed.path())
            .await
            .unwrap_err(),
        AdmissionError::SpoolCorrupt
    );

    let bad_digest = tempdir().unwrap();
    let store = AdmissionClaimStore::new(bad_digest.path()).await.unwrap();
    store.claim(&verified).await.unwrap();
    let terminal = store
        .spool_terminal(TerminalSpoolInput::release(&verified, now))
        .await
        .unwrap();
    let mut wire: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&terminal.path).unwrap()).unwrap();
    wire["canonical_digest"] = "0".repeat(64).into();
    std::fs::write(&terminal.path, serde_json::to_vec(&wire).unwrap()).unwrap();
    drop(store);
    assert_eq!(
        AdmissionClaimStore::new(bad_digest.path())
            .await
            .unwrap_err(),
        AdmissionError::SpoolCorrupt
    );
}

#[tokio::test]
async fn shared_capacity_tracks_claim_reservation_terminal_ack_and_cleanup() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 57, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 82, now),
        vec![],
    )
    .unwrap();
    let verified = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let capacity = Arc::new(MeteringSpoolCapacity::new(1024 * 1024));
    let claims = AdmissionClaimStore::new_with_capacity(temp.path(), capacity.clone())
        .await
        .unwrap();

    let marker_path = claims.claim(&verified).await.unwrap();
    let claim_bytes = std::fs::metadata(&marker_path).unwrap().len();
    assert_eq!(capacity.accounted_bytes(), claim_bytes + 4096);

    let terminal = claims
        .spool_terminal(TerminalSpoolInput::release(&verified, now))
        .await
        .unwrap();
    let terminal_bytes = std::fs::metadata(&terminal.path).unwrap().len();
    assert_eq!(capacity.accounted_bytes(), claim_bytes + terminal_bytes);

    claims
        .acknowledge_terminal("token-1", &terminal.canonical_digest, now)
        .await
        .unwrap();
    let acknowledged_claim_bytes = std::fs::metadata(&marker_path).unwrap().len();
    assert_eq!(capacity.accounted_bytes(), acknowledged_claim_bytes);
    claims
        .cleanup_acknowledged(verified.expires_at + chrono::Duration::seconds(300))
        .await
        .unwrap();
    assert_eq!(capacity.accounted_bytes(), 0);
}

#[tokio::test]
async fn acknowledgement_crash_replay_deletes_terminal_and_retains_marker() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 58, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 84, now),
        vec![],
    )
    .unwrap();
    let verified = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let initial = AdmissionClaimStore::new(temp.path()).await.unwrap();
    let marker_path = initial.claim(&verified).await.unwrap();
    let terminal = initial
        .spool_terminal(TerminalSpoolInput::release(&verified, now))
        .await
        .unwrap();
    let mut marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
    marker["acknowledged_at"] = (now + chrono::Duration::seconds(1)).to_rfc3339().into();
    std::fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
    drop(initial);

    let replay = AdmissionClaimStore::new(temp.path()).await.unwrap();
    replay
        .acknowledge_terminal(
            "token-1",
            &terminal.canonical_digest,
            now + chrono::Duration::seconds(2),
        )
        .await
        .unwrap();
    assert!(!terminal.path.exists());
    drop(replay);

    let after_delete = AdmissionClaimStore::new(temp.path()).await.unwrap();
    assert!(after_delete.marker_exists("token-1").await.unwrap());
    assert!(
        after_delete
            .load_pending_terminals(10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        after_delete
            .cleanup_acknowledged(verified.expires_at + chrono::Duration::seconds(299))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn startup_reconstructs_over_limit_capacity_but_blocks_new_publication() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 59, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 83, now),
        vec![],
    )
    .unwrap();
    let verified = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let initial_capacity = Arc::new(MeteringSpoolCapacity::new(1024 * 1024));
    let initial = AdmissionClaimStore::new_with_capacity(temp.path(), initial_capacity)
        .await
        .unwrap();
    initial.claim(&verified).await.unwrap();
    initial
        .spool_terminal(TerminalSpoolInput::release(&verified, now))
        .await
        .unwrap();
    drop(initial);

    let constrained = Arc::new(MeteringSpoolCapacity::new(1));
    let reopened = AdmissionClaimStore::new_with_capacity(temp.path(), constrained.clone())
        .await
        .unwrap();
    assert!(constrained.accounted_bytes() > 1);
    assert_eq!(reopened.load_pending_terminals(10).await.unwrap().len(), 1);
    let mut another = verified.clone();
    another.token_id = "another-token".to_string();
    assert_eq!(
        reopened.claim(&another).await.unwrap_err(),
        AdmissionError::SpoolQuotaExhausted
    );
}

#[tokio::test]
async fn marker_or_spool_write_failure_fails_closed() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 10, 0, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 46, now),
        vec![],
    )
    .unwrap();
    let verified = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let invalid_root = temp.path().join("not-a-directory");
    std::fs::write(&invalid_root, b"file").unwrap();

    assert!(AdmissionClaimStore::new(&invalid_root).await.is_err());

    let claims = AdmissionClaimStore::new(temp.path().join("valid"))
        .await
        .unwrap();
    claims.claim(&verified).await.unwrap();
    let terminal_dir = temp.path().join("valid").join("terminal");
    std::fs::remove_dir(&terminal_dir).unwrap();
    std::fs::write(&terminal_dir, b"file").unwrap();
    assert!(
        claims
            .spool_terminal(TerminalSpoolInput::release(
                &verified,
                now + chrono::Duration::seconds(1),
            ))
            .await
            .is_err()
    );
    assert!(claims.marker_exists("token-1").await.unwrap());
}

#[tokio::test]
async fn concurrent_claim_has_one_winner_and_terminal_content_conflicts() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 10, 30, 0).unwrap();
    let ring = AdmissionKeyRing::new(
        "lynshen-primary",
        signing_key("admission-key-1", 54, now),
        vec![],
    )
    .unwrap();
    let verified = ring
        .verify(&ring.issue(token_input(now)).unwrap(), &binding(), now)
        .unwrap();
    let temp = tempdir().unwrap();
    let claims = AdmissionClaimStore::new(temp.path()).await.unwrap();
    let first = claims.clone();
    let second = claims.clone();
    let a_token = verified.clone();
    let b_token = verified.clone();
    let (a, b) = tokio::join!(first.claim(&a_token), second.claim(&b_token));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        if let Err(error) = a {
            error
        } else {
            b.unwrap_err()
        },
        AdmissionError::Replay
    );

    let settlement =
        TerminalSpoolInput::settlement(&verified, 40_000_000, now + chrono::Duration::seconds(1));
    let first_record = claims.spool_terminal(settlement.clone()).await.unwrap();
    assert_eq!(
        claims.spool_terminal(settlement).await.unwrap(),
        first_record
    );
    assert_eq!(
        claims
            .spool_terminal(TerminalSpoolInput::release(
                &verified,
                now + chrono::Duration::seconds(1),
            ))
            .await
            .unwrap_err(),
        AdmissionError::TerminalConflict
    );
}
