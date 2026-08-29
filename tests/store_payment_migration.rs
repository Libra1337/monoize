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
    "store_settlement_reports",
    "store_settlement_lines",
    "store_redemption_limits",
    "store_redemption_attempts",
    "store_balance_holds",
    "store_reconciliation_leases",
    "store_reconciliation_cases",
    "store_fulfillment_retries",
    "store_privacy_records",
    "store_channel_readiness_profiles",
    "store_access_audits",
    "store_retention_runs",
    "store_legal_holds",
    "store_retention_state",
    "store_retention_alerts",
    "store_retention_containments",
    "store_legal_hold_approvals",
    "store_legal_hold_items",
    "store_primary_leases",
    "store_quota_gates",
    "store_quota_buckets",
    "store_quota_reservations",
    "store_admission_keys",
    "store_admission_tokens",
    "store_admission_terminal_receipts",
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
async fn migration_054_upgrades_reauth_scope_without_losing_existing_grants_or_indexes() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, Some(48)).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO store_reauth_grants
            (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
         VALUES ('legacy-grant', 'admin', 'legacy-session', 'legacy-token',
                 'credential_update', '2026-08-28T00:00:00Z', '2026-08-28T00:05:00Z')",
    )
    .await
    .unwrap();
    assert!(
        db.execute_unprepared(
            "INSERT INTO store_reauth_grants
                (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
             VALUES ('too-early', 'admin', 'session-early', 'token-early',
                     'compliance_confirm', '2026-08-28T00:00:00Z',
                     '2026-08-28T00:05:00Z')"
        )
        .await
        .is_err()
    );

    Migrator::up(&db, Some(2)).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO store_reauth_grants
            (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
         VALUES ('compliance-grant', 'admin', 'compliance-session', 'compliance-token',
                 'compliance_confirm', '2026-08-28T00:00:00Z',
                 '2026-08-28T00:05:00Z')",
    )
    .await
    .unwrap();
    assert!(
        db.execute_unprepared(
            "INSERT INTO store_reauth_grants
                (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
             VALUES ('unknown-grant', 'admin', 'unknown-session', 'unknown-token', 'unknown',
                     '2026-08-28T00:00:00Z', '2026-08-28T00:05:00Z')"
        )
        .await
        .is_err()
    );
    let rows = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id FROM store_reauth_grants ORDER BY id".to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let indexes = sqlite_names(&db, "index").await;
    for index in ["uq_store_reauth_token_digest", "idx_store_reauth_expiry"] {
        assert!(
            indexes.iter().any(|value| value == index),
            "missing {index}"
        );
    }
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

    let entitlement_columns = sqlite_columns(&db, "store_plan_entitlement_generations").await;
    for column in ["generation", "rate_numerator", "rate_denominator"] {
        assert!(
            entitlement_columns.iter().any(|value| value == column),
            "missing store_plan_entitlement_generations.{column}"
        );
    }
    let lifecycle_columns = sqlite_columns(&db, "store_plan_entitlement_lifecycle").await;
    for column in ["suspended_at", "suspension_reason", "revoked_at"] {
        assert!(
            lifecycle_columns.iter().any(|value| value == column),
            "missing store_plan_entitlement_lifecycle.{column}"
        );
    }
    assert!(
        sqlite_columns(&db, "store_plan_entitlements")
            .await
            .is_empty()
    );
    let redemption_columns = sqlite_columns(&db, "store_redemption_codes").await;
    for column in [
        "code_format_version",
        "encrypted_format_version",
        "encrypted_key_id",
        "encrypted_nonce_base64",
        "encrypted_ciphertext_base64",
        "ciphertext_destroyed_at",
        "revoked_at",
    ] {
        assert!(
            redemption_columns.iter().any(|value| value == column),
            "missing store_redemption_codes.{column}"
        );
    }

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
async fn payment_migration_preserves_legacy_orders_with_foreign_keys_enabled() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .unwrap();
    Migrator::up(&db, Some(46)).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO store_products
            (id, kind, name, description, price_currency, price_minor, duration_seconds,
             group_ids, sort_order, enabled, created_at, updated_at)
         VALUES ('legacy-product', 'balance', 'Legacy balance', '', 'CNY', '1000', NULL,
                 '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z');
         UPDATE store_payment_channels SET enabled = 1
         WHERE id IN ('store-channel-alipay', 'store-channel-wechat');
         INSERT INTO store_orders
            (id, order_number, user_id, product_id, product_kind, status,
             payment_channel_id, payment_currency, payment_minor, cny_per_usd,
             rate_source_updated_at, quote_json, created_at, updated_at,
             completed_at, cancelled_at)
         VALUES ('legacy-order', 'LS-LEGACY', 'legacy-user', 'legacy-product', 'balance',
                 'pending', 'store-channel-alipay', 'CNY', '1000', '6.7370',
                 '2026-08-27T00:00:00Z', '{}', '2026-08-27T00:00:00Z',
                 '2026-08-27T00:00:00Z', NULL, NULL)",
    )
    .await
    .unwrap();

    Migrator::up(&db, Some(1))
        .await
        .expect("migration 051 preserves referenced legacy Channels");

    let order = db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT payment_state, fulfillment_state, contract_version, payment_channel_id
             FROM store_orders WHERE id = 'legacy-order'"
                .to_string(),
        ))
        .await
        .unwrap()
        .expect("migrated legacy order");
    assert_eq!(
        String::try_get(&order, "", "payment_state").unwrap(),
        "closed"
    );
    assert_eq!(
        String::try_get(&order, "", "fulfillment_state").unwrap(),
        "pending"
    );
    assert_eq!(i64::try_get(&order, "", "contract_version").unwrap(), 1);
    assert_eq!(
        String::try_get(&order, "", "payment_channel_id").unwrap(),
        "store-channel-alipay"
    );

    let enabled = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT enabled FROM store_payment_channels ORDER BY id".to_string(),
        ))
        .await
        .unwrap();
    assert!(
        enabled
            .iter()
            .all(|row| i64::try_get(row, "", "enabled").unwrap() == 0)
    );
    assert!(
        db.query_all(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA foreign_key_check".to_string(),
        ))
        .await
        .unwrap()
        .is_empty()
    );
    let tables = sqlite_names(&db, "table").await;
    assert!(!tables.iter().any(|name| name.ends_with("_legacy")));
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
    assert!(
        indexes
            .iter()
            .any(|value| value == "idx_store_attempt_order_candidates")
    );
    let candidate_index_sql = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?",
            ["idx_store_attempt_order_candidates".into()],
        ))
        .await
        .expect("query candidate index")
        .expect("candidate index exists");
    assert!(
        String::try_get(&candidate_index_sql, "", "sql")
            .unwrap()
            .contains("(order_id, channel_id, adapter_kind, created_at DESC, id DESC)")
    );
}

