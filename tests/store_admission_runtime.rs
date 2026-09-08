use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration, TimeZone, Utc};
use ed25519_dalek::SigningKey;
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::replica::metering::{BalanceDelta, MeteringBatch, apply_metering_batch};
use monoize::store_billing::admission_runtime::{
    AdmissionDecision, AdmissionRuntimeError, AdmissionService, ConfirmAdmissionInput,
    IssueAdmissionInput, IssuedAdmission, TerminalApplyInput, TerminalApplyResult, terminal_digest,
};
use monoize::store_billing::admission_token::{PlanTerminalWire, TerminalKind};
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::models::{PlanQuota, WindowKind};
use monoize::store_billing::quota::{EntitlementGenerationInput, QuotaError, QuotaStore};
use monoize::store_billing::quota_gate::{GateSlot, QuotaGateStore, QuotaManifest};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

struct Fixture {
    db: DbPool,
    service: AdmissionService,
    now: chrono::DateTime<Utc>,
}

async fn fixture() -> Fixture {
    fixture_with_window(-60, 86_400).await
}

async fn fixture_with_window(starts_offset_seconds: i64, ends_offset_seconds: i64) -> Fixture {
    fixture_with_window_and_quota(starts_offset_seconds, ends_offset_seconds, "1000").await
}

async fn fixture_with_window_and_quota(
    starts_offset_seconds: i64,
    ends_offset_seconds: i64,
    quota_fen_cny: &str,
) -> Fixture {
    let db = DbPool::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&*db.write().await, None).await.unwrap();
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 12, 0, 0).unwrap();
    let group: String = db
        .read()
        .query_one(db.stmt("SELECT id FROM monoize_groups WHERE is_default = 1", vec![]))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "id")
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO users
             (id, username, password_hash, role, created_at, updated_at, enabled,
              balance_nano_usd, balance_unlimited, group_id)
             VALUES ('admission-user', 'admission-user', 'test', 'user', $1, $1, 1,
                     '0', 0, $2)",
            vec![now.to_rfc3339().into(), group.clone().into()],
        ))
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO users
             (id, username, password_hash, role, created_at, updated_at, enabled,
              balance_nano_usd, balance_unlimited, group_id)
             VALUES ('balance-user', 'balance-user', 'test', 'user', $1, $1, 1,
                     '0', 0, $2)",
            vec![now.to_rfc3339().into(), group.into()],
        ))
        .await
        .unwrap();
    db.write()
        .await
        .execute_unprepared(
            "INSERT INTO store_products
             (id, kind, name, description, price_currency, price_minor,
              duration_seconds, group_ids, sort_order, enabled, created_at, updated_at, revision)
             VALUES ('admission-plan', 'plan', 'Admission plan', '', 'CNY', '100',
                     86400, '[]', 0, 0, '2026-08-28T00:00:00Z',
                     '2026-08-28T00:00:00Z', 1)",
        )
        .await
        .unwrap();
    let gate = QuotaGateStore::new(db.clone());
    let environment = gate.live_environment().await.unwrap();
    gate.import_manifest(
        GateSlot::Current,
        QuotaManifest::passed(environment, "test", "drill", now, "admin").unwrap(),
    )
    .await
    .unwrap();
    QuotaStore::new(db.clone())
        .replace_entitlement(EntitlementGenerationInput {
            expected_generation: None,
            user_id: "admission-user".to_string(),
            product_id: "admission-plan".to_string(),
            product_name: "Admission plan".to_string(),
            starts_at: now + Duration::seconds(starts_offset_seconds),
            ends_at: now + Duration::seconds(ends_offset_seconds),
            rate_numerator: "6".to_string(),
            rate_denominator: "1".to_string(),
            group_ids: vec![],
            quotas: vec![PlanQuota {
                id: "admission-day".to_string(),
                window_kind: WindowKind::Day,
                window_seconds: 86_400,
                quota_fen_cny: quota_fen_cny.to_string(),
                sort_order: 0,
            }],
            source_kind: "order".to_string(),
            source_id: "admission-source".to_string(),
        })
        .await
        .unwrap();

    let wrap = Arc::new(
        PaymentKeyRing::new(PaymentKey::new("wrap-active", [41_u8; 32]).unwrap(), vec![]).unwrap(),
    );
    insert_admission_key(&db, &wrap, "admission-key", [19_u8; 32], now).await;
    let service = AdmissionService::new(db.clone(), wrap, "lynshen-primary").unwrap();
    Fixture { db, service, now }
}

async fn insert_admission_key(
    db: &DbPool,
    wrap: &PaymentKeyRing,
    key_id: &str,
    seed: [u8; 32],
    now: chrono::DateTime<Utc>,
) {
    let encrypted = wrap
        .encrypt(&format!("store-admission-key:{key_id}:seed:v1"), &seed)
        .unwrap();
    let public = URL_SAFE_NO_PAD.encode(SigningKey::from_bytes(&seed).verifying_key().as_bytes());
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_admission_keys
             (key_id, public_key_base64, encrypted_private_key_json, state,
              published_at, activated_at, retired_at, last_issued_expires_at,
              verify_until, config_epoch)
             VALUES ($1, $2, $3, 'active', $4, $4, NULL, NULL, NULL, 1)",
            vec![
                key_id.into(),
                public.into(),
                serde_json::to_string(&encrypted).unwrap().into(),
                now.to_rfc3339().into(),
            ],
        ))
        .await
        .unwrap();
}

