use chrono::{Duration, TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::models::StorePrivacyRetention;
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentAttemptState, PaymentOrderError,
    PaymentOrderStore,
};
use monoize::store_billing::retention::{
    CreateStoreLegalHoldInput, CreateStoreRetentionContainmentInput, RetentionRunActor,
    StoreRetention, StoreRetentionDataClass, StoreRetentionError, StoreRetentionRunState,
    RETENTION_BATCH_SIZE, retention_checkout_paused,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;
use serde_json::json;

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn instant() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 28, 12, 0, 0).unwrap()
}

fn retention_policy(financial_days: i64, grant_hours: i64) -> StorePrivacyRetention {
    StorePrivacyRetention {
        raw_callback_days: 30,
        network_metadata_days: 90,
        financial_records_days: financial_days,
        redemption_audit_days: 730,
        expired_reauth_grant_hours: grant_hours,
    }
}

fn retention_json(financial_days: i64, grant_hours: i64) -> String {
    serde_json::to_string(&retention_policy(financial_days, grant_hours)).unwrap()
}

async fn setup() -> DbPool {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    Migrator::up(&*db.write().await, None)
        .await
        .expect("run migrations");
    let write = db.write().await;
    write
        .execute_unprepared(
            "INSERT INTO users
                (id, username, password_hash, role, created_at, updated_at, enabled,
                 balance_nano_usd, balance_unlimited, group_id)
             SELECT 'retention-user', 'retention-user', 'test', 'user',
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1, '0', 0, id
             FROM monoize_groups WHERE is_default = 1 LIMIT 1;
             INSERT INTO users
                (id, username, password_hash, role, created_at, updated_at, enabled,
                 balance_nano_usd, balance_unlimited, group_id)
             SELECT 'retention-admin', 'retention-admin', 'test', 'admin',
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1, '0', 0, id
             FROM monoize_groups WHERE is_default = 1 LIMIT 1;
             INSERT INTO users
                (id, username, password_hash, role, created_at, updated_at, enabled,
                 balance_nano_usd, balance_unlimited, group_id)
             SELECT 'retention-requester', 'retention-requester', 'test', 'admin',
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1, '0', 0, id
             FROM monoize_groups WHERE is_default = 1 LIMIT 1;
             INSERT INTO store_products
                (id, kind, name, description, price_currency, price_minor,
                 duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
             VALUES
                ('retention-product', 'balance', 'Retention', '', 'CNY', '1000',
                 NULL, '[]', 0, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
             INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
             VALUES ('retention-product', '1000', '0');",
        )
        .await
        .expect("seed users and product");
    drop(write);
    db
}

async fn insert_privacy(db: &DbPool, version: &str, retention_json: &str) {
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ($1, $2, 'CN', '[\"CN\"]', $3, 'contract', 'retention-admin', $4,
                     '2026-01-01T00:00:00.000000Z', '2099-01-01T00:00:00.000000Z', 1)",
            vec![
                format!("privacy-{version}").into(),
                version.into(),
                retention_json.into(),
                DIGEST.into(),
            ],
        ))
        .await
        .expect("insert privacy record");
}

async fn insert_provider_event(
    db: &DbPool,
    id: &str,
    received_at: &str,
    with_raw: bool,
    with_network: bool,
) {
    let digest = format!("{DIGEST}-{id}");
    let raw_version: Option<i32> = with_raw.then_some(1);
    let raw_key: Option<String> = with_raw.then(|| "key-1".to_string());
    let raw_nonce: Option<String> = with_raw.then(|| "bm9uY2U=".to_string());
    let raw_cipher: Option<String> = with_raw.then(|| "Y2lwaGVy".to_string());
    let source_ip: Option<String> = with_network.then(|| "203.0.113.10".to_string());
    let user_agent: Option<String> = with_network.then(|| "retention-agent".to_string());
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_provider_events
                (id, credential_version_id, provider_event_id, event_kind, body_digest,
                 parsed_json, verification_result, raw_format_version, raw_key_id,
                 raw_nonce_base64, raw_ciphertext_base64, source_ip, user_agent,
                 projection_state, state_revision, received_at)
             VALUES ($1, 'cred-1', $2, 'payment.succeeded', $3, '{}', 'verified',
                     $4, $5, $6, $7, $8, $9, 'applied', 0, $10)",
            vec![
                id.into(),
                format!("provider-{id}").into(),
                digest.into(),
                raw_version.into(),
                raw_key.into(),
                raw_nonce.into(),
                raw_cipher.into(),
                source_ip.into(),
                user_agent.into(),
                received_at.into(),
            ],
        ))
        .await
        .expect("insert provider event");
}

async fn insert_closed_order(db: &DbPool, id: &str, created_at: &str) {
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_orders
                (id, order_number, user_id, product_id, product_kind, payment_state,
                 fulfillment_state, dispute_state, payment_hold, payment_channel_id,
                 payment_currency, payment_minor, cny_per_usd, rate_numerator,
                 rate_denominator, rate_source_updated_at, quote_json, contract_version,
                 state_revision, expires_at, created_at, updated_at)
             VALUES ($1, $2, 'retention-user', 'retention-product', 'balance', 'closed',
                     'pending', 'none', 0, 'store-channel-stripe', 'CNY', '1000', '6.0000',
                     '6', '1', '2026-01-01T00:00:00Z', '{}', 2, 0,
                     '2026-01-01T01:00:00Z', $3, $3)",
            vec![
                id.into(),
                format!("ORD-{id}").into(),
                created_at.into(),
            ],
        ))
        .await
        .expect("insert closed order");
}