#[tokio::test]
async fn admission_migration_installs_token_receipt_and_key_shape_guards() {
    let db = migrated_database().await;
    assert_eq!(
        sqlite_columns(&db, "store_admission_tokens").await,
        vec![
            "token_id",
            "audience",
            "request_id",
            "user_id",
            "effective_groups_json",
            "reservation_id",
            "entitlement_id",
            "generation",
            "maximum_nano_usd",
            "reserved_fen_cny",
            "pricing_revision",
            "key_id",
            "compact_jws",
            "compact_jws_digest",
            "issued_at",
            "expires_at",
            "expires_at_unix",
            "confirmed_at",
        ]
    );
    assert_eq!(
        sqlite_columns(&db, "store_admission_terminal_receipts").await,
        vec![
            "token_id",
            "reservation_id",
            "request_id",
            "audience",
            "terminal_kind",
            "actual_nano_usd",
            "canonical_digest",
            "applied_at",
        ]
    );
    let indexes = sqlite_names(&db, "index").await;
    for index in [
        "uq_store_admission_token_request",
        "uq_store_admission_token_reservation",
        "uq_store_admission_token_digest",
        "idx_store_admission_unconfirmed_expiry",
    ] {
        assert!(
            indexes.iter().any(|value| value == index),
            "missing {index}"
        );
    }

    assert!(
        db.execute_unprepared(
            "INSERT INTO store_admission_keys
             (key_id, public_key_base64, encrypted_private_key_json, state,
              published_at, activated_at, retired_at, last_issued_expires_at,
              verify_until, config_epoch)
             VALUES ('bad-active', 'bad', NULL, 'active',
                     '2026-08-28T00:00:00Z', NULL, NULL, NULL, NULL, 0)",
        )
        .await
        .is_err(),
        "active key shape must require encrypted seed and activation time"
    );
    assert!(
        db.execute_unprepared(
            "INSERT INTO store_admission_keys
             (key_id, public_key_base64, encrypted_private_key_json, state,
              published_at, activated_at, retired_at, last_issued_expires_at,
              verify_until, config_epoch)
             VALUES ('bad-published', 'bad', '{}', 'published',
                     '2026-08-28T00:00:00Z', '2026-08-28T00:00:00Z',
                     NULL, NULL, NULL, 0)",
        )
        .await
        .is_err(),
        "published key shape must reject activation and encrypted seed"
    );
}