fn issue_input(now: chrono::DateTime<Utc>) -> IssueAdmissionInput {
    IssueAdmissionInput {
        audience: "replica-a".to_string(),
        user_id: "admission-user".to_string(),
        request_id: "request-a".to_string(),
        effective_groups: vec!["group-z".to_string(), "group-z".to_string()],
        maximum_nano_usd: 10_000_000,
        pricing_revision: "pricing-v1".to_string(),
        issued_at: now,
    }
}

fn expect_plan(decision: AdmissionDecision) -> IssuedAdmission {
    match decision {
        AdmissionDecision::Plan(issued) => issued,
        AdmissionDecision::Balance => panic!("expected plan admission"),
    }
}

fn terminal_wire(input: &TerminalApplyInput) -> PlanTerminalWire {
    PlanTerminalWire {
        version: 1,
        token_id: input.token_id.clone(),
        reservation_id: input.reservation_id.clone(),
        request_id: input.request_id.clone(),
        audience: input.audience.clone(),
        kind: input.kind,
        actual_nano_usd: input.actual_nano_usd.map(|value| value.to_string()),
        canonical_digest: input.canonical_digest.clone(),
        created_at: input.applied_at,
    }
}

async fn confirm_issued(fixture: &Fixture, issued: &IssuedAdmission, request_id: &str) {
    fixture
        .service
        .confirm(ConfirmAdmissionInput {
            audience: "replica-a".to_string(),
            token_id: issued.token_id.clone(),
            reservation_id: issued.reservation_id.clone(),
            request_id: request_id.to_string(),
            confirmed_at: fixture.now + Duration::seconds(1),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn ambiguous_issue_retry_returns_one_reservation_and_identical_token() {
    let fixture = fixture().await;
    let first = expect_plan(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
    );
    let retry = expect_plan(
        fixture
            .service
            .issue(issue_input(fixture.now + Duration::seconds(1)))
            .await
            .unwrap(),
    );
    assert!(!first.duplicate);
    assert!(retry.duplicate);
    assert_eq!(retry.compact_jws, first.compact_jws);
    assert_eq!(retry.token_id, first.token_id);
    assert_eq!(retry.reservation_id, first.reservation_id);
    let counts = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT (SELECT COUNT(*) FROM store_admission_tokens) AS tokens,
                    (SELECT COUNT(*) FROM store_quota_reservations) AS reservations",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counts.try_get::<i64>("", "tokens").unwrap(), 1);
    assert_eq!(counts.try_get::<i64>("", "reservations").unwrap(), 1);
}

#[tokio::test]
async fn concurrent_issue_exact_retry_has_one_token_and_changed_binding_conflicts() {
    let exact = fixture().await;
    let (left, right) = tokio::join!(
        exact.service.issue(issue_input(exact.now)),
        exact
            .service
            .issue(issue_input(exact.now + Duration::seconds(1))),
    );
    let left = expect_plan(left.unwrap());
    let right = expect_plan(right.unwrap());
    assert_eq!(left.compact_jws, right.compact_jws);
    assert_ne!(left.duplicate, right.duplicate);

    let changed = fixture().await;
    let first = issue_input(changed.now);
    let mut second = issue_input(changed.now);
    second.maximum_nano_usd += 1;
    let (left, right) = tokio::join!(changed.service.issue(first), changed.service.issue(second),);
    let results = [left, right];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .next()
            .unwrap()
            .code(),
        "admission_issue_conflict"
    );
}

#[tokio::test]
async fn equal_external_request_ids_are_independent_across_audiences() {
    let fixture = fixture().await;
    let first = expect_plan(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
    );
    let mut second_input = issue_input(fixture.now);
    second_input.audience = "replica-b".to_string();
    let second = expect_plan(fixture.service.issue(second_input).await.unwrap());
    assert_ne!(first.token_id, second.token_id);
    assert_ne!(first.reservation_id, second.reservation_id);
    let requests = fixture
        .db
        .read()
        .query_all(fixture.db.stmt(
            "SELECT request_id FROM store_quota_reservations ORDER BY request_id",
            vec![],
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "request_id").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|value| value.starts_with("admission:")));
    assert_ne!(requests[0], requests[1]);
}

#[tokio::test]
async fn no_applicable_plan_returns_balance_without_loading_a_signing_key() {
    let fixture = fixture().await;
    fixture
        .db
        .write()
        .await
        .execute_unprepared("DELETE FROM store_admission_keys")
        .await
        .unwrap();
    let mut input = issue_input(fixture.now);
    input.user_id = "balance-user".to_string();
    input.request_id = "balance-request".to_string();
    assert_eq!(
        fixture.service.issue(input).await.unwrap(),
        AdmissionDecision::Balance
    );
    let counts = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT (SELECT COUNT(*) FROM store_admission_tokens) AS tokens,
                    (SELECT COUNT(*) FROM store_quota_reservations) AS reservations",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counts.try_get::<i64>("", "tokens").unwrap(), 0);
    assert_eq!(counts.try_get::<i64>("", "reservations").unwrap(), 0);
}

#[tokio::test]
async fn inactive_entitlements_return_balance_without_loading_a_signing_key() {
    let future = fixture_with_window(60, 3_600).await;
    assert_balance_without_keys(future, "future-request").await;

    let expired = fixture_with_window(-3_600, -60).await;
    assert_balance_without_keys(expired, "expired-request").await;

    let suspended = fixture().await;
    suspended
        .db
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_plan_entitlement_lifecycle
             SET suspended_at = '2026-08-28T11:59:00Z', suspension_reason = 'test'",
        )
        .await
        .unwrap();
    assert_balance_without_keys(suspended, "suspended-request").await;

    let revoked = fixture().await;
    revoked
        .db
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_plan_entitlement_lifecycle
             SET revoked_at = '2026-08-28T11:59:00Z', revocation_reason = 'test'",
        )
        .await
        .unwrap();
    assert_balance_without_keys(revoked, "revoked-request").await;
}