async fn insert_reauth_grant(db: &DbPool, id: &str, expires_at: &str) {
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_reauth_grants
                (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
             VALUES ($1, 'retention-admin', $2, $3, 'credential_update',
                     '2026-01-01T00:00:00.000000Z', $4)",
            vec![
                id.into(),
                format!("{:0<64}", format!("session-{id}")).chars().take(64).collect::<String>().into(),
                format!("{:0<64}", format!("token-{id}")).chars().take(64).collect::<String>().into(),
                expires_at.into(),
            ],
        ))
        .await
        .expect("insert reauth grant");
}

async fn insert_access_audit(db: &DbPool, id: &str, action: &str, created_at: &str) {
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_access_audits
                (id, actor_id, actor_role, action, scope_json, reason, result, created_at)
             VALUES ($1, 'retention-admin', 'admin', $2, '{}', 'fixture', 'succeeded', $3)",
            vec![id.into(), action.into(), created_at.into()],
        ))
        .await
        .expect("insert access audit");
}

fn actor(reason: &str) -> RetentionRunActor {
    RetentionRunActor {
        actor_id: "retention-admin".to_string(),
        actor_role: "admin".to_string(),
        reason: reason.to_string(),
    }
}

async fn event_raw_present(db: &DbPool, id: &str) -> bool {
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT raw_key_id, source_ip FROM store_provider_events WHERE id = $1",
            vec![id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<Option<String>>("", "raw_key_id")
        .unwrap()
        .is_some()
}

async fn event_network_present(db: &DbPool, id: &str) -> bool {
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT source_ip FROM store_provider_events WHERE id = $1",
            vec![id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<Option<String>>("", "source_ip")
        .unwrap()
        .is_some()
}

async fn count_where(db: &DbPool, sql: &str, id: &str) -> i64 {
    db.read()
        .query_one(db.stmt(sql, vec![id.into()]))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap()
}

async fn count_rows(db: &DbPool, sql: &str) -> i64 {
    db.read()
        .query_one(db.stmt(sql, vec![]))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "value")
        .unwrap()
}

// Test-only channel governance fixtures that let create_order/create_attempt succeed
// against the in-memory database; production channel enablement is untouched.
async fn seed_checkout_governance(db: &DbPool) {
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_payment_channels SET enabled = 1 WHERE id = 'store-channel-stripe';
             INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES
                ('retention-credential', 'store-channel-stripe', 'stripe', 1, 'key-1',
                 'bm9uY2U=', 'Y2lwaGVydGV4dA==',
                 '1111111111111111111111111111111111111111111111111111111111111111',
                 'active', '2026-08-27T00:00:00Z');
             INSERT INTO store_payment_compliance
                (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
             VALUES
                ('retention-compliance', 'store-channel-stripe', '2026-08-28',
                 'retention-admin', '127.0.0.1', '2026-08-27T00:00:00Z');
             INSERT INTO store_merchant_capabilities
                (id, channel_id, capability, state, environment, merchant_account_digest,
                 provider_product, evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES
                ('retention-cap-payment-query', 'store-channel-stripe', 'payment_query',
                 'supported', 'sandbox',
                 '1111111111111111111111111111111111111111111111111111111111111111',
                 'checkout',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'retention-admin', '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('retention-cap-refund', 'store-channel-stripe', 'refund',
                 'supported', 'sandbox',
                 '1111111111111111111111111111111111111111111111111111111111111111',
                 'checkout',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'retention-admin', '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('retention-cap-refund-query', 'store-channel-stripe', 'refund_query',
                 'supported', 'sandbox',
                 '1111111111111111111111111111111111111111111111111111111111111111',
                 'checkout',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'retention-admin', '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('retention-cap-settlement', 'store-channel-stripe', 'settlement_report',
                 'supported', 'sandbox',
                 '1111111111111111111111111111111111111111111111111111111111111111',
                 'checkout',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'retention-admin', '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z');
             INSERT INTO store_channel_readiness_profiles
                (channel_id, active_credential_digest, privacy_record_id,
                 callback_verification_passed, supported_currencies_json, amount_limits_json,
                 checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                 availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES ('store-channel-stripe',
                     '1111111111111111111111111111111111111111111111111111111111111111',
                     'privacy-v1', 1,
                     '[\"CNY\",\"USD\"]',
                     '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                     '[\"redirect\"]',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'retention-admin', '2026-08-27T00:00:00Z', '2099-01-01T00:00:00Z')",
        )
        .await
        .expect("seed checkout governance fixtures");
}

#[tokio::test]
async fn missing_privacy_policy_fails_and_records_audit() {
    let db = setup().await;
    let retention = StoreRetention::new(db.clone(), "owner-a");

    let run = retention
        .run_at(instant(), RetentionRunActor::scheduled())
        .await
        .expect("run");

    assert_eq!(run.state, StoreRetentionRunState::Failed);
    assert_eq!(run.policy_version, "unavailable");
    assert_eq!(
        run.error_category.as_deref(),
        Some("privacy_policy_unavailable")
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_access_audits
             WHERE action = 'retention_run' AND result = 'failed' AND actor_id = $1",
            "_monoize_retention_job",
        )
        .await,
        1
    );
}

#[tokio::test]
async fn run_clears_expired_callback_and_network_fields_idempotently() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 24)).await;
    insert_provider_event(&db, "evt-raw", "2026-07-01T00:00:00.000000Z", true, true).await;
    insert_provider_event(
        &db,
        "evt-network",
        "2026-05-01T00:00:00.000000Z",
        true,
        true,
    )
    .await;
    insert_provider_event(&db, "evt-fresh", "2026-08-20T00:00:00.000000Z", true, true).await;

    let retention = StoreRetention::new(db.clone(), "owner-a");
    let run = retention
        .run_at(instant(), actor("manual-retention"))
        .await
        .expect("run");

    assert_eq!(run.state, StoreRetentionRunState::Succeeded);
    assert_eq!(run.counts.raw_callback_bodies, 2);
    assert_eq!(run.counts.network_metadata, 1);
    assert!(!event_raw_present(&db, "evt-raw").await);
    assert!(event_network_present(&db, "evt-raw").await);
    assert!(!event_raw_present(&db, "evt-network").await);
    assert!(!event_network_present(&db, "evt-network").await);
    assert!(event_raw_present(&db, "evt-fresh").await);
    assert!(event_network_present(&db, "evt-fresh").await);

    let second = retention
        .run_at(instant() + Duration::seconds(1), actor("manual-retention-2"))
        .await
        .expect("second run");
    assert_eq!(second.state, StoreRetentionRunState::Succeeded);
    assert_eq!(second.counts.raw_callback_bodies, 0);
    assert_eq!(second.counts.network_metadata, 0);
}

#[tokio::test]
async fn legal_hold_skips_held_records_and_writes_create_audit() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(1, 1)).await;
    insert_provider_event(&db, "evt-held", "2026-01-01T00:00:00.000000Z", true, true).await;
    insert_provider_event(&db, "evt-free", "2026-01-01T00:00:00.000000Z", true, true).await;
    insert_closed_order(&db, "order-held", "2026-01-01T00:00:00.000000Z").await;
    insert_closed_order(&db, "order-free", "2026-01-01T00:00:00.000000Z").await;
    insert_reauth_grant(&db, "grant-held", "2026-08-28T10:00:00.000000Z").await;
    insert_reauth_grant(&db, "grant-free", "2026-08-28T10:00:00.000000Z").await;

    let retention = StoreRetention::new(db.clone(), "owner-a");
    retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::RawCallbackBodies,
                identifiers: vec!["evt-held".to_string()],
                reason: "litigation hold".to_string(),
                requesting_authority: "court".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "privacy".to_string(),
                expires_at: instant() + Duration::days(30),
                extends_hold_id: None,
            },
            "retention-admin",
            instant(),
        )
        .await
        .expect("raw hold");
    retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::FinancialRecords,
                identifiers: vec!["order-held".to_string(), "evt-held".to_string()],
                reason: "financial hold".to_string(),
                requesting_authority: "legal".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "legal".to_string(),
                expires_at: instant() + Duration::days(30),
                extends_hold_id: None,
            },
            "retention-admin",
            instant() + Duration::seconds(1),
        )
        .await
        .expect("financial hold");
    retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::ExpiredReauthGrants,
                identifiers: vec!["grant-held".to_string()],
                reason: "grant hold".to_string(),
                requesting_authority: "security".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "privacy".to_string(),
                expires_at: instant() + Duration::days(30),
                extends_hold_id: None,
            },
            "retention-admin",
            instant() + Duration::seconds(2),
        )
        .await
        .expect("grant hold");

    let run = retention
        .run_at(instant() + Duration::seconds(3), actor("held-run"))
        .await
        .expect("run");
    assert_eq!(run.state, StoreRetentionRunState::Succeeded);
    assert_eq!(run.counts.raw_callback_bodies, 1);
    assert!(run.counts.financial_records >= 2);
    assert_eq!(run.counts.expired_reauth_grants, 1);
    assert!(event_raw_present(&db, "evt-held").await);
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events WHERE id = $1",
            "evt-free",
        )
        .await,
        0
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_orders WHERE id = $1",
            "order-held",
        )
        .await,
        1
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_orders WHERE id = $1",
            "order-free",
        )
        .await,
        0
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_reauth_grants WHERE id = $1",
            "grant-held",
        )
        .await,
        1
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_reauth_grants WHERE id = $1",
            "grant-free",
        )
        .await,
        0
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_access_audits
             WHERE action = 'legal_hold_create' AND actor_id = $1",
            "retention-admin",
        )
        .await,
        3
    );
}

