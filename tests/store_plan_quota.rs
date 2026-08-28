use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::models::{PlanQuota, WindowKind};
use monoize::store_billing::quota::{
    EntitlementGenerationInput, PlanFundingAdmission, PlanFundingInput, QuotaReservationInput,
    QuotaStore, QuotaTerminalState, quota_window,
};
use monoize::store_billing::quota_gate::{
    GateSlot, QuotaEnvironment, QuotaGateState, QuotaGateStore, QuotaManifest,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

async fn setup() -> (DbPool, QuotaStore, QuotaEnvironment) {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    Migrator::up(&*db.write().await, None)
        .await
        .expect("run migrations");
    insert_user(&db, "quota-user").await;

    let environment = QuotaGateStore::new(db.clone())
        .live_environment()
        .await
        .expect("read live SQLite quota environment");
    pass_gate(&db, &environment, "test-app-a").await;
    (db.clone(), QuotaStore::new(db), environment)
}

async fn insert_user(db: &DbPool, id: &str) {
    let group = db
        .read()
        .query_one(db.stmt("SELECT id FROM monoize_groups WHERE is_default = 1", vec![]))
        .await
        .unwrap()
        .unwrap();
    let group_id: String = group.try_get("", "id").unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO users
                (id, username, password_hash, role, created_at, updated_at, enabled,
                 balance_nano_usd, balance_unlimited, group_id)
             VALUES ($1, $2, 'test', 'user', $3, $3, 1, '0', 0, $4)",
            vec![
                id.into(),
                format!("name-{id}").into(),
                "2026-08-28T00:00:00Z".into(),
                group_id.into(),
            ],
        ))
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_products
                (id, kind, name, description, price_currency, price_minor,
                 duration_seconds, group_ids, sort_order, enabled, created_at,
                 updated_at, revision)
             VALUES ('quota-product', 'plan', 'Quota test plan', '', 'CNY', '100',
                     86400, '[]', 0, 0, $1, $1, 1)
             ON CONFLICT (id) DO NOTHING",
            vec!["2026-08-28T00:00:00Z".into()],
        ))
        .await
        .unwrap();
}

async fn pass_gate(db: &DbPool, environment: &QuotaEnvironment, app_version: &str) {
    let manifest = QuotaManifest::passed(
        environment.clone(),
        app_version,
        "drill-result-sha256",
        Utc.with_ymd_and_hms(2026, 8, 28, 0, 0, 0).unwrap(),
        "admin-1",
    )
    .unwrap();
    QuotaGateStore::new(db.clone())
        .import_manifest(GateSlot::Current, manifest)
        .await
        .unwrap();
}

fn quota(id: &str, kind: WindowKind, seconds: i64, amount: &str, order: i32) -> PlanQuota {
    PlanQuota {
        id: id.to_string(),
        window_kind: kind,
        window_seconds: seconds,
        quota_fen_cny: amount.to_string(),
        sort_order: order,
    }
}

fn generation_input(
    expected_generation: Option<i64>,
    source_id: &str,
    starts_at: chrono::DateTime<Utc>,
    ends_at: chrono::DateTime<Utc>,
    quotas: Vec<PlanQuota>,
) -> EntitlementGenerationInput {
    EntitlementGenerationInput {
        expected_generation,
        user_id: "quota-user".to_string(),
        product_id: "quota-product".to_string(),
        product_name: format!("Plan {source_id}"),
        starts_at,
        ends_at,
        rate_numerator: "6".to_string(),
        rate_denominator: "1".to_string(),
        group_ids: vec![],
        quotas,
        source_kind: "order".to_string(),
        source_id: source_id.to_string(),
    }
}