#[tokio::test]
async fn entitlement_activity_uses_parsed_instants_not_rfc3339_text_order() {
    let fixture = fixture().await;
    fixture
        .db
        .write()
        .await
        .execute_unprepared(
            r#"INSERT INTO store_plan_entitlement_generations
             (id, user_id, generation, product_id, product_name, starts_at, ends_at,
              rate_numerator, rate_denominator, group_ids, quota_json, source_kind,
              source_id, created_at)
             VALUES ('offset-entitlement', 'admission-user', 2, 'admission-plan',
                     'Offset plan', '2026-08-28T13:00:00+01:00',
                     '2026-08-28T14:00:00+01:00', '6', '1', '[]',
                     '[{"id":"offset-day","window_kind":"day","window_seconds":86400,"quota_fen_cny":"1000","sort_order":0}]',
                     'order', 'offset-source', '2026-08-28T12:00:00Z');
             INSERT INTO store_plan_entitlement_lifecycle
             (entitlement_id, suspended_at, suspension_reason, revoked_at,
              revocation_reason, updated_at)
             VALUES ('offset-entitlement', NULL, NULL, NULL, NULL,
                     '2026-08-28T12:00:00Z');
             UPDATE store_plan_entitlement_current
             SET entitlement_id = 'offset-entitlement', generation = 2,
                 updated_at = '2026-08-28T12:00:00Z'
             WHERE user_id = 'admission-user';"#,
        )
        .await
        .unwrap();

    assert!(matches!(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
        AdmissionDecision::Plan(_)
    ));
}