#[tokio::test]
async fn deletes_expired_grants_and_old_redemption_audits() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 2)).await;
    insert_reauth_grant(&db, "grant-old", "2026-08-28T09:00:00.000000Z").await;
    insert_reauth_grant(&db, "grant-new", "2026-08-28T11:00:00.000000Z").await;
    insert_access_audit(
        &db,
        "audit-old",
        "redemption_reveal",
        "2024-01-01T00:00:00.000000Z",
    )
    .await;
    insert_access_audit(
        &db,
        "audit-new",
        "redemption_export",
        "2026-08-01T00:00:00.000000Z",
    )
    .await;

    let run = StoreRetention::new(db.clone(), "owner-a")
        .run_at(instant(), actor("cleanup"))
        .await
        .expect("run");

    assert_eq!(run.state, StoreRetentionRunState::Succeeded);
    assert_eq!(run.counts.expired_reauth_grants, 1);
    assert_eq!(run.counts.redemption_audits, 1);
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_reauth_grants WHERE id = $1",
            "grant-old",
        )
        .await,
        0
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_reauth_grants WHERE id = $1",
            "grant-new",
        )
        .await,
        1
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_access_audits WHERE id = $1",
            "audit-old",
        )
        .await,
        0
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_access_audits WHERE id = $1",
            "audit-new",
        )
        .await,
        1
    );
}

