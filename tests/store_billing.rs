use chrono::{Duration, TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::crypto::{PaymentKey, PaymentKeyRing};
use monoize::store_billing::order::{CreatePaymentOrderInput, PaymentOrderStore};
use monoize::store_billing::quota_gate::{GateSlot, QuotaGateStore, QuotaManifest};
use monoize::store_billing::{
    BalanceProductInput, CreatePaymentChannelInput, CreateProductInput, Currency,
    ExchangeRateSnapshot, GenerateRedemptionCodesInput, IconKind, PaymentAdapterKind,
    PlanQuotaInput, ProductKind, RedemptionCodeStatus, RedemptionRewardInput, StoreBillingError,
    StoreBillingStore, StoreSettings, UpdatePaymentChannelInput, WindowKind,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

async fn setup() -> (DbPool, StoreBillingStore) {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    Migrator::up(&*db.write().await, None)
        .await
        .expect("run migrations");
    let store = StoreBillingStore::new(db.clone());
    (db, store)
}

async fn insert_user(db: &DbPool, id: &str) {
    let group = db
        .read()
        .query_one(db.stmt("SELECT id FROM monoize_groups WHERE is_default = 1", vec![]))
        .await
        .expect("query default group")
        .expect("default group");
    let group_id: String = group.try_get("", "id").expect("group id");
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
                "2026-08-27T00:00:00Z".into(),
                group_id.into(),
            ],
        ))
        .await
        .expect("insert user");
}

async fn default_group_id(db: &DbPool) -> String {
    db.read()
        .query_one(db.stmt("SELECT id FROM monoize_groups WHERE is_default = 1", vec![]))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "id")
        .unwrap()
}

fn rate() -> ExchangeRateSnapshot {
    ExchangeRateSnapshot {
        base: "USD".to_string(),
        quote: "CNY".to_string(),
        cny_per_usd: "6.0000".to_string(),
        source_updated_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 0, 0).unwrap(),
        refreshed_at: Utc.with_ymd_and_hms(2026, 8, 27, 0, 5, 0).unwrap(),
    }
}

fn redemption_keys() -> PaymentKeyRing {
    PaymentKeyRing::new(
        PaymentKey::new("store-billing-redemption", [71_u8; 32]).unwrap(),
        vec![],
    )
    .unwrap()
}

async fn pass_quota_gate(db: &DbPool) {
    let gate = QuotaGateStore::new(db.clone());
    let manifest = QuotaManifest::passed(
        gate.live_environment().await.unwrap(),
        "store-billing-test",
        "store-billing-drill",
        Utc::now(),
        "store-billing-admin",
    )
    .unwrap();
    gate.import_manifest(GateSlot::Current, manifest)
        .await
        .unwrap();
}

