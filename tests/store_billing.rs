use chrono::{Duration, TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::{
    BalanceProductInput, CreateOrderInput, CreatePaymentChannelInput, CreateProductInput, Currency,
    ExchangeRateSnapshot, GenerateRedemptionCodesInput, IconKind, PaymentChannelKind,
    PaymentChannelMode, PlanQuotaInput, ProductKind, RedemptionCodeStatus, RedemptionRewardInput,
    StoreBillingError, StoreBillingStore, StoreSettings, UpdatePaymentChannelInput, WindowKind,
};
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

async fn setup() -> (DbPool, StoreBillingStore) {
    let db = DbPool::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    {
        let write = db.write().await;
        Migrator::up(&*write, None).await.expect("run migrations");
    }
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

fn enabled_channel(name: &str, sort_order: i32) -> CreatePaymentChannelInput {
    CreatePaymentChannelInput {
        kind: PaymentChannelKind::Custom,
        name: name.to_string(),
        mode: PaymentChannelMode::Manual,
        endpoint: None,
        icon_kind: IconKind::Builtin,
        icon_value: None,
        config_secret: None,
        sort_order,
        enabled: true,
    }
}

#[tokio::test]
async fn catalog_filters_disabled_records_and_uses_stable_order() {
    let (_db, store) = setup().await;
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
        .create_payment_channel(enabled_channel("Second channel", 20))
        .await
        .unwrap();
    let hidden_channel = store
        .create_payment_channel(enabled_channel("Hidden channel", 0))
        .await
        .unwrap();
    store
        .update_payment_channel(
            &hidden_channel.id,
            UpdatePaymentChannelInput {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    store
        .create_payment_channel(enabled_channel("First channel", 10))
        .await
        .unwrap();

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
        ["First channel", "Second channel"]
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
async fn order_quote_is_immutable_and_user_lists_are_scoped() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    insert_user(&db, "user-b").await;
    let product = store
        .create_product(balance_product("Original product", 0, true))
        .await
        .unwrap();
    let channel = store
        .create_payment_channel(enabled_channel("Original channel", 0))
        .await
        .unwrap();

    let order = store
        .create_order(
            "user-a",
            CreateOrderInput {
                product_id: product.id.clone(),
                payment_channel_id: channel.id.clone(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &rate(),
        )
        .await
        .unwrap();
    let mut changed = balance_product("Changed product", 0, true);
    changed.balance.as_mut().unwrap().bonus_minor = "900".to_string();
    store.update_product(&product.id, changed).await.unwrap();
    store
        .update_payment_channel(
            &channel.id,
            UpdatePaymentChannelInput {
                name: Some("Changed channel".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(store.list_orders_for_user("user-b", 100).await.unwrap(), []);
    let visible = store.list_orders_for_user("user-a", 100).await.unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, order.id);
    assert_eq!(visible[0].quote.product.name, "Original product");
    assert_eq!(
        visible[0]
            .quote
            .balance
            .as_ref()
            .unwrap()
            .actual_received_minor,
        "1200"
    );
    assert_eq!(visible[0].quote.payment_channel.name, "Original channel");
}

#[tokio::test]
async fn custom_recharge_bounds_are_enforced_and_remove_the_bonus() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    let product = store
        .create_product(balance_product("Custom base", 0, true))
        .await
        .unwrap();
    let channel = store
        .create_payment_channel(enabled_channel("Manual", 0))
        .await
        .unwrap();

    let too_small = store
        .create_order(
            "user-a",
            CreateOrderInput {
                product_id: product.id.clone(),
                payment_channel_id: channel.id.clone(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: Some("999".to_string()),
            },
            &rate(),
        )
        .await;
    assert_eq!(too_small.unwrap_err(), StoreBillingError::InvalidAmount);

    let order = store
        .create_order(
            "user-a",
            CreateOrderInput {
                product_id: product.id,
                payment_channel_id: channel.id,
                payment_currency: Currency::CNY,
                custom_recharge_minor: Some("1500".to_string()),
            },
            &rate(),
        )
        .await
        .unwrap();
    let balance = order.quote.balance.unwrap();
    assert_eq!(balance.recharge_minor, "1500");
    assert_eq!(balance.bonus_minor, "0");
    assert_eq!(balance.actual_received_minor, "1500");
    assert_eq!(order.payment_minor, "1500");
}

#[tokio::test]
async fn balance_completion_and_cancellation_are_idempotent() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    let product = store
        .create_product(balance_product("Recharge", 0, true))
        .await
        .unwrap();
    let channel = store
        .create_payment_channel(enabled_channel("Manual", 0))
        .await
        .unwrap();

    let create = || CreateOrderInput {
        product_id: product.id.clone(),
        payment_channel_id: channel.id.clone(),
        payment_currency: Currency::CNY,
        custom_recharge_minor: None,
    };
    let completed_id = store
        .create_order("user-a", create(), &rate())
        .await
        .unwrap()
        .id;
    store.complete_order(&completed_id).await.unwrap();
    store.complete_order(&completed_id).await.unwrap();

    let balance: String = db
        .read()
        .query_one(db.stmt(
            "SELECT balance_nano_usd FROM users WHERE id = $1",
            vec!["user-a".into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "balance_nano_usd")
        .unwrap();
    assert_eq!(balance, "2000000000");
    let ledger_count: i64 = db
        .read()
        .query_one(db.stmt(
            "SELECT COUNT(*) AS count FROM billing_ledger WHERE idempotency_key = $1",
            vec![format!("store-order:{completed_id}").into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "count")
        .unwrap();
    assert_eq!(ledger_count, 1);

    let cancelled_id = store
        .create_order("user-a", create(), &rate())
        .await
        .unwrap()
        .id;
    store.cancel_order(&cancelled_id).await.unwrap();
    store.cancel_order(&cancelled_id).await.unwrap();
    assert_eq!(
        store.complete_order(&cancelled_id).await.unwrap_err(),
        StoreBillingError::OrderCancelled
    );
}

#[tokio::test]
async fn plan_completion_uses_the_order_snapshot_and_replaces_the_current_entitlement() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    let channel = store
        .create_payment_channel(enabled_channel("Manual", 0))
        .await
        .unwrap();
    let first = store
        .create_product(plan_product("First plan", "2000"))
        .await
        .unwrap();
    let first_order = store
        .create_order(
            "user-a",
            CreateOrderInput {
                product_id: first.id.clone(),
                payment_channel_id: channel.id.clone(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &rate(),
        )
        .await
        .unwrap();
    store
        .update_product(&first.id, plan_product("Edited plan", "9900"))
        .await
        .unwrap();
    store.complete_order(&first_order.id).await.unwrap();
    let first_entitlement = store.current_entitlement("user-a").await.unwrap().unwrap();
    assert_eq!(first_entitlement.product_name, "First plan");
    assert_eq!(first_entitlement.quotas[0].quota_fen_cny, "2000");

    let second = store
        .create_product(plan_product("Second plan", "6800"))
        .await
        .unwrap();
    let second_order = store
        .create_order(
            "user-a",
            CreateOrderInput {
                product_id: second.id,
                payment_channel_id: channel.id,
                payment_currency: Currency::USD,
                custom_recharge_minor: None,
            },
            &rate(),
        )
        .await
        .unwrap();
    store.complete_order(&second_order.id).await.unwrap();
    let current = store.current_entitlement("user-a").await.unwrap().unwrap();
    assert_eq!(current.product_name, "Second plan");
    assert_eq!(current.quotas[0].quota_fen_cny, "6800");
    assert_eq!(current.source_id, second_order.id);
}

#[tokio::test]
async fn expired_codes_fail_and_one_code_can_be_redeemed_only_once_concurrently() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    insert_user(&db, "user-b").await;
    insert_user(&db, "admin").await;
    let generated = store
        .generate_redemption_codes(
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
    assert_eq!(generated.len(), 2);

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
            .redeem("user-a", &generated[0].code, Some(&rate()))
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
                .redeem("user-a", &first_code, Some(&rate()))
                .await
        },
        async move { second_store.redeem("user-b", &code, Some(&rate())).await }
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let error = first.err().or_else(|| second.err()).unwrap();
    assert_eq!(error, StoreBillingError::RedemptionCodeUsed);

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
        .redeem("user-a", &generated[0].code, None)
        .await
        .unwrap();
    assert_eq!(redeemed.status, RedemptionCodeStatus::Used);
}

#[tokio::test]
async fn plan_group_ids_are_canonical_and_validated_in_the_product_write() {
    let (db, store) = setup().await;
    let group_id = default_group_id(&db).await;
    let mut input = plan_product("Canonical groups", "2000");
    input.group_ids = vec![
        format!("  {group_id}  "),
        String::new(),
        group_id.clone(),
        "   ".to_string(),
    ];

    let product = store.create_product(input).await.unwrap();
    assert_eq!(product.group_ids, [group_id.clone()]);

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
async fn store_settings_default_validate_and_bound_custom_recharge_by_currency() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    assert_eq!(
        store.get_settings().await.unwrap(),
        StoreSettings {
            custom_recharge_cny_min_minor: "1000".to_string(),
            custom_recharge_cny_max_minor: "100000000".to_string(),
            custom_recharge_usd_min_minor: "1000".to_string(),
            custom_recharge_usd_max_minor: "100000000".to_string(),
        }
    );

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
    let mut invalid = settings.clone();
    invalid.custom_recharge_usd_min_minor = "3001".to_string();
    assert_eq!(
        store.update_settings(invalid).await.unwrap_err(),
        StoreBillingError::InvalidAmount
    );

    let product = store
        .create_product(balance_product("Custom USD", 0, true))
        .await
        .unwrap();
    let channel = store
        .create_payment_channel(enabled_channel("Manual", 0))
        .await
        .unwrap();
    let request = |amount: &str| CreateOrderInput {
        product_id: product.id.clone(),
        payment_channel_id: channel.id.clone(),
        payment_currency: Currency::USD,
        custom_recharge_minor: Some(amount.to_string()),
    };
    assert_eq!(
        store
            .create_order("user-a", request("1999"), &rate())
            .await
            .unwrap_err(),
        StoreBillingError::InvalidAmount
    );
    assert_eq!(
        store
            .create_order("user-a", request("2500"), &rate())
            .await
            .unwrap()
            .payment_minor,
        "2500"
    );
}

#[tokio::test]
async fn order_creation_distinguishes_no_enabled_channel_from_invalid_selection() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    let product = store
        .create_product(balance_product("Recharge", 0, true))
        .await
        .unwrap();
    let request = |channel_id: &str| CreateOrderInput {
        product_id: product.id.clone(),
        payment_channel_id: channel_id.to_string(),
        payment_currency: Currency::CNY,
        custom_recharge_minor: None,
    };
    assert_eq!(
        store
            .create_order("user-a", request("missing"), &rate())
            .await
            .unwrap_err(),
        StoreBillingError::NoPaymentChannel
    );

    store
        .create_payment_channel(enabled_channel("Enabled", 0))
        .await
        .unwrap();
    assert_eq!(
        store
            .create_order("user-a", request("missing"), &rate())
            .await
            .unwrap_err(),
        StoreBillingError::InvalidPaymentChannel
    );
}

#[tokio::test]
async fn uploaded_channel_icons_require_the_same_origin_store_path() {
    let (_db, store) = setup().await;
    let mut input = enabled_channel("Upload", 0);
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
async fn admin_lists_include_disabled_records_and_deletes_report_missing_or_in_use() {
    let (db, store) = setup().await;
    insert_user(&db, "user-a").await;
    let enabled_product = store
        .create_product(balance_product("Enabled", 20, true))
        .await
        .unwrap();
    store
        .create_product(balance_product("Disabled", 10, false))
        .await
        .unwrap();
    let active_channel = store
        .create_payment_channel(enabled_channel("Enabled channel", 20))
        .await
        .unwrap();
    let mut disabled_channel_input = enabled_channel("Disabled channel", 5);
    disabled_channel_input.enabled = false;
    store
        .create_payment_channel(disabled_channel_input)
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
    assert_eq!(
        store
            .list_payment_channels_admin()
            .await
            .unwrap()
            .iter()
            .map(|channel| channel.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Disabled channel",
            "Alipay",
            "WeChat Pay",
            "Enabled channel"
        ]
    );

    let order = store
        .create_order(
            "user-a",
            CreateOrderInput {
                product_id: enabled_product.id.clone(),
                payment_channel_id: active_channel.id.clone(),
                payment_currency: Currency::CNY,
                custom_recharge_minor: None,
            },
            &rate(),
        )
        .await
        .unwrap();
    assert_eq!(store.list_orders_admin(100).await.unwrap()[0].id, order.id);
    assert_eq!(
        store.delete_product(&enabled_product.id).await.unwrap_err(),
        StoreBillingError::Conflict
    );
    assert_eq!(
        store
            .delete_payment_channel(&active_channel.id)
            .await
            .unwrap_err(),
        StoreBillingError::Conflict
    );
    assert_eq!(
        store.delete_product("missing").await.unwrap_err(),
        StoreBillingError::NotFound
    );
    assert_eq!(
        store.delete_payment_channel("missing").await.unwrap_err(),
        StoreBillingError::NotFound
    );
}