#[tokio::test]
async fn three_failures_pause_checkout_and_containment_clears_pause_only() {
    let db = setup().await;
    let retention = StoreRetention::new(db.clone(), "owner-a");

    for index in 0..3 {
        let run = retention
            .run_at(
                instant() + Duration::seconds(index),
                RetentionRunActor::scheduled(),
            )
            .await
            .expect("failed run");
        assert_eq!(run.state, StoreRetentionRunState::Failed);
    }

    let status = retention.status().await.expect("status");
    assert!(status.checkout_paused);
    assert_eq!(status.consecutive_failures, 3);
    assert!(status.active_alert.is_some());
    assert!(
        retention_checkout_paused(&db, &*db.read())
            .await
            .expect("pause read")
    );

    insert_privacy(&db, "v-success", &retention_json(2557, 24)).await;
    let success = retention
        .run_at(instant() + Duration::seconds(10), actor("after-pause"))
        .await
        .expect("success while paused");
    assert_eq!(success.state, StoreRetentionRunState::Succeeded);
    let status_after_success = retention.status().await.expect("status");
    assert!(status_after_success.checkout_paused);
    assert_eq!(status_after_success.consecutive_failures, 0);
    assert!(status_after_success.active_alert.is_some());

    let containment = retention
        .contain(
            CreateStoreRetentionContainmentInput {
                reason: "reviewed deletion backlog".to_string(),
                evidence_digest: DIGEST.to_string(),
            },
            "retention-admin",
            instant() + Duration::seconds(20),
        )
        .await
        .expect("contain");
    let status_after = retention.status().await.expect("status");
    assert!(!status_after.checkout_paused);
    assert_eq!(status_after.consecutive_failures, 0);
    assert!(status_after.active_alert.is_none());
    assert_eq!(
        status_after.latest_containment_id.as_deref(),
        Some(containment.id.as_str())
    );
}

#[tokio::test]
async fn paused_checkout_rejects_new_orders_before_insert() {
    let db = setup().await;
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_retention_state
             SET checkout_paused = 1, consecutive_failures = 3,
                 updated_at = '2026-08-28T12:00:00.000000Z'
             WHERE singleton_id = 1",
        )
        .await
        .expect("pause checkout");

    let rate = ExchangeRateSnapshot {
        base: "USD".to_string(),
        quote: "CNY".to_string(),
        cny_per_usd: "6.0000".to_string(),
        source_updated_at: instant(),
        refreshed_at: instant(),
    };
    let error = PaymentOrderStore::new(db.clone())
        .create_order(
            "retention-user",
            CreatePaymentOrderInput {
                idempotency_key: "retention-paused-order".to_string(),
                product_id: "retention-product".to_string(),
                payment_channel_id: "store-channel-stripe".to_string(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &rate,
        )
        .await
        .expect_err("paused checkout");
    assert!(matches!(error, PaymentOrderError::RetentionPaused));
}

#[tokio::test]
async fn self_approval_and_extension_rules_are_enforced() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 24)).await;
    let retention = StoreRetention::new(db.clone(), "owner-a");

    let self_approval = retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::NetworkMetadata,
                identifiers: vec!["evt-1".to_string()],
                reason: "self approval".to_string(),
                requesting_authority: "court".to_string(),
                requester_id: "retention-admin".to_string(),
                approver_role: "privacy".to_string(),
                expires_at: instant() + Duration::days(7),
                extends_hold_id: None,
            },
            "retention-admin",
            instant(),
        )
        .await
        .expect_err("self approval");
    assert_eq!(self_approval, StoreRetentionError::InvalidInput);

    let hold = retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::NetworkMetadata,
                identifiers: vec!["evt-1".to_string()],
                reason: "initial hold".to_string(),
                requesting_authority: "court".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "privacy".to_string(),
                expires_at: instant() + Duration::days(7),
                extends_hold_id: None,
            },
            "retention-admin",
            instant(),
        )
        .await
        .expect("create hold");

    let earlier = retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::NetworkMetadata,
                identifiers: vec!["evt-1".to_string()],
                reason: "earlier extension".to_string(),
                requesting_authority: "court".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "privacy".to_string(),
                expires_at: instant() + Duration::days(3),
                extends_hold_id: Some(hold.id.clone()),
            },
            "retention-admin",
            instant() + Duration::seconds(1),
        )
        .await
        .expect_err("earlier expiry");
    assert_eq!(earlier, StoreRetentionError::InvalidInput);

    let extension = retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::NetworkMetadata,
                identifiers: vec!["evt-1".to_string()],
                reason: "extend hold".to_string(),
                requesting_authority: "court".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "legal".to_string(),
                expires_at: instant() + Duration::days(14),
                extends_hold_id: Some(hold.id.clone()),
            },
            "retention-admin",
            instant() + Duration::seconds(2),
        )
        .await
        .expect("extension");
    assert_eq!(extension.extends_hold_id.as_deref(), Some(hold.id.as_str()));
    assert!(extension.active);
}