async fn insert_raw_current_entitlement(db: &DbPool, id: &str, starts_at: &str, ends_at: &str) {
    let write = db.write().await;
    write
        .execute_unprepared("PRAGMA ignore_check_constraints = ON")
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "INSERT INTO store_plan_entitlement_generations
             (id, user_id, generation, product_id, product_name, starts_at, ends_at,
              rate_numerator, rate_denominator, group_ids, quota_json, source_kind,
              source_id, created_at)
             VALUES ($1, 'quota-user', 1, 'quota-product', 'Raw plan', $2, $3,
                     '6', '1', '[]',
                     '[{\"id\":\"raw-day\",\"window_kind\":\"day\",\"window_seconds\":86400,\"quota_fen_cny\":\"100\",\"sort_order\":0}]',
                     'order', $1, '2026-08-28T12:00:00Z')",
            vec![id.into(), starts_at.into(), ends_at.into()],
        ))
        .await
        .unwrap();
    write
        .execute_unprepared("PRAGMA ignore_check_constraints = OFF")
        .await
        .unwrap();
    drop(write);
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_plan_entitlement_lifecycle
             (entitlement_id, suspended_at, suspension_reason, revoked_at,
              revocation_reason, updated_at)
             VALUES ($1, NULL, NULL, NULL, NULL, '2026-08-28T12:00:00Z')",
            vec![id.into()],
        ))
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_plan_entitlement_current
             (user_id, entitlement_id, generation, updated_at)
             VALUES ('quota-user', $1, 1, '2026-08-28T12:00:00Z')",
            vec![id.into()],
        ))
        .await
        .unwrap();
}

#[test]
fn rolling_and_shanghai_calendar_windows_have_exact_utc_bounds() {
    let now = Utc.with_ymd_and_hms(2026, 8, 27, 16, 30, 0).unwrap();

    let rolling = quota_window(WindowKind::FiveHours, 18_000, now).unwrap();
    assert_eq!(rolling.start, now);
    assert_eq!(
        rolling.end,
        Utc.with_ymd_and_hms(2026, 8, 27, 21, 30, 0).unwrap()
    );

    let day = quota_window(WindowKind::Day, 86_400, now).unwrap();
    assert_eq!(
        day.start,
        Utc.with_ymd_and_hms(2026, 8, 27, 16, 0, 0).unwrap()
    );
    assert_eq!(
        day.end,
        Utc.with_ymd_and_hms(2026, 8, 28, 16, 0, 0).unwrap()
    );

    let week = quota_window(WindowKind::Week, 604_800, now).unwrap();
    assert_eq!(
        week.start,
        Utc.with_ymd_and_hms(2026, 8, 23, 16, 0, 0).unwrap()
    );
    assert_eq!(
        week.end,
        Utc.with_ymd_and_hms(2026, 8, 30, 16, 0, 0).unwrap()
    );

    let month = quota_window(WindowKind::Month, 2_592_000, now).unwrap();
    assert_eq!(
        month.start,
        Utc.with_ymd_and_hms(2026, 7, 31, 16, 0, 0).unwrap()
    );
    assert_eq!(
        month.end,
        Utc.with_ymd_and_hms(2026, 8, 31, 16, 0, 0).unwrap()
    );
}