#[tokio::test]
async fn malformed_persisted_entitlement_times_fail_with_quota_storage_error() {
    for column in ["starts_at", "ends_at", "suspended_at", "revoked_at"] {
        let fixture = fixture().await;
        if matches!(column, "starts_at" | "ends_at") {
            let starts_at = if column == "starts_at" {
                "not-rfc3339"
            } else {
                "2026-08-28T11:59:00Z"
            };
            let ends_at = if column == "ends_at" {
                "not-rfc3339"
            } else {
                "2026-08-29T12:00:00Z"
            };
            fixture
                .db
                .write()
                .await
                .execute_unprepared(&format!(
                    r#"PRAGMA ignore_check_constraints = ON;
                       INSERT INTO store_plan_entitlement_generations
                       (id, user_id, generation, product_id, product_name, starts_at, ends_at,
                        rate_numerator, rate_denominator, group_ids, quota_json, source_kind,
                        source_id, created_at)
                       VALUES ('malformed-entitlement', 'admission-user', 2, 'admission-plan',
                               'Malformed plan', '{starts_at}', '{ends_at}', '6', '1', '[]',
                               '[{{"id":"malformed-day","window_kind":"day","window_seconds":86400,"quota_fen_cny":"1000","sort_order":0}}]',
                               'order', 'malformed-source', '2026-08-28T12:00:00Z');
                       INSERT INTO store_plan_entitlement_lifecycle
                       (entitlement_id, suspended_at, suspension_reason, revoked_at,
                        revocation_reason, updated_at)
                       VALUES ('malformed-entitlement', NULL, NULL, NULL, NULL,
                               '2026-08-28T12:00:00Z');
                       UPDATE store_plan_entitlement_current
                       SET entitlement_id = 'malformed-entitlement', generation = 2,
                           updated_at = '2026-08-28T12:00:00Z'
                       WHERE user_id = 'admission-user';
                       PRAGMA ignore_check_constraints = OFF;"#
                ))
                .await
                .unwrap();
        } else {
            fixture
                .db
                .write()
                .await
                .execute_unprepared(&format!(
                    "PRAGMA ignore_check_constraints = ON;
                     UPDATE store_plan_entitlement_lifecycle SET {column} = 'not-rfc3339';
                     PRAGMA ignore_check_constraints = OFF;"
                ))
                .await
                .unwrap();
        }

        assert_eq!(
            fixture
                .service
                .issue(issue_input(fixture.now))
                .await
                .unwrap_err()
                .code(),
            "quota_storage_error",
            "malformed entitlement {column} must not select Balance"
        );
        let counts = fixture
            .db
            .read()
            .query_one(fixture.db.stmt(
                "SELECT (SELECT COUNT(*) FROM store_admission_tokens) AS tokens,
                        (SELECT COUNT(*) FROM store_quota_reservations) AS reservations",
                vec![],
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(counts.try_get::<i64>("", "tokens").unwrap(), 0);
        assert_eq!(counts.try_get::<i64>("", "reservations").unwrap(), 0);
    }
}

async fn assert_balance_without_keys(fixture: Fixture, request_id: &str) {
    fixture
        .db
        .write()
        .await
        .execute_unprepared("DELETE FROM store_admission_keys")
        .await
        .unwrap();
    let mut input = issue_input(fixture.now);
    input.request_id = request_id.to_string();
    assert_eq!(
        fixture.service.issue(input).await.unwrap(),
        AdmissionDecision::Balance
    );
    let count = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT (SELECT COUNT(*) FROM store_admission_tokens) +
                    (SELECT COUNT(*) FROM store_quota_reservations) AS value",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn issue_rejects_tampered_key_missing_active_and_missing_wrap_key() {
    let tampered = fixture().await;
    tampered
        .db
        .write()
        .await
        .execute_unprepared("UPDATE store_admission_keys SET public_key_base64 = 'tampered'")
        .await
        .unwrap();
    assert_eq!(
        tampered
            .service
            .issue(issue_input(tampered.now))
            .await
            .unwrap_err()
            .code(),
        "admission_key_invalid"
    );

    let invalid_secret = fixture().await;
    invalid_secret
        .db
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_admission_keys SET encrypted_private_key_json = '{invalid'",
        )
        .await
        .unwrap();
    assert_eq!(
        invalid_secret
            .service
            .issue(issue_input(invalid_secret.now))
            .await
            .unwrap_err()
            .code(),
        "admission_key_invalid"
    );

    let invalid_time = fixture().await;
    invalid_time
        .db
        .write()
        .await
        .execute_unprepared("UPDATE store_admission_keys SET last_issued_expires_at = 'not-a-time'")
        .await
        .unwrap();
    assert_eq!(
        invalid_time
            .service
            .issue(issue_input(invalid_time.now))
            .await
            .unwrap_err()
            .code(),
        "admission_key_invalid"
    );

    let missing = fixture().await;
    missing
        .db
        .write()
        .await
        .execute_unprepared("DELETE FROM store_admission_keys")
        .await
        .unwrap();
    assert_eq!(
        missing
            .service
            .issue(issue_input(missing.now))
            .await
            .unwrap_err()
            .code(),
        "admission_active_key_missing"
    );

    let no_wrap = fixture().await;
    let wrong = Arc::new(
        PaymentKeyRing::new(PaymentKey::new("other-wrap", [42_u8; 32]).unwrap(), vec![]).unwrap(),
    );
    let service = AdmissionService::new(no_wrap.db, wrong, "lynshen-primary").unwrap();
    assert_eq!(
        service
            .issue(issue_input(no_wrap.now))
            .await
            .unwrap_err()
            .code(),
        "admission_wrap_key_missing"
    );
}

#[tokio::test]
async fn issue_preserves_quota_business_error_codes() {
    let exhausted = fixture_with_window_and_quota(-60, 86_400, "5").await;
    assert_eq!(
        exhausted
            .service
            .issue(issue_input(exhausted.now))
            .await
            .unwrap_err()
            .code(),
        "plan_quota_exhausted"
    );

    let held = fixture().await;
    held.db
        .write()
        .await
        .execute(held.db.stmt(
            "INSERT INTO store_balance_holds
             (user_id, active, reason, opened_at, cleared_at)
             VALUES ('admission-user', 1, 'payment_dispute', $1, NULL)",
            vec![held.now.to_rfc3339().into()],
        ))
        .await
        .unwrap();
    assert_eq!(
        held.service
            .issue(issue_input(held.now))
            .await
            .unwrap_err()
            .code(),
        "plan_payment_hold"
    );

    let gate = fixture().await;
    gate.db
        .write()
        .await
        .execute_unprepared("DELETE FROM store_quota_gates")
        .await
        .unwrap();
    assert_eq!(
        gate.service
            .issue(issue_input(gate.now))
            .await
            .unwrap_err()
            .code(),
        "quota_gate_unavailable"
    );

    let blocked = fixture().await;
    let issued = expect_plan(
        blocked
            .service
            .issue(issue_input(blocked.now))
            .await
            .unwrap(),
    );
    confirm_issued(&blocked, &issued, "request-a").await;
    let mut terminal = TerminalApplyInput {
        token_id: issued.token_id,
        reservation_id: issued.reservation_id,
        request_id: "request-a".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Settlement,
        actual_nano_usd: Some(20_000_000),
        canonical_digest: String::new(),
        applied_at: blocked.now + Duration::seconds(1),
    };
    terminal.canonical_digest = terminal_digest(&terminal).unwrap();
    blocked.service.apply_terminal(terminal).await.unwrap();
    let mut next = issue_input(blocked.now + Duration::seconds(2));
    next.request_id = "request-after-block".to_string();
    assert_eq!(
        blocked.service.issue(next).await.unwrap_err().code(),
        "plan_quota_violation_blocked"
    );
}

#[test]
fn admission_runtime_preserves_every_quota_error_code() {
    for code in [
        "plan_quota_exhausted",
        "plan_request_unbounded",
        "plan_payment_hold",
        "plan_quota_violation_blocked",
        "quota_gate_unavailable",
    ] {
        assert_eq!(
            AdmissionRuntimeError::from(QuotaError::Code(code)).code(),
            code
        );
    }
    assert_eq!(
        AdmissionRuntimeError::from(QuotaError::Storage("offline".to_string())).code(),
        "quota_storage_error"
    );
}

#[tokio::test]
async fn admission_service_requires_fixed_issuer_and_wrap_key_only_for_plan_issue() {
    let fixture = fixture().await;
    assert_eq!(
        AdmissionService::new(fixture.db.clone(), None, "other-primary")
            .err()
            .unwrap()
            .code(),
        "admission_issuer_invalid"
    );
    let service = AdmissionService::new(fixture.db, None, "lynshen-primary").unwrap();
    let mut balance = issue_input(fixture.now);
    balance.user_id = "balance-user".to_string();
    balance.request_id = "balance-without-wrap".to_string();
    assert_eq!(
        service.issue(balance).await.unwrap(),
        AdmissionDecision::Balance
    );
    assert_eq!(
        service
            .issue(issue_input(fixture.now))
            .await
            .unwrap_err()
            .code(),
        "admission_wrap_key_missing"
    );
}

#[tokio::test]
async fn provisional_admission_confirmation_is_exact_idempotent_and_bound() {
    let current = fixture().await;
    let issued = expect_plan(
        current
            .service
            .issue(issue_input(current.now))
            .await
            .unwrap(),
    );
    let stored = current
        .db
        .read()
        .query_one(current.db.stmt(
            "SELECT confirmed_at FROM store_admission_tokens WHERE token_id = $1",
            vec![issued.token_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored
            .try_get::<Option<String>>("", "confirmed_at")
            .unwrap()
            .is_none()
    );

    let input = ConfirmAdmissionInput {
        audience: "replica-a".to_string(),
        token_id: issued.token_id.clone(),
        reservation_id: issued.reservation_id.clone(),
        request_id: "request-a".to_string(),
        confirmed_at: current.now + Duration::seconds(1),
    };
    let first = current.service.confirm(input.clone()).await.unwrap();
    assert!(!first.duplicate);
    let duplicate = current.service.confirm(input.clone()).await.unwrap();
    assert!(duplicate.duplicate);
    let mut late_duplicate = input.clone();
    late_duplicate.confirmed_at = current.now + Duration::minutes(10);
    assert!(
        current
            .service
            .confirm(late_duplicate)
            .await
            .unwrap()
            .duplicate
    );

    let mut changed = input.clone();
    changed.request_id = "changed".to_string();
    assert_eq!(
        current.service.confirm(changed).await.unwrap_err().code(),
        "admission_binding_mismatch"
    );
    let late = fixture().await;
    let late_issue = expect_plan(late.service.issue(issue_input(late.now)).await.unwrap());
    assert_eq!(
        late.service
            .confirm(ConfirmAdmissionInput {
                audience: "replica-a".to_string(),
                token_id: late_issue.token_id,
                reservation_id: late_issue.reservation_id,
                request_id: "request-a".to_string(),
                confirmed_at: late.now + Duration::seconds(35),
            })
            .await
            .unwrap_err()
            .code(),
        "admission_confirmation_expired"
    );
}

#[tokio::test]
async fn settlement_requires_confirmation_and_reaper_releases_only_expired_provisional_tokens() {
    let fixture = fixture().await;
    let provisional = expect_plan(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
    );
    let mut settlement = TerminalApplyInput {
        token_id: provisional.token_id.clone(),
        reservation_id: provisional.reservation_id.clone(),
        request_id: "request-a".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Settlement,
        actual_nano_usd: Some(1),
        canonical_digest: String::new(),
        applied_at: fixture.now + Duration::seconds(1),
    };
    settlement.canonical_digest = terminal_digest(&settlement).unwrap();
    assert_eq!(
        fixture
            .service
            .apply_terminal(settlement)
            .await
            .unwrap_err()
            .code(),
        "admission_terminal_conflict"
    );

    let mut confirmed_input = issue_input(fixture.now);
    confirmed_input.request_id = "confirmed-request".to_string();
    let confirmed = expect_plan(fixture.service.issue(confirmed_input).await.unwrap());
    fixture
        .service
        .confirm(ConfirmAdmissionInput {
            audience: "replica-a".to_string(),
            token_id: confirmed.token_id.clone(),
            reservation_id: confirmed.reservation_id,
            request_id: "confirmed-request".to_string(),
            confirmed_at: fixture.now + Duration::seconds(1),
        })
        .await
        .unwrap();

    let service_without_wrap =
        AdmissionService::new(fixture.db.clone(), None, "lynshen-primary").unwrap();
    assert_eq!(
        service_without_wrap
            .recover_unconfirmed(fixture.now + Duration::seconds(35), 100)
            .await
            .unwrap(),
        1
    );
    let rows = fixture
        .db
        .read()
        .query_all(fixture.db.stmt(
            "SELECT token_id, terminal_kind FROM store_admission_terminal_receipts ORDER BY token_id",
            vec![],
        ))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].try_get::<String>("", "token_id").unwrap(),
        provisional.token_id
    );
    assert_eq!(
        rows[0].try_get::<String>("", "terminal_kind").unwrap(),
        "release"
    );
}

#[tokio::test]
async fn unconfirmed_reaper_limits_and_orders_candidates_in_the_database() {
    let fixture = fixture_with_window_and_quota(-60, 86_400, "100000").await;
    let mut token_ids = Vec::new();
    for index in 0..101 {
        let mut input = issue_input(fixture.now);
        input.request_id = format!("bounded-reaper-{index:03}");
        token_ids.push(expect_plan(fixture.service.issue(input).await.unwrap()).token_id);
    }
    token_ids.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));

    assert_eq!(
        fixture
            .service
            .recover_unconfirmed(fixture.now + Duration::seconds(35), 1000)
            .await
            .unwrap(),
        100
    );
    let recovered = fixture
        .db
        .read()
        .query_all(fixture.db.stmt(
            "SELECT token_id FROM store_admission_terminal_receipts ORDER BY token_id",
            vec![],
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "token_id").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(recovered, token_ids[..100]);
    assert_eq!(
        fixture
            .service
            .recover_unconfirmed(fixture.now + Duration::seconds(35), 100)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn unconfirmed_reaper_rejects_malformed_or_inconsistent_expiry_mirrors() {
    for (expires_at, expires_at_unix) in [("not-rfc3339", 0_i64), ("2026-08-28T12:00:30Z", 1_i64)] {
        let fixture = fixture().await;
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap();
        fixture
            .db
            .write()
            .await
            .execute(fixture.db.stmt(
                "UPDATE store_admission_tokens
                 SET expires_at = $1, expires_at_unix = $2",
                vec![expires_at.into(), expires_at_unix.into()],
            ))
            .await
            .unwrap();
        assert_eq!(
            fixture
                .service
                .recover_unconfirmed(fixture.now + Duration::seconds(35), 100)
                .await
                .unwrap_err()
                .code(),
            "admission_storage_error"
        );
        let receipts = fixture
            .db
            .read()
            .query_one(fixture.db.stmt(
                "SELECT COUNT(*) AS value FROM store_admission_terminal_receipts",
                vec![],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "value")
            .unwrap();
        assert_eq!(receipts, 0);
    }
}

#[tokio::test]
async fn public_keyset_contains_verifier_data_only() {
    let fixture = fixture().await;
    let keys = fixture.service.public_keyset(fixture.now).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key_id, "admission-key");
    assert_eq!(keys[0].state, "active");
    assert!(!keys[0].public_key_base64.is_empty());
}

#[tokio::test]
async fn key_retention_uses_parsed_instants_and_keyset_validates_public_keys() {
    let fixture = fixture().await;
    let retired_seed = [23_u8; 32];
    let retired_public = URL_SAFE_NO_PAD.encode(
        SigningKey::from_bytes(&retired_seed)
            .verifying_key()
            .as_bytes(),
    );
    fixture
        .db
        .write()
        .await
        .execute(fixture.db.stmt(
            "INSERT INTO store_admission_keys
             (key_id, public_key_base64, encrypted_private_key_json, state,
              published_at, activated_at, retired_at, last_issued_expires_at,
              verify_until, config_epoch)
             VALUES ('expired-offset-key', $1, '{invalid', 'retired',
                     '2026-08-28T10:00:00Z', '2026-08-28T10:00:00Z',
                     '2026-08-28T11:00:00Z', NULL,
                     '2026-08-28T13:00:00+01:00', 1)",
            vec![retired_public.into()],
        ))
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
        AdmissionDecision::Plan(_)
    ));
    let keys = fixture.service.public_keyset(fixture.now).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key_id, "admission-key");

    fixture
        .db
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_admission_keys SET public_key_base64 = 'bad='
             WHERE state = 'active'",
        )
        .await
        .unwrap();
    assert_eq!(
        fixture
            .service
            .public_keyset(fixture.now)
            .await
            .unwrap_err()
            .code(),
        "admission_key_invalid"
    );
}