#[tokio::test]
async fn competing_owner_interrupts_active_claim() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 24)).await;
    db.write()
        .await
        .execute_unprepared(
            "INSERT INTO store_retention_runs
                (id, policy_version, counts_json, oldest_remaining_at, state,
                 error_category, started_at, completed_at, worker_owner_id)
             VALUES
                ('run-stale', 'v1',
                 '{\"raw_callback_bodies\":0,\"network_metadata\":0,\"financial_records\":0,\"redemption_audits\":0,\"expired_reauth_grants\":0}',
                 NULL, 'running', NULL, '2026-08-28T11:00:00.000000Z', NULL, 'owner-old');
             UPDATE store_retention_state
             SET run_in_progress = 1, current_run_id = 'run-stale',
                 current_worker_owner_id = 'owner-old',
                 updated_at = '2026-08-28T11:00:00.000000Z'
             WHERE singleton_id = 1;",
        )
        .await
        .expect("seed active claim");

    let run = StoreRetention::new(db.clone(), "owner-new")
        .run_at(instant(), actor("takeover"))
        .await
        .expect("takeover run");
    assert_eq!(run.state, StoreRetentionRunState::Succeeded);
    assert_eq!(run.worker_owner_id, "owner-new");

    let interrupted = db
        .read()
        .query_one(db.stmt(
            "SELECT state, error_category FROM store_retention_runs WHERE id = $1",
            vec!["run-stale".into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        interrupted.try_get::<String>("", "state").unwrap(),
        "failed"
    );
    assert_eq!(
        interrupted
            .try_get::<String>("", "error_category")
            .unwrap(),
        "interrupted"
    );
}

#[tokio::test]
async fn overview_lists_runs_holds_and_containments() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 24)).await;
    let retention = StoreRetention::new(db.clone(), "owner-a");
    retention
        .run_at(instant(), actor("overview-run"))
        .await
        .expect("run");
    retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::RedemptionAudits,
                identifiers: vec!["audit-1".to_string()],
                reason: "overview hold".to_string(),
                requesting_authority: "privacy".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "privacy".to_string(),
                expires_at: instant() + Duration::days(1),
                extends_hold_id: None,
            },
            "retention-admin",
            instant() + Duration::seconds(1),
        )
        .await
        .expect("hold");

    let overview = retention.overview().await.expect("overview");
    assert!(!overview.runs.is_empty());
    assert_eq!(overview.holds.len(), 1);
    assert!(overview.holds[0].active);
    assert_eq!(overview.status.consecutive_failures, 0);
}

#[tokio::test]
async fn invalid_privacy_retention_document_fails_run() {
    let db = setup().await;
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('privacy-bad', 'bad', 'CN', '[\"CN\"]', $1, 'contract',
                     'retention-admin', $2, '2026-01-01T00:00:00.000000Z',
                     '2099-01-01T00:00:00.000000Z', 1)",
            vec![
                json!({
                    "raw_callback_days": 10,
                    "network_metadata_days": 90,
                    "financial_records_days": 2557,
                    "redemption_audit_days": 730,
                    "expired_reauth_grant_hours": 24
                })
                .to_string()
                .into(),
                DIGEST.into(),
            ],
        ))
        .await
        .expect("insert invalid privacy");

    let run = StoreRetention::new(db.clone(), "owner-a")
        .run_at(instant(), actor("invalid-policy"))
        .await
        .expect("run");
    assert_eq!(run.state, StoreRetentionRunState::Failed);
    assert_eq!(run.error_category.as_deref(), Some("privacy_policy_invalid"));
}

#[tokio::test]
async fn bounded_deletion_caps_each_class_at_batch_size() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 1)).await;
    let total = (RETENTION_BATCH_SIZE as usize) + 1;
    for index in 0..total {
        insert_reauth_grant(
            &db,
            &format!("grant-batch-{index:04}"),
            "2026-08-28T10:00:00.000000Z",
        )
        .await;
    }

    let retention = StoreRetention::new(db.clone(), "owner-a");
    let first = retention
        .run_at(instant(), actor("bounded-1"))
        .await
        .expect("first bounded run");
    assert_eq!(first.state, StoreRetentionRunState::Succeeded);
    assert_eq!(
        first.counts.expired_reauth_grants,
        RETENTION_BATCH_SIZE as u64
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_reauth_grants WHERE id LIKE $1",
            "grant-batch-%",
        )
        .await,
        1
    );

    let second = retention
        .run_at(instant() + Duration::seconds(1), actor("bounded-2"))
        .await
        .expect("second bounded run");
    assert_eq!(second.state, StoreRetentionRunState::Succeeded);
    assert_eq!(second.counts.expired_reauth_grants, 1);
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_reauth_grants WHERE id LIKE $1",
            "grant-batch-%",
        )
        .await,
        0
    );
}