#[tokio::test]
async fn replacement_uses_expected_generation_and_old_generation_settles() {
    let (db, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 1, 0, 0).unwrap();
    let first = store
        .replace_entitlement(generation_input(
            None,
            "order-1",
            now,
            now + chrono::Duration::days(30),
            vec![quota("daily-1", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    assert_eq!(first.generation, 1);

    let reservation = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-old-generation".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap();

    let second = store
        .replace_entitlement(generation_input(
            Some(1),
            "order-2",
            now + chrono::Duration::minutes(1),
            now + chrono::Duration::days(31),
            vec![quota("daily-2", WindowKind::Day, 86_400, "200", 0)],
        ))
        .await
        .unwrap();
    assert_eq!(second.generation, 2);

    let error = store
        .replace_entitlement(generation_input(
            Some(1),
            "order-stale",
            now,
            now + chrono::Duration::days(2),
            vec![quota("daily-stale", WindowKind::Day, 86_400, "200", 0)],
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "entitlement_generation_conflict");

    let settled = store
        .settle(
            &reservation.id,
            10_000_000,
            now + chrono::Duration::minutes(2),
        )
        .await
        .unwrap();
    assert_eq!(settled.generation, 1);
    assert_eq!(settled.state, QuotaTerminalState::Settled);

    let current = store
        .current_entitlement("quota-user")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.generation, 2);
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT generation, settled_fen_cny, reserved_fen_cny
             FROM store_quota_buckets ORDER BY generation",
            vec![],
        ))
        .await
        .unwrap();
    assert_eq!(rows[0].try_get::<i64>("", "generation").unwrap(), 1);
    assert_eq!(
        rows[0].try_get::<String>("", "settled_fen_cny").unwrap(),
        "6"
    );
    assert_eq!(
        rows[0].try_get::<String>("", "reserved_fen_cny").unwrap(),
        "0"
    );
}

#[tokio::test]
async fn five_bucket_reservation_is_atomic_exact_and_idempotent() {
    let (db, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 2, 0, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-five",
            now,
            now + chrono::Duration::days(30),
            vec![
                quota("five", WindowKind::FiveHours, 18_000, "100", 0),
                quota("twelve", WindowKind::TwelveHours, 43_200, "100", 1),
                quota("day", WindowKind::Day, 86_400, "100", 2),
                quota("week", WindowKind::Week, 604_800, "100", 3),
                quota("month", WindowKind::Month, 2_592_000, "100", 4),
            ],
        ))
        .await
        .unwrap();

    let input = QuotaReservationInput {
        user_id: "quota-user".to_string(),
        request_id: "request-five".to_string(),
        maximum_nano_usd: 10_000_001,
        pricing_revision: "pricing-v1".to_string(),
        now,
    };
    let reservation = store.reserve(input.clone()).await.unwrap();
    assert_eq!(reservation.reserved_fen_cny, 7);
    assert_eq!(reservation.bucket_count, 5);
    assert_eq!(store.reserve(input).await.unwrap().id, reservation.id);

    let conflict = store
        .reserve(QuotaReservationInput {
            maximum_nano_usd: 10_000_002,
            ..QuotaReservationInput {
                user_id: "quota-user".to_string(),
                request_id: "request-five".to_string(),
                maximum_nano_usd: 0,
                pricing_revision: "pricing-v1".to_string(),
                now,
            }
        })
        .await
        .unwrap_err();
    assert_eq!(conflict.code(), "quota_idempotency_conflict");

    let links = db
        .read()
        .query_all(db.stmt(
            "SELECT reserved_fen_cny FROM store_quota_reservation_buckets
             WHERE reservation_id = $1 ORDER BY bucket_id",
            vec![reservation.id.clone().into()],
        ))
        .await
        .unwrap();
    assert_eq!(links.len(), 5);
    assert!(
        links
            .iter()
            .all(|row| { row.try_get::<String>("", "reserved_fen_cny").unwrap() == "7" })
    );

    let settled = store
        .settle(
            &reservation.id,
            10_000_001,
            now + chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(settled.actual_fen_cny, Some(6));
    assert_eq!(settled.state, QuotaTerminalState::Settled);
    let replay = store
        .settle(
            &reservation.id,
            10_000_001,
            now + chrono::Duration::seconds(2),
        )
        .await
        .unwrap();
    assert_eq!(replay, settled);
    let terminal_conflict = store
        .release(&reservation.id, now + chrono::Duration::seconds(3))
        .await
        .unwrap_err();
    assert_eq!(terminal_conflict.code(), "quota_terminal_conflict");
}

#[tokio::test]
async fn hold_expiry_and_failed_gate_write_no_quota_rows() {
    let (db, store, environment) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 3, 0, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-guard",
            now,
            now + chrono::Duration::hours(1),
            vec![quota("day-guard", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();

    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_balance_holds
                (user_id, active, reason, opened_at, cleared_at)
             VALUES ($1, 1, 'payment_dispute', $2, NULL)",
            vec!["quota-user".into(), now.to_rfc3339().into()],
        ))
        .await
        .unwrap();
    let held = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-held".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap_err();
    assert_eq!(held.code(), "plan_payment_hold");
    assert_quota_write_count(&db, 0).await;

    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_balance_holds SET active = 0, cleared_at = $2 WHERE user_id = $1",
            vec!["quota-user".into(), now.to_rfc3339().into()],
        ))
        .await
        .unwrap();
    let expired = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-expired".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now: now + chrono::Duration::hours(2),
        })
        .await
        .unwrap_err();
    assert_eq!(expired.code(), "plan_entitlement_inactive");
    assert_quota_write_count(&db, 0).await;

    QuotaGateStore::new(db.clone())
        .record_failure(GateSlot::Current, environment, "drill-failure-digest", now)
        .await
        .unwrap();
    let failed_gate = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-gate".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap_err();
    assert_eq!(failed_gate.code(), "quota_gate_unavailable");
    assert_quota_write_count(&db, 0).await;
}

