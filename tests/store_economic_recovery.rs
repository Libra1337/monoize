use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::exchange_rate::ExchangeRateSnapshot;
use monoize::store_billing::money::Currency;
use monoize::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderError, PaymentOrderStore,
};
use monoize::store_billing::recovery::{
    BeginRefundInput, RecoveryClaimKind, RecoveryError, RecoveryStore, VerifiedRecoveryClaimInput,
};
use monoize::store_billing::settlement::{
    SettlementLineClass, SettlementLineInput, SettlementReportInput, SettlementStore,
};
use monoize::store_billing::{
    GenerateRedemptionCodesInput, RedemptionCodeStatus, RedemptionRewardInput, StoreBillingError,
    StoreBillingStore,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

const USER_ID: &str = "recovery-user";
const CREDENTIAL_ID: &str = "recovery-credential";
const ORIGINAL_REWARD: i128 = 2_000_000_000;

struct PaidOrderFixture {
    db: DbPool,
    order_id: String,
}

async fn setup_paid_balance_order(current_balance: i128) -> PaidOrderFixture {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    {
        let write = db.write().await;
        Migrator::up(&*write, None).await.expect("run migrations");
        write
            .execute_unprepared(
                "INSERT INTO users
                    (id, username, password_hash, role, created_at, updated_at, enabled,
                     balance_nano_usd, balance_unlimited, group_id)
                 SELECT 'recovery-user', 'recovery-user', 'test', 'user',
                        '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z', 1, '0', 0, id
                 FROM monoize_groups WHERE is_default = 1 LIMIT 1",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_products
                    (id, kind, name, description, price_currency, price_minor,
                     duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
                 VALUES ('recovery-product', 'balance', 'Recharge', '', 'CNY', '1000',
                         NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_balance_products (product_id, recharge_minor, bonus_minor)
                 VALUES ('recovery-product', '1000', '200')",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "UPDATE store_payment_channels SET enabled = 1
                 WHERE id = 'store-channel-stripe'",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_payment_compliance
                    (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
                 VALUES ('recovery-compliance', 'store-channel-stripe', '2026-08-28',
                         'recovery-admin', '127.0.0.1', '2026-08-28T00:00:00Z')",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_merchant_capabilities
                    (id, channel_id, capability, state, environment,
                     merchant_account_digest, provider_product, evidence_digest,
                     verifier_admin_id, verified_at, expires_at)
                 VALUES
                    ('recovery-cap-payment-query', 'store-channel-stripe', 'payment_query',
                     'supported', 'sandbox',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'checkout',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'recovery-admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                    ('recovery-cap-refund', 'store-channel-stripe', 'refund',
                     'supported', 'sandbox',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'checkout',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'recovery-admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                    ('recovery-cap-refund-query', 'store-channel-stripe', 'refund_query',
                     'supported', 'sandbox',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'checkout',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'recovery-admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                    ('recovery-cap-settlement', 'store-channel-stripe', 'settlement_report',
                     'supported', 'sandbox',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'checkout',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'recovery-admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z')",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_privacy_records
                    (id, policy_version, jurisdiction, allowed_regions_json,
                     retention_json, legal_basis, reviewer_id, evidence_digest,
                     approved_at, next_review_at, accepted)
                 VALUES ('recovery-privacy', 'v1', 'CN', '[]', '{}', 'contract',
                         'recovery-admin',
                         'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                         '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z', 1)",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_channel_readiness_profiles
                    (channel_id, active_credential_digest, privacy_record_id,
                     callback_verification_passed, supported_currencies_json,
                     amount_limits_json, checkout_action_kinds_json,
                     license_evidence_digest, runtime_evidence_digest,
                     availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
                 VALUES ('store-channel-stripe',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                         'recovery-privacy', 1, '[\"CNY\"]',
                         '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                         '[\"redirect\"]',
                         'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                         'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                         'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                         'recovery-admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z')",
            )
            .await
            .unwrap();
        write
            .execute_unprepared(
                "INSERT INTO store_channel_credentials
                    (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                     ciphertext_base64, account_identity_digest, status, created_at)
                 VALUES ('recovery-credential', 'store-channel-stripe', 'stripe', 1,
                         'key-1', 'bm9uY2U=', 'Y2lwaGVydGV4dA==',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                         'active', '2026-08-27T00:00:00Z')",
            )
            .await
            .unwrap();
    }
    let orders = PaymentOrderStore::new(db.clone());
    let order = orders
        .create_order(
            USER_ID,
            CreatePaymentOrderInput {
                idempotency_key: "recovery-order".to_string(),
                product_id: "recovery-product".to_string(),
                payment_channel_id: "store-channel-stripe".to_string(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &rate(),
        )
        .await
        .unwrap();
    let attempt = orders
        .create_attempt(
            USER_ID,
            &order.id,
            CreatePaymentAttemptInput {
                idempotency_key: "recovery-attempt".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    let now = Utc::now().to_rfc3339();
    let write = db.write().await;
    write
        .execute(db.stmt(
            "UPDATE store_orders
             SET payment_state = 'paid', fulfillment_state = 'fulfilled',
                 paid_at = $2, fulfilled_at = $2, updated_at = $2,
                 state_revision = state_revision + 1
             WHERE id = $1",
            vec![order.id.clone().into(), now.clone().into()],
        ))
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "UPDATE store_payment_attempts
             SET state = 'paid', provider_object_id = 'cs-recovery',
                 provider_transaction_id = 'pi-recovery', paid_at = $2, updated_at = $2
             WHERE id = $1",
            vec![attempt.id.into(), now.clone().into()],
        ))
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "UPDATE users SET balance_nano_usd = $2 WHERE id = $1",
            vec![USER_ID.into(), current_balance.to_string().into()],
        ))
        .await
        .unwrap();
    write
        .execute(db.stmt(
            "INSERT INTO billing_ledger
                (id, user_id, kind, delta_nano_usd, balance_after_nano_usd,
                 meta_json, created_at, idempotency_key)
             VALUES ('recovery-fulfillment', $1, 'store_recharge', $2, $3,
                     $4, $5, $6)",
            vec![
                USER_ID.into(),
                ORIGINAL_REWARD.to_string().into(),
                current_balance.to_string().into(),
                serde_json::json!({"order_id": order.id}).to_string().into(),
                now.into(),
                format!("store:fulfillment:{}", order.id).into(),
            ],
        ))
        .await
        .unwrap();
    drop(write);
    PaidOrderFixture {
        db,
        order_id: order.id,
    }
}

async fn setup_paid_fulfilled_plan_order() -> PaidOrderFixture {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    fixture
        .db
        .write()
        .await
        .execute_unprepared(
            "INSERT INTO store_products
                (id, kind, name, description, price_currency, price_minor,
                 duration_seconds, group_ids, sort_order, enabled, created_at, updated_at)
             VALUES ('recovery-plan', 'plan', 'Plan', '', 'CNY', '5900',
                     2592000, '[]', 0, 1,
                     '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
        )
        .await
        .unwrap();
    let now = Utc::now().to_rfc3339();
    let write = fixture.db.write().await;
    write
        .execute(fixture.db.stmt(
            "INSERT INTO store_orders
                (id, order_number, user_id, product_id, product_kind, payment_state,
                 fulfillment_state, dispute_state, payment_hold, payment_channel_id,
                 payment_currency, payment_minor, cny_per_usd, rate_numerator,
                 rate_denominator, rate_source_updated_at, quote_json, contract_version,
                 state_revision, creation_idempotency_key, creation_request_digest,
                 expires_at, created_at, updated_at, paid_at, fulfilled_at)
             VALUES ('recovery-plan-order-id', 'LS-RECOVERY-PLAN', $1,
                     'recovery-plan', 'plan', 'paid', 'fulfilled', 'none', 0,
                     'store-channel-stripe', 'CNY', '5900', '6.0000', '6', '1',
                     '2026-08-27T00:00:00Z', '{}', 2, 1,
                     'recovery-plan-order', 'recovery-plan-digest',
                     '2026-08-27T01:00:00Z', $2, $2, $2, $2)",
            vec![USER_ID.into(), now.clone().into()],
        ))
        .await
        .unwrap();
    write
        .execute(fixture.db.stmt(
            "INSERT INTO store_payment_attempts
                (id, order_id, channel_id, adapter_kind, credential_version_id,
                 merchant_account_identity, expected_payment_method,
                 payment_contract_version, state, provider_transaction_id,
                 provider_object_id, idempotency_key, paid_at, created_at, updated_at)
             VALUES ('recovery-plan-attempt-id', 'recovery-plan-order-id',
                     'store-channel-stripe', 'stripe', $1,
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'card', 2, 'paid', 'pi-plan-recovery', 'cs-plan-recovery',
                     'recovery-plan-attempt', $2, $2, $2)",
            vec![CREDENTIAL_ID.into(), now.into()],
        ))
        .await
        .unwrap();
    drop(write);
    PaidOrderFixture {
        db: fixture.db,
        order_id: "recovery-plan-order-id".to_string(),
    }
}

fn rate() -> ExchangeRateSnapshot {
    ExchangeRateSnapshot {
        base: "USD".to_string(),
        quote: "CNY".to_string(),
        cny_per_usd: "6.0000".to_string(),
        source_updated_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 0, 0).unwrap(),
        refreshed_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 1, 0).unwrap(),
    }
}

fn refund_input(order_id: &str, key: &str) -> BeginRefundInput {
    BeginRefundInput {
        order_id: order_id.to_string(),
        requested_by_admin_id: "admin-user".to_string(),
        idempotency_key: key.to_string(),
    }
}

fn dispute_input(order_id: &str, provider_claim_id: &str) -> VerifiedRecoveryClaimInput {
    VerifiedRecoveryClaimInput {
        order_id: order_id.to_string(),
        credential_version_id: CREDENTIAL_ID.to_string(),
        provider_claim_id: provider_claim_id.to_string(),
        provider_event_row_id: None,
        kind: RecoveryClaimKind::Dispute,
    }
}

#[tokio::test]
async fn refund_reserves_the_original_balance_once_before_provider_mutation() {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    let store = RecoveryStore::new(fixture.db.clone());
    let input = refund_input(&fixture.order_id, "refund-key-1");

    let first = store.begin_refund(input.clone()).await.unwrap();
    let replay = store.begin_refund(input).await.unwrap();

    assert_eq!(replay, first);
    assert_eq!(first.original_nano_usd, ORIGINAL_REWARD.to_string());
    let row = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT o.payment_state, u.balance_nano_usd, r.reserved_nano_usd
             FROM store_orders o
             JOIN users u ON u.id = o.user_id
             JOIN store_order_reward_recoveries r ON r.order_id = o.id
             WHERE o.id = $1",
            vec![fixture.order_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "payment_state").unwrap(),
        "refund_pending"
    );
    assert_eq!(row.try_get::<String>("", "balance_nano_usd").unwrap(), "0");
    assert_eq!(
        row.try_get::<String>("", "reserved_nano_usd").unwrap(),
        ORIGINAL_REWARD.to_string()
    );
    assert_eq!(
        ledger_count(
            &fixture.db,
            &format!("store:recovery:{}:reserve", first.recovery_id)
        )
        .await,
        1
    );
}

#[tokio::test]
async fn concurrent_refund_starts_create_one_reserve_and_one_refund() {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    let first = RecoveryStore::new(fixture.db.clone());
    let second = first.clone();
    let order_id = fixture.order_id.clone();
    let (a, b) = tokio::join!(
        first.begin_refund(refund_input(&order_id, "refund-concurrent-a")),
        second.begin_refund(refund_input(&order_id, "refund-concurrent-b")),
    );

    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(
        a.err().or_else(|| b.err()).unwrap(),
        RecoveryError::OrderNotRecoverable | RecoveryError::Conflict
    ));
    assert_eq!(table_count(&fixture.db, "store_refunds").await, 1);
    assert_eq!(
        table_count(&fixture.db, "store_order_reward_recoveries").await,
        1
    );
}

#[tokio::test]
async fn definite_refund_rejection_releases_the_reserve_once() {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    let store = RecoveryStore::new(fixture.db.clone());
    let refund = store
        .begin_refund(refund_input(&fixture.order_id, "refund-rejected"))
        .await
        .unwrap();

    store.reject_refund(&refund.id).await.unwrap();
    store.reject_refund(&refund.id).await.unwrap();

    let row = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT o.payment_state, u.balance_nano_usd, r.state
             FROM store_orders o
             JOIN users u ON u.id = o.user_id
             JOIN store_order_reward_recoveries r ON r.order_id = o.id
             WHERE o.id = $1",
            vec![fixture.order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "payment_state").unwrap(), "paid");
    assert_eq!(
        row.try_get::<String>("", "balance_nano_usd").unwrap(),
        ORIGINAL_REWARD.to_string()
    );
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "released");
    assert_eq!(
        ledger_count(
            &fixture.db,
            &format!("store:recovery:{}:release", refund.recovery_id)
        )
        .await,
        1
    );
}

#[tokio::test]
async fn verified_refund_consumes_the_reserve_without_a_second_debit() {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    let store = RecoveryStore::new(fixture.db.clone());
    let refund = store
        .begin_refund(refund_input(&fixture.order_id, "refund-succeeded"))
        .await
        .unwrap();
    store
        .mark_refund_pending(&refund.id, "provider-refund-1")
        .await
        .unwrap();

    store.complete_refund(&refund.id).await.unwrap();
    store.complete_refund(&refund.id).await.unwrap();

    let row = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT o.payment_state, u.balance_nano_usd,
                    r.reserved_nano_usd, r.recovered_nano_usd, r.state
             FROM store_orders o
             JOIN users u ON u.id = o.user_id
             JOIN store_order_reward_recoveries r ON r.order_id = o.id
             WHERE o.id = $1",
            vec![fixture.order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "payment_state").unwrap(),
        "refunded"
    );
    assert_eq!(row.try_get::<String>("", "balance_nano_usd").unwrap(), "0");
    assert_eq!(row.try_get::<String>("", "reserved_nano_usd").unwrap(), "0");
    assert_eq!(
        row.try_get::<String>("", "recovered_nano_usd").unwrap(),
        ORIGINAL_REWARD.to_string()
    );
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "recovered");
}

#[tokio::test]
async fn fulfilled_plan_orders_are_not_refundable() {
    let fixture = setup_paid_fulfilled_plan_order().await;

    assert_eq!(
        RecoveryStore::new(fixture.db)
            .begin_refund(refund_input(&fixture.order_id, "refund-plan"))
            .await
            .unwrap_err(),
        RecoveryError::OrderNotRecoverable
    );
}

#[tokio::test]
async fn dispute_loss_shares_one_reserve_and_debits_only_the_unreserved_remainder() {
    let fixture = setup_paid_balance_order(500_000_000).await;
    let store = RecoveryStore::new(fixture.db.clone());
    let dispute = store
        .open_claim(dispute_input(&fixture.order_id, "dispute-1"))
        .await
        .unwrap();
    let replay = store
        .open_claim(dispute_input(&fixture.order_id, "dispute-1"))
        .await
        .unwrap();
    assert_eq!(replay, dispute);

    store.lose_claim(&dispute.id).await.unwrap();
    store.lose_claim(&dispute.id).await.unwrap();

    let row = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT o.dispute_state, o.payment_hold, u.balance_nano_usd,
                    r.reserved_nano_usd, r.recovered_nano_usd
             FROM store_orders o
             JOIN users u ON u.id = o.user_id
             JOIN store_order_reward_recoveries r ON r.order_id = o.id
             WHERE o.id = $1",
            vec![fixture.order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "dispute_state").unwrap(), "lost");
    assert_eq!(row.try_get::<i32>("", "payment_hold").unwrap(), 1);
    assert_eq!(
        row.try_get::<String>("", "balance_nano_usd").unwrap(),
        "-1500000000"
    );
    assert_eq!(row.try_get::<String>("", "reserved_nano_usd").unwrap(), "0");
    assert_eq!(
        row.try_get::<String>("", "recovered_nano_usd").unwrap(),
        ORIGINAL_REWARD.to_string()
    );
}