#[tokio::test]
async fn legal_hold_expiry_allows_deletion_and_does_not_restore() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 1)).await;
    insert_provider_event(&db, "evt-hold", "2026-01-01T00:00:00.000000Z", true, true).await;
    insert_provider_event(&db, "evt-gone", "2026-01-01T00:00:00.000000Z", true, true).await;

    let retention = StoreRetention::new(db.clone(), "owner-a");
    retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::RawCallbackBodies,
                identifiers: vec!["evt-hold".to_string()],
                reason: "temporary hold".to_string(),
                requesting_authority: "court".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "privacy".to_string(),
                expires_at: instant() + Duration::hours(1),
                extends_hold_id: None,
            },
            "retention-admin",
            instant(),
        )
        .await
        .expect("create hold");

    let during_hold = retention
        .run_at(instant() + Duration::minutes(30), actor("during-hold"))
        .await
        .expect("during hold");
    assert_eq!(during_hold.state, StoreRetentionRunState::Succeeded);
    assert_eq!(during_hold.counts.raw_callback_bodies, 1);
    assert!(event_raw_present(&db, "evt-hold").await);
    assert!(!event_raw_present(&db, "evt-gone").await);

    let after_expiry = retention
        .run_at(instant() + Duration::hours(2), actor("after-expiry"))
        .await
        .expect("after expiry");
    assert_eq!(after_expiry.state, StoreRetentionRunState::Succeeded);
    assert_eq!(after_expiry.counts.raw_callback_bodies, 1);
    assert!(!event_raw_present(&db, "evt-hold").await);

    retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::RawCallbackBodies,
                identifiers: vec!["evt-gone".to_string()],
                reason: "hold after deletion".to_string(),
                requesting_authority: "court".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "legal".to_string(),
                expires_at: instant() + Duration::days(30),
                extends_hold_id: None,
            },
            "retention-admin",
            instant() + Duration::hours(3),
        )
        .await
        .expect("hold deleted id");

    let restore_probe = retention
        .run_at(instant() + Duration::hours(4), actor("no-restore"))
        .await
        .expect("no restore run");
    assert_eq!(restore_probe.state, StoreRetentionRunState::Succeeded);
    assert_eq!(restore_probe.counts.raw_callback_bodies, 0);
    assert!(!event_raw_present(&db, "evt-gone").await);
    assert!(!event_raw_present(&db, "evt-hold").await);
}

#[tokio::test]
async fn financial_deletion_orders_by_global_timestamp_across_tables() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 1)).await;
    insert_provider_event(
        &db,
        "evt-oldest",
        "2019-01-01T00:00:00.000000Z",
        true,
        true,
    )
    .await;
    let total = RETENTION_BATCH_SIZE as usize;
    for index in 0..total {
        insert_closed_order(
            &db,
            &format!("order-batch-{index:04}"),
            "2019-06-01T00:00:00.000000Z",
        )
        .await;
    }

    let retention = StoreRetention::new(db.clone(), "owner-a");
    let run = retention
        .run_at(instant(), actor("financial-ordering"))
        .await
        .expect("run");

    assert_eq!(run.state, StoreRetentionRunState::Succeeded);
    assert_eq!(run.counts.financial_records, RETENTION_BATCH_SIZE as u64);
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events WHERE id = $1",
            "evt-oldest",
        )
        .await,
        0
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_orders WHERE id LIKE $1",
            "order-batch-%",
        )
        .await,
        1
    );
}

#[tokio::test]
async fn provider_event_not_deleted_before_network_metadata_floor() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(30, 1)).await;
    insert_provider_event(
        &db,
        "evt-young",
        "2026-07-15T00:00:00.000000Z",
        true,
        true,
    )
    .await;

    let retention = StoreRetention::new(db.clone(), "owner-a");
    let run = retention
        .run_at(instant(), actor("network-floor"))
        .await
        .expect("run");

    assert_eq!(run.state, StoreRetentionRunState::Succeeded);
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events WHERE id = $1",
            "evt-young",
        )
        .await,
        1
    );
    assert!(event_network_present(&db, "evt-young").await);
}

#[tokio::test]
async fn invalid_hold_extension_rolls_back_without_residue() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 24)).await;
    let retention = StoreRetention::new(db.clone(), "owner-a");

    let hold = retention
        .create_legal_hold(
            CreateStoreLegalHoldInput {
                data_class: StoreRetentionDataClass::NetworkMetadata,
                identifiers: vec!["evt-1".to_string()],
                reason: "initial hold".to_string(),
                requesting_authority: "court".to_string(),
                requester_id: "retention-requester".to_string(),
                approver_role: "privacy".to_string(),
                expires_at: instant() + Duration::days(7),
                extends_hold_id: None,
            },
            "retention-admin",
            instant(),
        )
        .await
        .expect("create initial hold");

    let invalid_extensions = [
        (
            "earlier expiry",
            vec!["evt-1".to_string()],
            instant() + Duration::days(3),
            Some(hold.id.clone()),
        ),
        (
            "identifier mismatch",
            vec!["evt-2".to_string()],
            instant() + Duration::days(14),
            Some(hold.id.clone()),
        ),
        (
            "missing referenced hold",
            vec!["evt-1".to_string()],
            instant() + Duration::days(14),
            Some("missing-hold".to_string()),
        ),
    ];
    for (label, identifiers, expires_at, extends_hold_id) in invalid_extensions {
        let error = retention
            .create_legal_hold(
                CreateStoreLegalHoldInput {
                    data_class: StoreRetentionDataClass::NetworkMetadata,
                    identifiers,
                    reason: label.to_string(),
                    requesting_authority: "court".to_string(),
                    requester_id: "retention-requester".to_string(),
                    approver_role: "privacy".to_string(),
                    expires_at,
                    extends_hold_id,
                },
                "retention-admin",
                instant() + Duration::seconds(1),
            )
            .await
            .expect_err(label);
        assert_eq!(error, StoreRetentionError::InvalidInput);
    }

    assert_eq!(
        count_rows(&db, "SELECT COUNT(*) AS value FROM store_legal_holds").await,
        1
    );
    assert_eq!(
        count_rows(&db, "SELECT COUNT(*) AS value FROM store_legal_hold_items").await,
        1
    );
    assert_eq!(
        count_rows(&db, "SELECT COUNT(*) AS value FROM store_legal_hold_approvals").await,
        1
    );
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_access_audits WHERE action = $1",
            "legal_hold_create",
        )
        .await,
        1
    );
    let holds = retention
        .list_legal_holds(instant(), 100)
        .await
        .expect("list holds");
    assert_eq!(holds.len(), 1);
    assert_eq!(holds[0].id, hold.id);
}

