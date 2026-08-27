use monoize::migration::Migrator;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, TryGetable};
use sea_orm_migration::MigratorTrait;

const PAYMENT_TABLES: &[&str] = &[
    "store_channel_credentials",
    "store_reauth_grants",
    "store_payment_compliance",
    "store_merchant_capabilities",
    "store_payment_attempts",
    "store_provider_events",
    "store_order_event_applications",
    "store_refunds",
    "store_order_reward_recoveries",
    "store_order_recovery_claims",
    "store_balance_holds",
    "store_reconciliation_leases",
    "store_reconciliation_cases",
    "store_fulfillment_retries",
    "store_privacy_records",
    "store_access_audits",
    "store_retention_runs",
    "store_legal_holds",
    "store_primary_leases",
    "store_quota_gates",
    "store_quota_buckets",
    "store_quota_reservations",
    "store_admission_keys",
];

async fn migrated_database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    db.execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .expect("enable foreign keys");
    Migrator::up(&db, None).await.expect("run migrations");
    db
}

async fn sqlite_names(db: &DatabaseConnection, object_type: &str) -> Vec<String> {
    db.query_all(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "SELECT name FROM sqlite_master WHERE type = ? ORDER BY name",
        [object_type.into()],
    ))
    .await
    .expect("query SQLite schema")
    .into_iter()
    .map(|row| String::try_get(&row, "", "name").expect("schema name"))
    .collect()
}

async fn sqlite_columns(db: &DatabaseConnection, table: &str) -> Vec<String> {
    db.query_all(Statement::from_string(
        DbBackend::Sqlite,
        format!("PRAGMA table_info({table})"),
    ))
    .await
    .expect("query SQLite columns")
    .into_iter()
    .map(|row| String::try_get(&row, "", "name").expect("column name"))
    .collect()
}