async fn seed_governed_stripe(db: &DbPool) {
    db.write()
        .await
        .execute_unprepared(
            "UPDATE store_payment_channels SET enabled = 1
             WHERE id = 'store-channel-stripe';
             INSERT INTO store_channel_credentials
                (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
                 ciphertext_base64, account_identity_digest, status, created_at)
             VALUES ('unit-governance-credential', 'store-channel-stripe', 'stripe', 1,
                     'key', 'nonce', 'ciphertext',
                     '2222222222222222222222222222222222222222222222222222222222222222',
                     'active', '2026-08-28T00:00:00Z');
             INSERT INTO store_payment_compliance
                (id, channel_id, terms_version, admin_user_id, source_ip, confirmed_at)
             VALUES ('unit-governance-compliance', 'store-channel-stripe', '2026-08-28',
                     'admin', '127.0.0.1', '2026-08-28T00:00:00Z');
             INSERT INTO store_merchant_capabilities
                (id, channel_id, capability, state, environment, merchant_account_digest,
                 provider_product, evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES
                ('unit-cap-payment-query', 'store-channel-stripe', 'payment_query', 'supported',
                 'sandbox', '2222222222222222222222222222222222222222222222222222222222222222', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin',
                 '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('unit-cap-refund', 'store-channel-stripe', 'refund', 'supported',
                 'sandbox', '2222222222222222222222222222222222222222222222222222222222222222', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin',
                 '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('unit-cap-refund-query', 'store-channel-stripe', 'refund_query', 'supported',
                 'sandbox', '2222222222222222222222222222222222222222222222222222222222222222', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin',
                 '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z'),
                ('unit-cap-settlement', 'store-channel-stripe', 'settlement_report', 'supported',
                 'sandbox', '2222222222222222222222222222222222222222222222222222222222222222', 'checkout', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'admin',
                 '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z');
             INSERT INTO store_privacy_records
                (id, policy_version, jurisdiction, allowed_regions_json, retention_json,
                 legal_basis, reviewer_id, evidence_digest, approved_at, next_review_at, accepted)
             VALUES ('unit-governance-privacy', 'v1', 'CN', '[]', '{}', 'contract', 'admin',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z', 1);
             INSERT INTO store_channel_readiness_profiles
                (channel_id, active_credential_digest, privacy_record_id,
                 callback_verification_passed, supported_currencies_json, amount_limits_json,
                 checkout_action_kinds_json, license_evidence_digest, runtime_evidence_digest,
                 availability_evidence_digest, verifier_admin_id, verified_at, expires_at)
             VALUES ('store-channel-stripe',
                     '2222222222222222222222222222222222222222222222222222222222222222',
                     'unit-governance-privacy', 1, '[\"CNY\",\"USD\"]',
                     '{\"CNY\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"},\"USD\":{\"min_minor\":\"1\",\"max_minor\":\"100000000\"}}',
                     '[\"redirect\"]',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                     'admin', '2026-08-28T00:00:00Z', '2099-01-01T00:00:00Z')",
        )
        .await
        .unwrap();
}

fn balance_product(name: &str, sort_order: i32, enabled: bool) -> CreateProductInput {
    CreateProductInput {
        kind: ProductKind::Balance,
        name: name.to_string(),
        description: String::new(),
        price_currency: Currency::CNY,
        price_minor: "1000".to_string(),
        duration_seconds: None,
        group_ids: vec![],
        sort_order,
        enabled,
        balance: Some(BalanceProductInput {
            recharge_minor: "1000".to_string(),
            bonus_minor: "200".to_string(),
        }),
        quotas: vec![],
    }
}

fn plan_product(name: &str, quota_fen_cny: &str) -> CreateProductInput {
    CreateProductInput {
        kind: ProductKind::Plan,
        name: name.to_string(),
        description: "Plan snapshot".to_string(),
        price_currency: Currency::CNY,
        price_minor: "5900".to_string(),
        duration_seconds: Some(30 * 86_400),
        group_ids: vec![],
        sort_order: 0,
        enabled: true,
        balance: None,
        quotas: vec![PlanQuotaInput {
            window_kind: WindowKind::Day,
            window_seconds: 86_400,
            quota_fen_cny: quota_fen_cny.to_string(),
            sort_order: 0,
        }],
    }
}

fn payment_channel(name: &str, sort_order: i32, enabled: bool) -> CreatePaymentChannelInput {
    CreatePaymentChannelInput {
        adapter_kind: PaymentAdapterKind::Http,
        name: name.to_string(),
        icon_kind: IconKind::Builtin,
        icon_value: Some("custom".to_string()),
        sort_order,
        enabled,
    }
}

#[tokio::test]
async fn pending_sqlite_gate_blocks_enabled_plan_creation_but_not_balance() {
    let (_db, store) = setup().await;

    assert_eq!(
        store
            .create_product(plan_product("Blocked plan", "2000"))
            .await
            .unwrap_err(),
        StoreBillingError::ProductNotAvailable
    );

    let mut disabled_plan = plan_product("Disabled plan", "2000");
    disabled_plan.enabled = false;
    assert!(store.create_product(disabled_plan).await.is_ok());
    assert!(
        store
            .create_product(balance_product("Allowed balance", 0, true))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn pending_sqlite_gate_blocks_enabling_an_existing_plan() {
    let (_db, store) = setup().await;
    let mut plan = plan_product("Disabled plan", "2000");
    plan.enabled = false;
    let created = store.create_product(plan.clone()).await.unwrap();

    plan.enabled = true;
    assert_eq!(
        store.update_product(&created.id, plan).await.unwrap_err(),
        StoreBillingError::ProductNotAvailable
    );
}

#[tokio::test]
async fn mismatched_sqlite_gate_hides_enabled_plans_from_catalog() {
    let (db, store) = setup().await;
    pass_quota_gate(&db).await;
    store
        .create_product(plan_product("Hidden plan", "2000"))
        .await
        .unwrap();
    store
        .create_product(balance_product("Visible balance", 1, true))
        .await
        .unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_quota_gates SET compatibility_fingerprint = 'mismatched'
             WHERE backend = 'sqlite' AND slot = 'current'",
            vec![],
        ))
        .await
        .unwrap();

    let catalog = store.catalog().await.unwrap();
    assert_eq!(catalog.products.len(), 1);
    assert_eq!(catalog.products[0].kind, ProductKind::Balance);
}

#[tokio::test]
async fn pending_sqlite_gate_rejects_plan_codes_but_not_balance_codes() {
    let (db, store) = setup().await;
    insert_user(&db, "gate-code-admin").await;
    let mut plan = plan_product("Code plan", "2000");
    plan.enabled = false;
    let plan = store.create_product(plan).await.unwrap();
    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_products SET enabled = 1 WHERE id = $1",
            vec![plan.id.clone().into()],
        ))
        .await
        .unwrap();

    assert_eq!(
        store
            .generate_redemption_codes(
                &redemption_keys(),
                "gate-code-admin",
                GenerateRedemptionCodesInput {
                    reward: RedemptionRewardInput::Plan {
                        product_id: plan.id,
                    },
                    count: 1,
                    validity_days: 30,
                },
            )
            .await
            .unwrap_err(),
        StoreBillingError::ProductNotAvailable
    );
    assert!(
        store
            .generate_redemption_codes(
                &redemption_keys(),
                "gate-code-admin",
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
            .is_ok()
    );
}

#[tokio::test]
async fn catalog_filters_disabled_records_and_uses_stable_order() {
    let (db, store) = setup().await;
    store
        .create_product(balance_product("Later", 20, true))
        .await
        .unwrap();
    store
        .create_product(balance_product("Hidden", 0, false))
        .await
        .unwrap();
    store
        .create_product(balance_product("First", 10, true))
        .await
        .unwrap();
    store
        .create_payment_channel(payment_channel("Second channel", 20, true))
        .await
        .unwrap();
    store
        .create_payment_channel(payment_channel("Hidden channel", 0, false))
        .await
        .unwrap();
    store
        .create_payment_channel(payment_channel("First channel", 10, true))
        .await
        .unwrap();
    seed_governed_stripe(&db).await;

    let catalog = store.catalog().await.unwrap();
    assert_eq!(catalog.settings, StoreSettings::default());
    assert_eq!(
        catalog
            .products
            .iter()
            .map(|product| product.name.as_str())
            .collect::<Vec<_>>(),
        ["First", "Later"]
    );
    assert_eq!(
        catalog
            .payment_channels
            .iter()
            .map(|channel| channel.name.as_str())
            .collect::<Vec<_>>(),
        ["Stripe"]
    );
    assert_eq!(
        catalog.products[0]
            .balance
            .as_ref()
            .unwrap()
            .actual_received_minor,
        "1200"
    );
}

#[tokio::test]
async fn payment_channel_adapter_is_immutable_and_updates_require_the_current_revision() {
    let (_db, store) = setup().await;
    let channel = store
        .create_payment_channel(payment_channel("Custom provider", 0, false))
        .await
        .unwrap();
    assert_eq!(channel.adapter_kind, PaymentAdapterKind::Http);
    assert_eq!(channel.revision, 1);

    let error = store
        .update_payment_channel(
            &channel.id,
            UpdatePaymentChannelInput {
                adapter_kind: Some(PaymentAdapterKind::Stripe),
                expected_revision: channel.revision,
                name: Some("Stripe backup".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error, StoreBillingError::InvalidPaymentChannel);

    let updated = store
        .update_payment_channel(
            &channel.id,
            UpdatePaymentChannelInput {
                adapter_kind: Some(PaymentAdapterKind::Http),
                expected_revision: channel.revision,
                name: Some("Custom backup".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.adapter_kind, PaymentAdapterKind::Http);
    assert_eq!(updated.revision, 2);

    let stale = store
        .update_payment_channel(
            &channel.id,
            UpdatePaymentChannelInput {
                expected_revision: channel.revision,
                name: Some("Stale update".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(stale, StoreBillingError::Conflict);
}

#[tokio::test]
async fn expired_codes_fail_and_one_code_can_be_redeemed_only_once_concurrently() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    insert_user(&db, "user-b").await;
    insert_user(&db, "admin").await;
    let generated = store
        .generate_redemption_codes(
            &redemption_keys(),
            "admin",
            GenerateRedemptionCodesInput {
                reward: RedemptionRewardInput::Balance {
                    currency: Currency::USD,
                    amount_minor: "100".to_string(),
                },
                count: 2,
                validity_days: 30,
            },
        )
        .await
        .unwrap();

    db.write()
        .await
        .execute(db.stmt(
            "UPDATE store_redemption_codes SET expires_at = $1 WHERE id = $2",
            vec![
                (Utc::now() - Duration::seconds(1)).to_rfc3339().into(),
                generated[0].record.id.clone().into(),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .redeem("user-a", &generated[0].code, Some(&rate()), "203.0.113.71")
            .await
            .unwrap_err(),
        StoreBillingError::RedemptionCodeExpired
    );

    let code = generated[1].code.clone();
    let first_store = store.clone();
    let second_store = store.clone();
    let first_code = code.clone();
    let (first, second) = tokio::join!(
        async move {
            first_store
                .redeem("user-a", &first_code, None, "203.0.113.72")
                .await
        },
        async move {
            second_store
                .redeem("user-b", &code, None, "203.0.113.73")
                .await
        }
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        first.err().or_else(|| second.err()).unwrap(),
        StoreBillingError::RedemptionCodeUsed
    );

    let ledger_count: i64 = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS count FROM billing_ledger WHERE idempotency_key = $1",
            vec![format!("store-redemption:{}", generated[1].record.id).into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "count")
        .unwrap();
    assert_eq!(ledger_count, 1);
    assert!(
        store
            .list_redemption_codes_admin(100)
            .await
            .unwrap()
            .iter()
            .all(|record| !serde_json::to_string(record)
                .unwrap()
                .contains(&generated[1].code))
    );
}

#[tokio::test]
async fn usd_balance_redemption_does_not_require_an_exchange_rate() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    insert_user(&db, "admin").await;
    let generated = store
        .generate_redemption_codes(
            &redemption_keys(),
            "admin",
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

    let redeemed = store
        .redeem("user-a", &generated[0].code, None, "203.0.113.74")
        .await
        .unwrap();
    assert_eq!(redeemed.status, RedemptionCodeStatus::Used);
}

#[tokio::test]
async fn plan_group_ids_are_canonical_and_validated_in_the_product_write() {
    let (db, store) = setup().await;
    pass_quota_gate(&db).await;
    let group_id = default_group_id(&db).await;
    let mut input = plan_product("Canonical groups", "2000");
    input.group_ids = vec![
        format!("  {group_id}  "),
        String::new(),
        group_id.clone(),
        "   ".to_string(),
    ];

    let product = store.create_product(input).await.unwrap();
    assert_eq!(product.group_ids, [group_id]);

    let mut invalid = plan_product("Unknown group", "2000");
    invalid.group_ids = vec!["missing-group".to_string()];
    assert_eq!(
        store.create_product(invalid).await.unwrap_err(),
        StoreBillingError::InvalidInput
    );

    let mut too_many = plan_product("Too many groups", "2000");
    too_many.group_ids = (0..33).map(|index| format!("group-{index}")).collect();
    assert_eq!(
        store.create_product(too_many).await.unwrap_err(),
        StoreBillingError::InvalidInput
    );
}

#[tokio::test]
async fn store_settings_bound_custom_recharge_on_the_new_order_path() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    let settings = StoreSettings {
        custom_recharge_cny_min_minor: "1000".to_string(),
        custom_recharge_cny_max_minor: "2000".to_string(),
        custom_recharge_usd_min_minor: "2000".to_string(),
        custom_recharge_usd_max_minor: "3000".to_string(),
    };
    assert_eq!(
        store.update_settings(settings.clone()).await.unwrap(),
        settings
    );
    let mut invalid = settings;
    invalid.custom_recharge_usd_min_minor = "3001".to_string();
    assert_eq!(
        store.update_settings(invalid).await.unwrap_err(),
        StoreBillingError::InvalidAmount
    );

    let product = store
        .create_product(balance_product("Custom USD", 0, true))
        .await
        .unwrap();
    seed_governed_stripe(&db).await;
    let orders = PaymentOrderStore::new(db);
    let request = |key: &str, amount: &str| CreatePaymentOrderInput {
        idempotency_key: key.to_string(),
        product_id: product.id.clone(),
        payment_channel_id: "store-channel-stripe".to_string(),
        payment_currency: Currency::USD,
        custom_recharge_minor: Some(amount.to_string()),
    };
    assert!(
        orders
            .create_order("user-a", request("too-small", "1999"), &rate())
            .await
            .is_err()
    );
    assert_eq!(
        orders
            .create_order("user-a", request("accepted", "2500"), &rate())
            .await
            .unwrap()
            .payment_minor,
        "2500"
    );
}

#[tokio::test]
async fn uploaded_channel_icons_require_the_same_origin_store_path() {
    let (_db, store) = setup().await;
    let mut input = payment_channel("Upload", 0, false);
    input.icon_kind = IconKind::Upload;
    input.icon_value = Some("https://example.test/icon.webp".to_string());
    assert_eq!(
        store
            .create_payment_channel(input.clone())
            .await
            .unwrap_err(),
        StoreBillingError::InvalidInput
    );

    input.icon_value = Some("/api/dashboard/store/icons/channel.webp".to_string());
    assert_eq!(
        store
            .create_payment_channel(input)
            .await
            .unwrap()
            .icon_value
            .as_deref(),
        Some("/api/dashboard/store/icons/channel.webp")
    );
}

#[tokio::test]
async fn admin_lists_include_disabled_records_and_order_references_block_deletes() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    let product = store
        .create_product(balance_product("Enabled", 20, true))
        .await
        .unwrap();
    store
        .create_product(balance_product("Disabled", 10, false))
        .await
        .unwrap();
    let removable_channel = store
        .create_payment_channel(payment_channel("Custom draft", 5, false))
        .await
        .unwrap();
    seed_governed_stripe(&db).await;
    PaymentOrderStore::new(db.clone())
        .create_order(
            "user-a",
            CreatePaymentOrderInput {
                idempotency_key: "referenced-order".to_string(),
                product_id: product.id.clone(),
                payment_channel_id: "store-channel-stripe".to_string(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &rate(),
        )
        .await
        .unwrap();

    assert_eq!(
        store
            .list_products_admin()
            .await
            .unwrap()
            .iter()
            .map(|product| product.name.as_str())
            .collect::<Vec<_>>(),
        ["Disabled", "Enabled"]
    );
    assert!(
        store
            .list_payment_channels_admin()
            .await
            .unwrap()
            .iter()
            .any(|channel| channel.name == "Custom draft")
    );
    assert_eq!(
        store.delete_product(&product.id).await.unwrap_err(),
        StoreBillingError::Conflict
    );
    assert_eq!(
        store
            .delete_payment_channel("store-channel-stripe")
            .await
            .unwrap_err(),
        StoreBillingError::Conflict
    );
    store
        .delete_payment_channel(&removable_channel.id)
        .await
        .unwrap();
    assert_eq!(
        store
            .delete_payment_channel(&removable_channel.id)
            .await
            .unwrap_err(),
        StoreBillingError::NotFound
    );
}