#[tokio::test]
async fn public_keyset_rejects_an_expired_retired_key_with_a_malformed_public_key() {
    let fixture = fixture().await;
    fixture
        .db
        .write()
        .await
        .execute_unprepared(
            "INSERT INTO store_admission_keys
             (key_id, public_key_base64, encrypted_private_key_json, state,
              published_at, activated_at, retired_at, last_issued_expires_at,
              verify_until, config_epoch)
             VALUES ('expired-malformed-key', 'bad=', '{invalid', 'retired',
                     '2026-08-28T10:00:00Z', '2026-08-28T10:00:00Z',
                     '2026-08-28T11:00:00Z', NULL,
                     '2026-08-28T12:00:00Z', 1)",
        )
        .await
        .unwrap();

    assert_eq!(
        fixture
            .service
            .public_keyset(fixture.now)
            .await
            .unwrap_err()
            .code(),
        "admission_key_invalid"
    );
}

#[tokio::test]
async fn active_key_update_must_affect_one_row_and_rolls_back_issue() {
    let fixture = fixture().await;
    fixture
        .db
        .write()
        .await
        .execute_unprepared(
            "CREATE TRIGGER retire_key_before_expiry_update
             AFTER INSERT ON store_admission_tokens
             BEGIN
                 UPDATE store_admission_keys
                 SET state = 'retired', retired_at = '2026-08-28T12:00:00Z',
                     verify_until = '2026-08-28T12:05:00Z'
                 WHERE key_id = NEW.key_id;
             END",
        )
        .await
        .unwrap();
    assert_eq!(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap_err()
            .code(),
        "admission_key_invalid"
    );
    let count = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT (SELECT COUNT(*) FROM store_admission_tokens) +
                    (SELECT COUNT(*) FROM store_quota_reservations) AS value",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn terminal_apply_is_exactly_once_and_conflicts_on_changed_replay() {
    let fixture = fixture().await;
    let issued = expect_plan(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
    );
    confirm_issued(&fixture, &issued, "request-a").await;
    let mut terminal = TerminalApplyInput {
        token_id: issued.token_id,
        reservation_id: issued.reservation_id,
        request_id: "request-a".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Settlement,
        actual_nano_usd: Some(5_000_000),
        canonical_digest: String::new(),
        applied_at: fixture.now + Duration::seconds(2),
    };
    terminal.canonical_digest = terminal_digest(&terminal).unwrap();
    assert_eq!(
        fixture
            .service
            .apply_terminal(terminal.clone())
            .await
            .unwrap(),
        TerminalApplyResult::Applied
    );
    assert_eq!(
        fixture
            .service
            .apply_terminal(terminal.clone())
            .await
            .unwrap(),
        TerminalApplyResult::Duplicate
    );
    terminal.actual_nano_usd = Some(6_000_000);
    terminal.canonical_digest = terminal_digest(&terminal).unwrap();
    assert_eq!(
        fixture
            .service
            .apply_terminal(terminal.clone())
            .await
            .unwrap_err()
            .code(),
        "admission_terminal_conflict"
    );

    for field in ["reservation", "request", "audience"] {
        let mut changed = terminal.clone();
        changed.actual_nano_usd = Some(5_000_000);
        match field {
            "reservation" => changed.reservation_id = "changed-reservation".to_string(),
            "request" => changed.request_id = "changed-request".to_string(),
            "audience" => changed.audience = "replica-b".to_string(),
            _ => unreachable!(),
        }
        changed.canonical_digest = terminal_digest(&changed).unwrap();
        assert_eq!(
            fixture
                .service
                .apply_terminal(changed)
                .await
                .unwrap_err()
                .code(),
            "admission_terminal_conflict"
        );
    }
}