#[tokio::test]
async fn failure_after_containment_creates_new_alert_and_repauses() {
    let db = setup().await;
    let retention = StoreRetention::new(db.clone(), "owner-a");

    for index in 0..3 {
        let run = retention
            .run_at(
                instant() + Duration::seconds(index),
                RetentionRunActor::scheduled(),
            )
            .await
            .expect("failed run");
        assert_eq!(run.state, StoreRetentionRunState::Failed);
    }
    let paused = retention.status().await.expect("status");
    assert!(paused.checkout_paused);
    assert_eq!(paused.consecutive_failures, 3);
    let first_alert = paused.active_alert.expect("first alert");

    retention
        .contain(
            CreateStoreRetentionContainmentInput {
                reason: "contained before repair".to_string(),
                evidence_digest: DIGEST.to_string(),
            },
            "retention-admin",
            instant() + Duration::seconds(10),
        )
        .await
        .expect("contain");
    let contained = retention.status().await.expect("status");
    assert!(!contained.checkout_paused);
    assert_eq!(contained.consecutive_failures, 3);
    assert!(contained.active_alert.is_none());

    let fourth = retention
        .run_at(
            instant() + Duration::seconds(20),
            RetentionRunActor::scheduled(),
        )
        .await
        .expect("fourth failed run");
    assert_eq!(fourth.state, StoreRetentionRunState::Failed);

    let repaused = retention.status().await.expect("status");
    assert!(repaused.checkout_paused);
    assert_eq!(repaused.consecutive_failures, 4);
    let second_alert = repaused.active_alert.expect("second alert");
    assert_ne!(second_alert.id, first_alert.id);
    assert_eq!(second_alert.consecutive_failures, 4);
    assert_eq!(second_alert.run_id, fourth.id);
    assert!(second_alert.contained_at.is_none());
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_retention_alerts
             WHERE id = $1 AND contained_at IS NOT NULL",
            &first_alert.id,
        )
        .await,
        1
    );
    assert!(
        retention_checkout_paused(&db, &*db.read())
            .await
            .expect("pause read")
    );
}

#[tokio::test]
async fn paused_checkout_replays_existing_order_and_terminal_attempt() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 24)).await;
    seed_checkout_governance(&db).await;
    let store = PaymentOrderStore::new(db.clone());
    let rate = ExchangeRateSnapshot {
        base: "USD".to_string(),
        quote: "CNY".to_string(),
        cny_per_usd: "6.0000".to_string(),
        source_updated_at: instant(),
        refreshed_at: instant(),
    };
    let order_input = CreatePaymentOrderInput {
        idempotency_key: "retention-replay-order".to_string(),
        product_id: "retention-product".to_string(),
        payment_channel_id: "store-channel-stripe".to_string(),
        payment_currency: Currency::CNY,
        custom_recharge_minor: None,
    };

    let order = store
        .create_order("retention-user", order_input.clone(), &rate)
        .await
        .expect("create order before pause");
    let attempt = store
        .create_attempt(
            "retention-user",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "retention-replay-attempt".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .expect("create attempt before pause");
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_payment_attempts SET state = 'expired' WHERE id = $1",
            vec![attempt.id.clone().into()],
        ))
        .await
        .expect("expire attempt");
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_retention_state
             SET checkout_paused = 1, consecutive_failures = 3,
                 updated_at = '2026-08-28T12:00:00.000000Z'
             WHERE singleton_id = 1",
        )
        .await
        .expect("pause checkout");

    let replayed_order = store
        .create_order("retention-user", order_input, &rate)
        .await
        .expect("replay order while paused");
    assert_eq!(replayed_order.id, order.id);

    let replay = store
        .create_attempt_with_outcome(
            "retention-user",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "retention-replay-attempt".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .expect("replay terminal attempt while paused");
    assert!(replay.replayed);
    assert_eq!(replay.attempt.id, attempt.id);
    assert_eq!(replay.attempt.state, PaymentAttemptState::Expired);

    let new_order = store
        .create_order(
            "retention-user",
            CreatePaymentOrderInput {
                idempotency_key: "retention-paused-new-order".to_string(),
                product_id: "retention-product".to_string(),
                payment_channel_id: "store-channel-stripe".to_string(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &rate,
        )
        .await
        .expect_err("new order while paused");
    assert!(matches!(new_order, PaymentOrderError::RetentionPaused));

    let new_attempt = store
        .create_attempt(
            "retention-user",
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "retention-paused-new-attempt".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .expect_err("new attempt while paused");
    assert!(matches!(new_attempt, PaymentOrderError::RetentionPaused));
}

#[tokio::test]
async fn bounded_clearing_caps_raw_and_network_classes_at_batch_size() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 24)).await;
    let total = (RETENTION_BATCH_SIZE as usize) + 1;
    for index in 0..total {
        insert_provider_event(
            &db,
            &format!("evt-bulk-{index:04}"),
            "2026-01-01T00:00:00.000000Z",
            true,
            true,
        )
        .await;
    }

    let retention = StoreRetention::new(db.clone(), "owner-a");
    let first = retention
        .run_at(instant(), actor("bounded-raw-network-1"))
        .await
        .expect("first bounded run");
    assert_eq!(first.state, StoreRetentionRunState::Succeeded);
    assert_eq!(first.counts.raw_callback_bodies, RETENTION_BATCH_SIZE as u64);
    assert_eq!(first.counts.network_metadata, RETENTION_BATCH_SIZE as u64);
    assert_eq!(first.counts.financial_records, 0);
    assert_eq!(
        count_rows(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE raw_ciphertext_base64 IS NOT NULL",
        )
        .await,
        1
    );
    assert_eq!(
        count_rows(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE source_ip IS NOT NULL OR user_agent IS NOT NULL",
        )
        .await,
        1
    );

    let second = retention
        .run_at(instant() + Duration::seconds(1), actor("bounded-raw-network-2"))
        .await
        .expect("second bounded run");
    assert_eq!(second.state, StoreRetentionRunState::Succeeded);
    assert_eq!(second.counts.raw_callback_bodies, 1);
    assert_eq!(second.counts.network_metadata, 1);
    assert_eq!(
        count_rows(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE raw_ciphertext_base64 IS NOT NULL",
        )
        .await,
        0
    );
    assert_eq!(
        count_rows(
            &db,
            "SELECT COUNT(*) AS value FROM store_provider_events
             WHERE source_ip IS NOT NULL OR user_agent IS NOT NULL",
        )
        .await,
        0
    );
    // SB-PR-11C: the event rows themselves must remain after clearing.
    assert_eq!(
        count_rows(&db, "SELECT COUNT(*) AS value FROM store_provider_events").await,
        total as i64
    );
}