#[tokio::test]
async fn dispute_win_releases_the_shared_reserve_and_clears_hold() {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    let store = RecoveryStore::new(fixture.db.clone());
    let dispute = store
        .open_claim(dispute_input(&fixture.order_id, "dispute-won"))
        .await
        .unwrap();

    store.win_claim(&dispute.id).await.unwrap();
    store.win_claim(&dispute.id).await.unwrap();

    let row = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT o.dispute_state, o.payment_hold, u.balance_nano_usd, h.active
             FROM store_orders o
             JOIN users u ON u.id = o.user_id
             JOIN store_balance_holds h ON h.user_id = o.user_id
             WHERE o.id = $1",
            vec![fixture.order_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "dispute_state").unwrap(), "won");
    assert_eq!(row.try_get::<i32>("", "payment_hold").unwrap(), 0);
    assert_eq!(row.try_get::<i32>("", "active").unwrap(), 0);
    assert_eq!(
        row.try_get::<String>("", "balance_nano_usd").unwrap(),
        ORIGINAL_REWARD.to_string()
    );
}

#[tokio::test]
async fn payment_hold_blocks_new_orders_and_redemption_without_consuming_the_code() {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    let billing = StoreBillingStore::new(fixture.db.clone());
    let generated = billing
        .generate_redemption_codes(
            &PaymentKeyRing::new(
                PaymentKey::new("recovery-redemption", [73_u8; 32]).unwrap(),
                vec![],
            )
            .unwrap(),
            "admin-user",
            GenerateRedemptionCodesInput {
                reward: RedemptionRewardInput::Balance {
                    currency: Currency::USD,
                    amount_minor: "100".to_string(),
                },
                count: 1,
                validity_days: 30,
            },
        )
        .await
        .unwrap();
    RecoveryStore::new(fixture.db.clone())
        .open_claim(dispute_input(&fixture.order_id, "dispute-hold"))
        .await
        .unwrap();

    assert_eq!(
        PaymentOrderStore::new(fixture.db.clone())
            .create_order(
                USER_ID,
                CreatePaymentOrderInput {
                    idempotency_key: "recovery-blocked-order".to_string(),
                    product_id: "recovery-product".to_string(),
                    payment_channel_id: "store-channel-stripe".to_string(),
                    payment_currency: Currency::CNY,
                    custom_recharge_minor: None,
                },
                &rate(),
            )
            .await
            .unwrap_err(),
        PaymentOrderError::PaymentHold
    );
    assert_eq!(
        billing
            .redeem(USER_ID, &generated[0].code, None, "203.0.113.75")
            .await
            .unwrap_err(),
        StoreBillingError::PaymentHold
    );
    assert_eq!(
        billing.list_redemption_codes_admin(10).await.unwrap()[0].status,
        RedemptionCodeStatus::Unused
    );
}