#[tokio::test]
async fn above_reserve_settles_fully_and_blocks_replacement_generation() {
    let (db, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 4, 0, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-violation-1",
            now,
            now + chrono::Duration::days(2),
            vec![quota("day-v1", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    let reservation = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-violation".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap();
    store
        .replace_entitlement(generation_input(
            Some(1),
            "order-violation-2",
            now,
            now + chrono::Duration::days(3),
            vec![quota("day-v2", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();

    let violated = store
        .settle(
            &reservation.id,
            20_000_000,
            now + chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(violated.state, QuotaTerminalState::Violated);
    assert_eq!(violated.generation, 1);
    assert_eq!(violated.actual_fen_cny, Some(12));

    let blocked = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-after-violation".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now: now + chrono::Duration::seconds(2),
        })
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), "plan_quota_violation_blocked");

    let counts = db
        .read()
        .query_one(db.stmt(
            "SELECT
                (SELECT COUNT(*) FROM store_quota_violations) AS violations,
                (SELECT COUNT(*) FROM store_quota_admission_blocks) AS blocks",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(counts.try_get::<i64>("", "violations").unwrap(), 1);
    assert_eq!(counts.try_get::<i64>("", "blocks").unwrap(), 1);
}

#[tokio::test]
async fn gate_uses_current_and_next_manifests_without_binding_app_version() {
    let (db, _, environment) = setup().await;
    let gate = QuotaGateStore::new(db);
    let measured = Utc.with_ymd_and_hms(2026, 8, 28, 5, 0, 0).unwrap();
    let current = QuotaManifest::passed(
        environment.clone(),
        "ordinary-app-v1",
        "drill-current",
        measured,
        "admin-1",
    )
    .unwrap();
    let next = QuotaManifest::passed(
        environment.clone(),
        "ordinary-app-v2",
        "drill-next",
        measured,
        "admin-1",
    )
    .unwrap();
    assert_eq!(
        current.compatibility_fingerprint,
        next.compatibility_fingerprint
    );

    gate.import_manifest(GateSlot::Next, next.clone())
        .await
        .unwrap();
    assert_eq!(
        gate.effective_state(&environment).await.unwrap(),
        QuotaGateState::Passed
    );
    gate.promote_next(&next.compatibility_fingerprint)
        .await
        .unwrap();
    assert_eq!(
        gate.current_manifest()
            .await
            .unwrap()
            .unwrap()
            .application_version,
        "ordinary-app-v2"
    );
}

#[tokio::test]
async fn live_environment_reads_sqlite_and_builtin_manifest_components() {
    let (db, _, environment) = setup().await;
    let sqlite_version: String = db
        .read()
        .query_one(db.stmt("SELECT sqlite_version() AS value", vec![]))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap();

    assert_eq!(environment.compatibility_id, "store-plan-quota-v1");
    assert_eq!(environment.schema_version, 1);
    assert_eq!(environment.sqlite_version, sqlite_version);
    assert_eq!(environment.journal_mode, "memory");
    assert_eq!(environment.busy_timeout_ms, 5_000);
    assert!(environment.page_size > 0);
    assert_eq!(environment.synchronous, "normal");
    assert!(environment.filesystem_id.starts_with("memory:"));
    assert_eq!(
        environment.quota_manifest_digest,
        "e7441428e449942c90445af10dd8441f51d9f5e1dc546952a96601b35c14d487"
    );
}

#[tokio::test]
async fn self_consistent_stale_manifest_disables_plan_features() {
    let (db, _, environment) = setup().await;
    let gate = QuotaGateStore::new(db);
    let mut stale = environment;
    stale.sqlite_version.push_str("-stale");
    let manifest = QuotaManifest::passed(
        stale,
        "ordinary-app-v1",
        "stale-drill",
        Utc.with_ymd_and_hms(2026, 8, 28, 5, 5, 0).unwrap(),
        "admin-1",
    )
    .unwrap();
    gate.import_manifest(GateSlot::Current, manifest)
        .await
        .unwrap();

    assert!(!gate.plan_features_enabled().await.unwrap());
}

#[tokio::test]
async fn promote_next_rejects_a_self_consistent_stale_environment() {
    let (db, _, environment) = setup().await;
    let gate = QuotaGateStore::new(db);
    let mut stale = environment;
    stale.sqlite_version.push_str("-stale");
    let manifest = QuotaManifest::passed(
        stale,
        "ordinary-app-v2",
        "stale-next-drill",
        Utc.with_ymd_and_hms(2026, 8, 28, 5, 10, 0).unwrap(),
        "admin-1",
    )
    .unwrap();
    let stale_fingerprint = manifest.compatibility_fingerprint.clone();
    gate.import_manifest(GateSlot::Next, manifest)
        .await
        .unwrap();

    assert!(matches!(
        gate.promote_next(&stale_fingerprint).await,
        Err(monoize::store_billing::quota_gate::QuotaGateError::FingerprintConflict)
    ));
}

#[tokio::test]
async fn concurrent_reservations_never_exceed_quota_or_partially_write() {
    let (db, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 5, 30, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-concurrent",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-concurrent", WindowKind::Day, 86_400, "6", 0)],
        ))
        .await
        .unwrap();

    let first = store.clone();
    let second = store.clone();
    let (a, b) = tokio::join!(
        first.reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-concurrent-a".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        }),
        second.reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-concurrent-b".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let rejected = if let Err(error) = a {
        error
    } else {
        b.unwrap_err()
    };
    assert_eq!(rejected.code(), "plan_quota_exhausted");

    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT
                (SELECT COUNT(*) FROM store_quota_reservations) AS reservations,
                (SELECT COUNT(*) FROM store_quota_reservation_buckets) AS links,
                (SELECT reserved_fen_cny FROM store_quota_buckets LIMIT 1) AS reserved",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "reservations").unwrap(), 1);
    assert_eq!(row.try_get::<i64>("", "links").unwrap(), 1);
    assert_eq!(row.try_get::<String>("", "reserved").unwrap(), "6");
}

#[tokio::test]
async fn entitlement_source_replay_is_idempotent_and_conflicts_on_changed_snapshot() {
    let (_, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 6, 0, 0).unwrap();
    let input = generation_input(
        None,
        "order-source",
        now,
        now + chrono::Duration::days(1),
        vec![quota("day-source", WindowKind::Day, 86_400, "100", 0)],
    );
    let first = store.replace_entitlement(input.clone()).await.unwrap();
    assert_eq!(store.replace_entitlement(input).await.unwrap(), first);

    let mut changed = generation_input(
        Some(1),
        "order-source",
        now,
        now + chrono::Duration::days(1),
        vec![quota("day-source", WindowKind::Day, 86_400, "101", 0)],
    );
    changed.product_name = "Changed snapshot".to_string();
    assert_eq!(
        store.replace_entitlement(changed).await.unwrap_err().code(),
        "entitlement_source_conflict"
    );
    assert_eq!(
        store
            .current_entitlement("quota-user")
            .await
            .unwrap()
            .unwrap(),
        first
    );
}

#[tokio::test]
async fn release_is_idempotent_and_conflicting_settlement_is_rejected() {
    let (_, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 6, 30, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-release",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-release", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    let reservation = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-release".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap();
    let released = store
        .release(&reservation.id, now + chrono::Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(released.state, QuotaTerminalState::Released);
    assert_eq!(
        store
            .release(&reservation.id, now + chrono::Duration::seconds(2))
            .await
            .unwrap(),
        released
    );
    assert_eq!(
        store
            .settle(&reservation.id, 1, now + chrono::Duration::seconds(3))
            .await
            .unwrap_err()
            .code(),
        "quota_terminal_conflict"
    );
}

#[tokio::test]
async fn sqlite_quota_path_uses_wal_foreign_keys_and_five_second_busy_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("quota.sqlite");
    let db = DbPool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    Migrator::up(&*db.write().await, None).await.unwrap();
    insert_user(&db, "quota-user").await;
    let environment = QuotaGateStore::new(db.clone())
        .live_environment()
        .await
        .unwrap();
    pass_gate(&db, &environment, "test-app").await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 7, 0, 0).unwrap();
    let store = QuotaStore::new(db.clone());
    store
        .replace_entitlement(generation_input(
            None,
            "order-pragmas",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-pragmas", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-pragmas".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap();

    let probe_db = db.clone();
    let observed: Result<(String, i64, i64), sea_orm::DbErr> = db
        .with_immediate_write(move |connection| {
            Box::pin(async move {
                let journal = connection
                    .query_one(probe_db.stmt("PRAGMA journal_mode", vec![]))
                    .await?
                    .unwrap();
                let busy = connection
                    .query_one(probe_db.stmt("PRAGMA busy_timeout", vec![]))
                    .await?
                    .unwrap();
                let foreign = connection
                    .query_one(probe_db.stmt("PRAGMA foreign_keys", vec![]))
                    .await?
                    .unwrap();
                Ok((
                    journal.try_get("", "journal_mode")?,
                    busy.try_get("", "timeout")?,
                    foreign.try_get("", "foreign_keys")?,
                ))
            })
        })
        .await;
    assert_eq!(observed.unwrap(), ("wal".to_string(), 5_000, 1));

    let write = db.write().await;
    let restored = write
        .query_one(db.stmt("PRAGMA busy_timeout", vec![]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.try_get::<i64>("", "timeout").unwrap(), 15_000);
}

#[tokio::test]
async fn funding_admission_uses_balance_only_without_an_applicable_plan() {
    let (_, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 8, 0, 0).unwrap();
    assert_eq!(
        store
            .admit_funding(PlanFundingInput {
                user_id: "quota-user".to_string(),
                request_id: "funding-no-plan".to_string(),
                effective_groups: vec![],
                maximum_nano_usd: None,
                pricing_revision: "pricing-v1".to_string(),
                now,
                replica: false,
            })
            .await
            .unwrap(),
        PlanFundingAdmission::Balance
    );

    let mut group_limited = generation_input(
        None,
        "order-group-limited",
        now,
        now + chrono::Duration::days(1),
        vec![quota("day-funding", WindowKind::Day, 86_400, "100", 0)],
    );
    group_limited.group_ids = vec!["group-plan-only".to_string()];
    store.replace_entitlement(group_limited).await.unwrap();
    assert_eq!(
        store
            .admit_funding(PlanFundingInput {
                user_id: "quota-user".to_string(),
                request_id: "funding-other-group".to_string(),
                effective_groups: vec!["group-other".to_string()],
                maximum_nano_usd: None,
                pricing_revision: "pricing-v1".to_string(),
                now,
                replica: false,
            })
            .await
            .unwrap(),
        PlanFundingAdmission::Balance
    );
}

#[tokio::test]
async fn funding_admission_parses_offset_boundaries_and_fails_closed_on_malformed_time() {
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 12, 0, 0).unwrap();

    let (start_db, start_store, _) = setup().await;
    insert_raw_current_entitlement(
        &start_db,
        "offset-start",
        "2026-08-28T13:00:00+01:00",
        "2026-08-28T14:00:00+01:00",
    )
    .await;
    assert!(matches!(
        start_store
            .admit_funding(PlanFundingInput {
                user_id: "quota-user".to_string(),
                request_id: "offset-start-request".to_string(),
                effective_groups: vec![],
                maximum_nano_usd: Some(10_000_000),
                pricing_revision: "pricing-v1".to_string(),
                now,
                replica: false,
            })
            .await
            .unwrap(),
        PlanFundingAdmission::Plan(_)
    ));

    let (end_db, end_store, _) = setup().await;
    insert_raw_current_entitlement(
        &end_db,
        "offset-end",
        "2026-08-28T12:00:00+01:00",
        "2026-08-28T13:00:00+01:00",
    )
    .await;
    assert_eq!(
        end_store
            .admit_funding(PlanFundingInput {
                user_id: "quota-user".to_string(),
                request_id: "offset-end-request".to_string(),
                effective_groups: vec![],
                maximum_nano_usd: Some(10_000_000),
                pricing_revision: "pricing-v1".to_string(),
                now,
                replica: false,
            })
            .await
            .unwrap(),
        PlanFundingAdmission::Balance
    );

    let (invalid_db, invalid_store, _) = setup().await;
    insert_raw_current_entitlement(
        &invalid_db,
        "invalid-time",
        "not-a-time",
        "2026-08-28T14:00:00+01:00",
    )
    .await;
    assert_eq!(
        invalid_store
            .admit_funding(PlanFundingInput {
                user_id: "quota-user".to_string(),
                request_id: "invalid-time-request".to_string(),
                effective_groups: vec![],
                maximum_nano_usd: Some(10_000_000),
                pricing_revision: "pricing-v1".to_string(),
                now,
                replica: false,
            })
            .await
            .unwrap_err()
            .code(),
        "quota_storage_error"
    );

    let (lifecycle_db, lifecycle_store, _) = setup().await;
    insert_raw_current_entitlement(
        &lifecycle_db,
        "invalid-lifecycle-time",
        "2026-08-28T12:00:00Z",
        "2026-08-28T13:00:00Z",
    )
    .await;
    lifecycle_db
        .write()
        .await
        .execute_unprepared(
            "UPDATE store_plan_entitlement_lifecycle
             SET suspended_at = 'not-a-time', suspension_reason = 'corrupt'",
        )
        .await
        .unwrap();
    assert_eq!(
        lifecycle_store
            .admit_funding(PlanFundingInput {
                user_id: "quota-user".to_string(),
                request_id: "invalid-lifecycle-request".to_string(),
                effective_groups: vec![],
                maximum_nano_usd: Some(10_000_000),
                pricing_revision: "pricing-v1".to_string(),
                now,
                replica: false,
            })
            .await
            .unwrap_err()
            .code(),
        "quota_storage_error"
    );
}

#[tokio::test]
async fn applicable_plan_requires_a_finite_bound_and_never_falls_back() {
    let (db, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 8, 30, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-funding",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-funding", WindowKind::Day, 86_400, "6", 0)],
        ))
        .await
        .unwrap();
    let unbounded = store
        .admit_funding(PlanFundingInput {
            user_id: "quota-user".to_string(),
            request_id: "funding-unbounded".to_string(),
            effective_groups: vec![],
            maximum_nano_usd: None,
            pricing_revision: "pricing-v1".to_string(),
            now,
            replica: false,
        })
        .await
        .unwrap_err();
    assert_eq!(unbounded.code(), "plan_request_unbounded");
    assert_quota_write_count(&db, 0).await;

    let admitted = store
        .admit_funding(PlanFundingInput {
            user_id: "quota-user".to_string(),
            request_id: "funding-bounded".to_string(),
            effective_groups: vec![],
            maximum_nano_usd: Some(10_000_000),
            pricing_revision: "pricing-v1".to_string(),
            now,
            replica: false,
        })
        .await
        .unwrap();
    assert!(matches!(admitted, PlanFundingAdmission::Plan(_)));
    let exhausted = store
        .admit_funding(PlanFundingInput {
            user_id: "quota-user".to_string(),
            request_id: "funding-exhausted".to_string(),
            effective_groups: vec![],
            maximum_nano_usd: Some(10_000_000),
            pricing_revision: "pricing-v1".to_string(),
            now,
            replica: false,
        })
        .await
        .unwrap_err();
    assert_eq!(exhausted.code(), "plan_quota_exhausted");
}

#[tokio::test]
async fn replica_plan_admission_fails_closed_before_local_reservation() {
    let (db, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 9, 0, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-replica",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-replica", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    let error = store
        .admit_funding(PlanFundingInput {
            user_id: "quota-user".to_string(),
            request_id: "funding-replica".to_string(),
            effective_groups: vec![],
            maximum_nano_usd: Some(10_000_000),
            pricing_revision: "pricing-v1".to_string(),
            now,
            replica: true,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), "plan_admission_token_required");
    assert_quota_write_count(&db, 0).await;
}

#[tokio::test]
async fn gate_fingerprint_tamper_blocks_reservation_without_writes() {
    let (db, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 10, 0, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-gate-tamper",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-gate-tamper", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_quota_gates SET compatibility_fingerprint = 'tampered'
             WHERE backend = 'sqlite' AND slot = 'current'",
            vec![],
        ))
        .await
        .unwrap();
    let error = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-gate-tamper".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), "quota_gate_unavailable");
    assert_quota_write_count(&db, 0).await;
}

#[tokio::test]
async fn self_consistent_stale_gate_blocks_reservation_without_writes() {
    let (db, store, environment) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 10, 15, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-gate-stale",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-gate-stale", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    let mut stale = environment;
    stale.quota_manifest_digest.push_str("-stale");
    QuotaGateStore::new(db.clone())
        .import_manifest(
            GateSlot::Current,
            QuotaManifest::passed(
                stale,
                "ordinary-app-v1",
                "stale-admission-drill",
                now,
                "admin-1",
            )
            .unwrap(),
        )
        .await
        .unwrap();

    let error = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-gate-stale".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), "quota_gate_unavailable");
    assert_quota_write_count(&db, 0).await;
}

#[tokio::test]
async fn request_id_replay_cannot_cross_entitlement_generation() {
    let (_, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 10, 30, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-replay-1",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-replay-1", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    let input = QuotaReservationInput {
        user_id: "quota-user".to_string(),
        request_id: "request-cross-generation".to_string(),
        maximum_nano_usd: 10_000_000,
        pricing_revision: "pricing-v1".to_string(),
        now,
    };
    store.reserve(input.clone()).await.unwrap();
    store
        .replace_entitlement(generation_input(
            Some(1),
            "order-replay-2",
            now,
            now + chrono::Duration::days(2),
            vec![quota("day-replay-2", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    assert_eq!(
        store.reserve(input).await.unwrap_err().code(),
        "quota_idempotency_conflict"
    );
}

#[tokio::test]
async fn a_new_violation_reactivates_a_cleared_user_block() {
    let (db, store, _) = setup().await;
    let now = Utc.with_ymd_and_hms(2026, 8, 28, 11, 0, 0).unwrap();
    store
        .replace_entitlement(generation_input(
            None,
            "order-block-1",
            now,
            now + chrono::Duration::days(1),
            vec![quota("day-block-1", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    let first = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-block-1".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap();
    store.settle(&first.id, 20_000_000, now).await.unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_quota_admission_blocks SET cleared_at = $2 WHERE user_id = $1",
            vec!["quota-user".into(), now.to_rfc3339().into()],
        ))
        .await
        .unwrap();
    store
        .replace_entitlement(generation_input(
            Some(1),
            "order-block-2",
            now,
            now + chrono::Duration::days(2),
            vec![quota("day-block-2", WindowKind::Day, 86_400, "100", 0)],
        ))
        .await
        .unwrap();
    let second = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-block-2".to_string(),
            maximum_nano_usd: 10_000_000,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap();
    store.settle(&second.id, 20_000_000, now).await.unwrap();
    let blocked = store
        .reserve(QuotaReservationInput {
            user_id: "quota-user".to_string(),
            request_id: "request-after-second-block".to_string(),
            maximum_nano_usd: 1,
            pricing_revision: "pricing-v1".to_string(),
            now,
        })
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), "plan_quota_violation_blocked");
}

async fn assert_quota_write_count(db: &DbPool, expected: i64) {
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT
                (SELECT COUNT(*) FROM store_quota_reservations) +
                (SELECT COUNT(*) FROM store_quota_buckets) AS value",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "value").unwrap(), expected);
}