#[tokio::test]
async fn concurrent_terminal_replays_are_serialized_by_token() {
    let exact = fixture().await;
    let issued = expect_plan(exact.service.issue(issue_input(exact.now)).await.unwrap());
    confirm_issued(&exact, &issued, "request-a").await;
    let mut terminal = TerminalApplyInput {
        token_id: issued.token_id,
        reservation_id: issued.reservation_id,
        request_id: "request-a".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Settlement,
        actual_nano_usd: Some(5_000_000),
        canonical_digest: String::new(),
        applied_at: exact.now + Duration::seconds(2),
    };
    terminal.canonical_digest = terminal_digest(&terminal).unwrap();
    let (left, right) = tokio::join!(
        exact.service.apply_terminal(terminal.clone()),
        exact.service.apply_terminal(terminal.clone()),
    );
    let mut results = [left.unwrap(), right.unwrap()];
    results.sort_by_key(|value| match value {
        TerminalApplyResult::Applied => 0,
        TerminalApplyResult::Duplicate => 1,
    });
    assert_eq!(
        results,
        [TerminalApplyResult::Applied, TerminalApplyResult::Duplicate]
    );

    let changed = fixture().await;
    let issued = expect_plan(
        changed
            .service
            .issue(issue_input(changed.now))
            .await
            .unwrap(),
    );
    confirm_issued(&changed, &issued, "request-a").await;
    let mut first = TerminalApplyInput {
        token_id: issued.token_id,
        reservation_id: issued.reservation_id,
        request_id: "request-a".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Settlement,
        actual_nano_usd: Some(5_000_000),
        canonical_digest: String::new(),
        applied_at: changed.now + Duration::seconds(2),
    };
    first.canonical_digest = terminal_digest(&first).unwrap();
    let mut second = first.clone();
    second.actual_nano_usd = Some(6_000_000);
    second.canonical_digest = terminal_digest(&second).unwrap();
    let (left, right) = tokio::join!(
        changed.service.apply_terminal(first),
        changed.service.apply_terminal(second),
    );
    let results = [left, right];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .next()
            .unwrap()
            .code(),
        "admission_terminal_conflict"
    );
}