#[tokio::test]
async fn provider_claim_identity_cannot_be_rebound_to_another_order() {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    let orders = PaymentOrderStore::new(fixture.db.clone());
    let second = orders
        .create_order(
            USER_ID,
            CreatePaymentOrderInput {
                idempotency_key: "recovery-second-order".to_string(),
                product_id: "recovery-product".to_string(),
                payment_channel_id: "store-channel-stripe".to_string(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &rate(),
        )
        .await
        .unwrap();
    let attempt = orders
        .create_attempt(
            USER_ID,
            &second.id,
            CreatePaymentAttemptInput {
                idempotency_key: "recovery-second-attempt".to_string(),
                expected_payment_method: Some("card".to_string()),
            },
        )
        .await
        .unwrap();
    let now = Utc::now().to_rfc3339();
    let write = fixture.db.write().await;
    write
        .execute(fixture.db.stmt(
            "UPDATE store_orders
             SET payment_state = 'paid', fulfillment_state = 'fulfilled',
                 paid_at = $2, fulfilled_at = $2, updated_at = $2,
                 state_revision = state_revision + 1 WHERE id = $1",
            vec![second.id.clone().into(), now.clone().into()],
        ))
        .await
        .unwrap();
    write
        .execute(fixture.db.stmt(
            "UPDATE store_payment_attempts
             SET state = 'paid', provider_object_id = 'cs-recovery-2',
                 provider_transaction_id = 'pi-recovery-2', paid_at = $2, updated_at = $2
             WHERE id = $1",
            vec![attempt.id.into(), now.clone().into()],
        ))
        .await
        .unwrap();
    write
        .execute(fixture.db.stmt(
            "INSERT INTO billing_ledger
                (id, user_id, kind, delta_nano_usd, balance_after_nano_usd,
                 meta_json, created_at, idempotency_key)
             VALUES ('recovery-fulfillment-2', $1, 'store_recharge', $2, $2,
                     $3, $4, $5)",
            vec![
                USER_ID.into(),
                ORIGINAL_REWARD.to_string().into(),
                serde_json::json!({"order_id": second.id}).to_string().into(),
                now.into(),
                format!("store:fulfillment:{}", second.id).into(),
            ],
        ))
        .await
        .unwrap();
    drop(write);

    let recovery = RecoveryStore::new(fixture.db);
    recovery
        .open_claim(dispute_input(&fixture.order_id, "provider-claim-shared"))
        .await
        .unwrap();
    assert_eq!(
        recovery
            .open_claim(dispute_input(&second.id, "provider-claim-shared"))
            .await
            .unwrap_err(),
        RecoveryError::Conflict
    );
}

#[tokio::test]
async fn settlement_import_is_idempotent_and_unmatched_money_creates_one_case() {
    let fixture = setup_paid_balance_order(ORIGINAL_REWARD).await;
    let store = SettlementStore::new(fixture.db.clone());
    let input = SettlementReportInput {
        channel_id: "store-channel-stripe".to_string(),
        credential_version_id: CREDENTIAL_ID.to_string(),
        provider_report_id: "report-2026-08-27".to_string(),
        report_date: "2026-08-27".to_string(),
        body_digest: "b".repeat(64),
        lines: vec![
            SettlementLineInput {
                provider_line_id: "gross-matched".to_string(),
                class: SettlementLineClass::Gross,
                amount_minor: "1000".to_string(),
                currency: Currency::CNY,
                provider_transaction_id: Some("pi-recovery".to_string()),
            },
            SettlementLineInput {
                provider_line_id: "gross-unmatched".to_string(),
                class: SettlementLineClass::Gross,
                amount_minor: "1000".to_string(),
                currency: Currency::CNY,
                provider_transaction_id: Some("pi-unknown".to_string()),
            },
            SettlementLineInput {
                provider_line_id: "provider-fee".to_string(),
                class: SettlementLineClass::Fee,
                amount_minor: "-35".to_string(),
                currency: Currency::CNY,
                provider_transaction_id: None,
            },
        ],
    };

    let first = store.import_report(input.clone()).await.unwrap();
    let replay = store.import_report(input).await.unwrap();

    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(first.report_id, replay.report_id);
    assert_eq!(first.line_count, 3);
    assert_eq!(first.unmatched_count, 1);
    assert_eq!(table_count(&fixture.db, "store_settlement_lines").await, 3);
    assert_eq!(
        fixture
            .db
            .read()
            .query_one(fixture.db.stmt(
                "SELECT COUNT(*) AS value FROM store_reconciliation_cases
                 WHERE kind = 'unmatched_settlement'",
                vec![],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "value")
            .unwrap(),
        1
    );
    let balance = fixture
        .db
        .read()
        .query_one(fixture.db.stmt(
            "SELECT balance_nano_usd FROM users WHERE id = $1",
            vec![USER_ID.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "balance_nano_usd")
        .unwrap();
    assert_eq!(balance, ORIGINAL_REWARD.to_string());
}

async fn ledger_count(db: &DbPool, key: &str) -> i64 {
    db.read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS value FROM billing_ledger WHERE idempotency_key = $1",
            vec![key.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap()
}

async fn table_count(db: &DbPool, table: &str) -> i64 {
    db.read()
        .query_one(db.stmt(&format!("SELECT COUNT(*) AS value FROM {table}"), vec![]))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap()
}