#[tokio::test]
async fn run_failure_after_partial_clearing_rolls_back_cleared_data() {
    let db = setup().await;
    insert_privacy(&db, "v1", &retention_json(2557, 24)).await;
    // Eligible for both raw-callback and network-metadata clearing at instant().
    insert_provider_event(&db, "evt-rollback", "2026-05-01T00:00:00.000000Z", true, true).await;
    // Expired grant that the run deletes inside its write transaction.
    insert_reauth_grant(&db, "grant-expired", "2026-01-02T00:00:00.000000Z").await;
    // The corrupt expiry sorts lexically after every RFC3339 timestamp, so the
    // deletion candidate query (expires_at <= cutoff) skips it. Once the run has
    // deleted grant-expired, this row is the only grant left, so oldest_remaining
    // reads it after the clearing already happened and parse_time fails. This
    // forces the SB-PR-11B failure path through execute_success/finish_transaction.
    insert_reauth_grant(&db, "grant-corrupt", "invalid-expiry").await;

    let retention = StoreRetention::new(db.clone(), "owner-a");
    let run = retention
        .run_at(instant(), actor("rollback-run"))
        .await
        .expect("failed run is recorded");

    assert_eq!(run.state, StoreRetentionRunState::Failed);
    assert_eq!(run.error_category.as_deref(), Some("storage"));
    assert_eq!(run.counts.raw_callback_bodies, 0);
    assert_eq!(run.counts.network_metadata, 0);
    assert_eq!(run.counts.expired_reauth_grants, 0);
    assert_eq!(run.oldest_remaining_at, None);

    // The rollback must restore the cleared event fields and the deleted grant.
    assert!(event_raw_present(&db, "evt-rollback").await);
    assert!(event_network_present(&db, "evt-rollback").await);
    assert_eq!(
        count_rows(&db, "SELECT COUNT(*) AS value FROM store_reauth_grants").await,
        2
    );
    let status = retention.status().await.expect("status");
    assert_eq!(status.consecutive_failures, 1);
    assert!(!status.checkout_paused);
    assert_eq!(
        count_where(
            &db,
            "SELECT COUNT(*) AS value FROM store_access_audits
             WHERE action = 'retention_run' AND result = $1",
            "failed",
        )
        .await,
        1
    );

    // With the corrupt grant removed, the same data set clears successfully,
    // proving the first run failed only after the clearing statements ran.
    db.write()
        .await
        .execute(db.stmt(
            "DELETE FROM store_reauth_grants WHERE id = $1",
            vec!["grant-corrupt".into()],
        ))
        .await
        .expect("remove corrupt grant");
    let second = retention
        .run_at(instant() + Duration::seconds(1), actor("rollback-run-2"))
        .await
        .expect("second run");
    assert_eq!(second.state, StoreRetentionRunState::Succeeded);
    assert_eq!(second.counts.raw_callback_bodies, 1);
    assert_eq!(second.counts.network_metadata, 1);
    assert_eq!(second.counts.expired_reauth_grants, 1);
    assert_eq!(
        second.oldest_remaining_at,
        Some(Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap())
    );
    assert!(!event_raw_present(&db, "evt-rollback").await);
    assert!(!event_network_present(&db, "evt-rollback").await);
    assert_eq!(
        count_rows(&db, "SELECT COUNT(*) AS value FROM store_reauth_grants").await,
        0
    );
}

#[tokio::test]
async fn malformed_privacy_retention_json_fails_run() {
    let db = setup().await;
    db.write()
        .await
        .execute(db.stmt(
            "INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('privacy-malformed', 'malformed', 'CN', '[\"CN\"]', $1, 'contract',
                     'retention-admin', $2, '2026-01-01T00:00:00.000000Z',
                     '2099-01-01T00:00:00.000000Z', 1)",
            vec!["{\"raw_callback_days\":".into(), DIGEST.into()],
        ))
        .await
        .expect("insert malformed privacy record");

    let run = StoreRetention::new(db.clone(), "owner-a")
        .run_at(instant(), actor("malformed-policy"))
        .await
        .expect("run");
    assert_eq!(run.state, StoreRetentionRunState::Failed);
    assert_eq!(run.policy_version, "malformed");
    assert_eq!(run.error_category.as_deref(), Some("privacy_policy_invalid"));
}