#[tokio::test]
async fn receipt_insert_failure_rolls_back_quota_terminal_mutation() {
    let fixture = fixture().await;
    let issued = expect_plan(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
    );
    fixture
        .db
        .write()
        .await
        .execute_unprepared(
            "CREATE TRIGGER fail_admission_receipt BEFORE INSERT ON store_admission_terminal_receipts
             BEGIN SELECT RAISE(ABORT, 'receipt failure'); END",
        )
        .await
        .unwrap();
    let mut terminal = TerminalApplyInput {
        token_id: issued.token_id,
        reservation_id: issued.reservation_id.clone(),
        request_id: "request-a".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Release,
        actual_nano_usd: None,
        canonical_digest: String::new(),
        applied_at: fixture.now + Duration::seconds(2),
    };
    terminal.canonical_digest = terminal_digest(&terminal).unwrap();
    assert!(fixture.service.apply_terminal(terminal).await.is_err());
    let reservation = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT state FROM store_quota_reservations WHERE id = $1",
            vec![issued.reservation_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reservation.try_get::<String>("", "state").unwrap(),
        "reserved"
    );
}

#[tokio::test]
async fn metering_batch_applies_plan_terminals_in_order_and_replays_as_duplicates() {
    let fixture = fixture().await;
    let first_issued = expect_plan(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
    );
    let mut second_issue = issue_input(fixture.now + Duration::seconds(1));
    second_issue.request_id = "request-b".to_string();
    let second_issued = expect_plan(fixture.service.issue(second_issue).await.unwrap());

    let mut first = TerminalApplyInput {
        token_id: first_issued.token_id,
        reservation_id: first_issued.reservation_id,
        request_id: "request-a".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Release,
        actual_nano_usd: None,
        canonical_digest: String::new(),
        applied_at: fixture.now + Duration::seconds(2),
    };
    first.canonical_digest = terminal_digest(&first).unwrap();
    let mut second = TerminalApplyInput {
        token_id: second_issued.token_id,
        reservation_id: second_issued.reservation_id,
        request_id: "request-b".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Release,
        actual_nano_usd: None,
        canonical_digest: String::new(),
        applied_at: fixture.now + Duration::seconds(3),
    };
    second.canonical_digest = terminal_digest(&second).unwrap();
    let batch = MeteringBatch {
        plan_terminals: vec![terminal_wire(&second), terminal_wire(&first)],
        ..Default::default()
    };

    let first_ack = apply_metering_batch(&fixture.db, &batch).await.unwrap();
    assert_eq!(first_ack.plan_terminal_acks.len(), 2);
    assert_eq!(first_ack.plan_terminal_acks[0].token_id, second.token_id);
    assert_eq!(first_ack.plan_terminal_acks[1].token_id, first.token_id);
    assert_eq!(
        first_ack.plan_terminal_acks[0].result,
        monoize::store_billing::admission_token::TerminalAcknowledgementResult::Applied
    );

    let replay_ack = apply_metering_batch(&fixture.db, &batch).await.unwrap();
    assert!(replay_ack.plan_terminal_acks.iter().all(|ack| {
        ack.result
            == monoize::store_billing::admission_token::TerminalAcknowledgementResult::Duplicate
    }));
}