#[tokio::test]
async fn payment_migration_replaces_legacy_store_shape() {
    let db = migrated_database().await;
    let tables = sqlite_names(&db, "table").await;
    for table in PAYMENT_TABLES {
        assert!(tables.iter().any(|value| value == table), "missing {table}");
    }

    let order_columns = sqlite_columns(&db, "store_orders").await;
    for column in [
        "payment_state",
        "fulfillment_state",
        "dispute_state",
        "payment_hold",
        "contract_version",
        "state_revision",
        "expires_at",
        "rate_numerator",
        "rate_denominator",
    ] {
        assert!(
            order_columns.iter().any(|value| value == column),
            "missing store_orders.{column}"
        );
    }
    for obsolete in ["status", "completed_at", "cancelled_at"] {
        assert!(
            !order_columns.iter().any(|value| value == obsolete),
            "obsolete store_orders.{obsolete} remains"
        );
    }

    let channel_columns = sqlite_columns(&db, "store_payment_channels").await;
    for obsolete in ["kind", "mode", "endpoint", "config_secret"] {
        assert!(
            !channel_columns.iter().any(|value| value == obsolete),
            "obsolete store_payment_channels.{obsolete} remains"
        );
    }
    assert!(channel_columns.iter().any(|value| value == "adapter_kind"));

    let channels = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT adapter_kind, enabled FROM store_payment_channels ORDER BY adapter_kind"
                .to_string(),
        ))
        .await
        .expect("query built-in Channels")
        .into_iter()
        .map(|row| {
            (
                String::try_get(&row, "", "adapter_kind").expect("adapter kind"),
                i64::try_get(&row, "", "enabled").expect("enabled"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        channels,
        vec![
            ("alipay".to_string(), 0),
            ("stripe".to_string(), 0),
            ("wechat".to_string(), 0),
        ]
    );
}

#[tokio::test]
async fn payment_migration_installs_transition_and_recovery_guards() {
    let db = migrated_database().await;
    let triggers = sqlite_names(&db, "trigger").await;
    for trigger in [
        "trg_store_orders_payment_transition",
        "trg_store_orders_fulfillment_transition",
        "trg_store_orders_quote_immutable",
        "trg_store_recovery_insert_limit",
        "trg_store_recovery_update_limit",
    ] {
        assert!(
            triggers.iter().any(|value| value == trigger),
            "missing {trigger}"
        );
    }

    db.execute_unprepared(
        "INSERT INTO store_products
         (id, kind, name, description, price_currency, price_minor, duration_seconds,
          group_ids, sort_order, enabled, revision, created_at, updated_at)
         VALUES
         ('product-1', 'balance', 'Guard product', '', 'CNY', '1000', NULL,
          '[]', 0, 1, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
    )
    .await
    .expect("insert guarded product");

    db.execute_unprepared(
        "INSERT INTO store_orders
         (id, order_number, user_id, product_id, product_kind, payment_state,
          fulfillment_state, dispute_state, payment_hold, payment_channel_id,
          payment_currency, payment_minor, cny_per_usd, rate_numerator,
          rate_denominator, rate_source_updated_at, quote_json, contract_version,
          state_revision, expires_at, created_at, updated_at)
         VALUES
         ('order-guard', 'LS-GUARD', 'user-1', 'product-1', 'balance', 'unpaid',
          'pending', 'none', 0, 'store-channel-alipay', 'CNY', '1000', '6.7',
          '67', '10', '2026-08-27T00:00:00Z', '{}', 2, 0,
          '2026-08-27T00:30:00Z', '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
    )
    .await
    .expect("insert guarded order");

    assert!(
        db.execute_unprepared(
            "UPDATE store_orders SET payment_state = 'refunded', state_revision = 1
             WHERE id = 'order-guard'",
        )
        .await
        .is_err(),
        "unpaid to refunded must fail"
    );
    assert!(
        db.execute_unprepared(
            "UPDATE store_orders SET quote_json = '{\"changed\":true}'
             WHERE id = 'order-guard'",
        )
        .await
        .is_err(),
        "immutable quote must reject update"
    );
}

#[tokio::test]
async fn reauth_migration_allows_only_one_active_credential_per_channel() {
    let db = migrated_database().await;
    db.execute_unprepared(
        "INSERT INTO store_channel_credentials
         (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
          ciphertext_base64, account_identity_digest, status, created_at)
         VALUES
         ('credential-active-1', 'store-channel-stripe', 'stripe', 1, 'key-1',
          'nonce-1', 'ciphertext-1', 'account-1', 'active', '2026-08-27T00:00:00Z')",
    )
    .await
    .expect("insert first active credential");

    assert!(
        db.execute_unprepared(
            "INSERT INTO store_channel_credentials
             (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
              ciphertext_base64, account_identity_digest, status, created_at)
             VALUES
             ('credential-active-2', 'store-channel-stripe', 'stripe', 1, 'key-1',
              'nonce-2', 'ciphertext-2', 'account-1', 'active', '2026-08-27T00:00:01Z')",
        )
        .await
        .is_err(),
        "a Channel must not have two active credential versions"
    );

    db.execute_unprepared(
        "INSERT INTO store_channel_credentials
         (id, channel_id, adapter_kind, format_version, key_id, nonce_base64,
          ciphertext_base64, account_identity_digest, status, created_at, retired_at)
         VALUES
         ('credential-retired', 'store-channel-stripe', 'stripe', 1, 'key-1',
          'nonce-3', 'ciphertext-3', 'account-1', 'retired',
          '2026-08-27T00:00:02Z', '2026-08-27T00:00:03Z')",
    )
    .await
    .expect("retired credential versions remain allowed");
}

#[tokio::test]
async fn reconciliation_migration_adds_bounded_fulfillment_retry_state() {
    let db = migrated_database().await;
    let columns = sqlite_columns(&db, "store_fulfillment_retries").await;
    assert_eq!(
        columns,
        vec![
            "order_id",
            "attempt_count",
            "next_attempt_at",
            "last_error_category",
            "updated_at",
        ]
    );
    let indexes = sqlite_names(&db, "index").await;
    assert!(
        indexes
            .iter()
            .any(|value| value == "idx_store_fulfillment_retries_due")
    );
}