#[tokio::test]
async fn later_balance_delta_failure_rolls_back_plan_terminal_receipt_and_quota() {
    let fixture = fixture().await;
    let issued = expect_plan(
        fixture
            .service
            .issue(issue_input(fixture.now))
            .await
            .unwrap(),
    );
    let mut terminal = TerminalApplyInput {
        token_id: issued.token_id.clone(),
        reservation_id: issued.reservation_id.clone(),
        request_id: "request-a".to_string(),
        audience: "replica-a".to_string(),
        kind: TerminalKind::Release,
        actual_nano_usd: None,
        canonical_digest: String::new(),
        applied_at: fixture.now + Duration::seconds(2),
    };
    terminal.canonical_digest = terminal_digest(&terminal).unwrap();
    fixture
        .db
        .write()
        .await
        .execute_unprepared(
            "CREATE TRIGGER reject_delta_after_plan_terminal
             BEFORE INSERT ON billing_ledger
             WHEN EXISTS (SELECT 1 FROM store_admission_terminal_receipts)
             BEGIN SELECT RAISE(ABORT, 'forced delta failure'); END",
        )
        .await
        .unwrap();
    let batch = MeteringBatch {
        plan_terminals: vec![terminal_wire(&terminal)],
        balance_deltas: vec![BalanceDelta {
            delta_id: "rollback-delta".to_string(),
            kind: "request_charge".to_string(),
            user_id: "admission-user".to_string(),
            api_key_id: None,
            amount_nano_usd: "1".to_string(),
            meta_json: serde_json::json!({}),
            created_at: fixture.now.to_rfc3339(),
        }],
        ..Default::default()
    };

    assert!(apply_metering_batch(&fixture.db, &batch).await.is_err());
    let state = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT
                 (SELECT COUNT(*) FROM store_admission_terminal_receipts) AS receipts,
                 (SELECT state FROM store_quota_reservations WHERE id = $1) AS reservation_state",
            vec![issued.reservation_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.try_get::<i64>("", "receipts").unwrap(), 0);
    assert_eq!(
        state.try_get::<String>("", "reservation_state").unwrap(),
        "reserved"
    );
}
