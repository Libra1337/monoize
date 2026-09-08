use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }

        let tx = manager.get_connection().begin().await?;
        for sql in up_statements(backend) {
            tx.execute(Statement::from_string(backend, sql)).await?;
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }

        let tx = manager.get_connection().begin().await?;
        for table in [
            "store_redemption_codes",
            "store_plan_entitlement_current",
            "store_plan_entitlement_lifecycle",
            "store_plan_entitlement_generations",
            "store_orders",
            "store_plan_quotas",
            "store_balance_products",
            "store_payment_channels",
            "store_products",
            "store_exchange_rates",
        ] {
            tx.execute(Statement::from_string(
                backend,
                format!("DROP TABLE IF EXISTS {table}"),
            ))
            .await?;
        }
        if backend == DbBackend::Postgres {
            tx.execute(Statement::from_string(
                backend,
                "DROP FUNCTION IF EXISTS store_guard_entitlement_generation_immutable()"
                    .to_string(),
            ))
            .await?;
        }
        tx.commit().await
    }
}

fn canonical_positive(column: &str, backend: DbBackend) -> String {
    match backend {
        DbBackend::Postgres => format!("{column} ~ '^[1-9][0-9]*$'"),
        _ => format!("{column} <> '' AND {column} NOT GLOB '*[^0-9]*' AND {column} NOT LIKE '0%'"),
    }
}

fn canonical_non_negative(column: &str, backend: DbBackend) -> String {
    match backend {
        DbBackend::Postgres => format!("{column} ~ '^(0|[1-9][0-9]*)$'"),
        _ => format!(
            "{column} = '0' OR ({column} <> '' AND {column} NOT GLOB '*[^0-9]*' AND {column} NOT LIKE '0%')"
        ),
    }
}

fn positive_decimal(column: &str, backend: DbBackend) -> String {
    match backend {
        DbBackend::Postgres => format!("{column} ~ '^[0-9]+(\\.[0-9]+)?$' AND {column} ~ '[1-9]'"),
        _ => format!(
            "{column} <> '' AND {column} NOT GLOB '*[^0-9.]*' AND {column} NOT LIKE '%.%.%' AND {column} NOT LIKE '.%' AND {column} NOT LIKE '%.' AND {column} GLOB '*[1-9]*'"
        ),
    }
}

fn up_statements(backend: DbBackend) -> Vec<String> {
    let price_positive = canonical_positive("price_minor", backend);
    let recharge_non_negative = canonical_non_negative("recharge_minor", backend);
    let bonus_non_negative = canonical_non_negative("bonus_minor", backend);
    let quota_positive = canonical_positive("quota_fen_cny", backend);
    let payment_positive = canonical_positive("payment_minor", backend);
    let rate_positive = positive_decimal("cny_per_usd", backend);
    let numerator_positive = canonical_positive("rate_numerator", backend);
    let denominator_positive = canonical_positive("rate_denominator", backend);
    let digest_check = match backend {
        DbBackend::Postgres => "code_digest ~ '^[0-9a-f]{64}$'",
        _ => "length(code_digest) = 64 AND code_digest NOT GLOB '*[^0-9a-f]*'",
    };

    let mut statements = vec![
        format!(
            "CREATE TABLE IF NOT EXISTS store_exchange_rates (base_currency TEXT NOT NULL, quote_currency TEXT NOT NULL, cny_per_usd TEXT NOT NULL, source_updated_at TEXT NOT NULL, refreshed_at TEXT NOT NULL, PRIMARY KEY (base_currency, quote_currency), CONSTRAINT ck_store_exchange_rates_pair CHECK (base_currency = 'USD' AND quote_currency = 'CNY'), CONSTRAINT ck_store_exchange_rates_positive CHECK ({rate_positive}))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS store_products (id TEXT NOT NULL PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', price_currency TEXT NOT NULL, price_minor TEXT NOT NULL, duration_seconds BIGINT, group_ids TEXT NOT NULL DEFAULT '[]', sort_order INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, CONSTRAINT ck_store_products_kind CHECK (kind IN ('balance', 'plan')), CONSTRAINT ck_store_products_name CHECK (length(trim(name)) BETWEEN 1 AND 100), CONSTRAINT ck_store_products_description CHECK (length(trim(description)) <= 500), CONSTRAINT ck_store_products_currency CHECK (price_currency IN ('CNY', 'USD')), CONSTRAINT ck_store_products_price CHECK ({price_positive}), CONSTRAINT ck_store_products_duration CHECK ((kind = 'balance' AND duration_seconds IS NULL AND group_ids = '[]') OR (kind = 'plan' AND duration_seconds BETWEEN 3600 AND 31536000)), CONSTRAINT ck_store_products_enabled CHECK (enabled IN (0, 1)))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS store_balance_products (product_id TEXT NOT NULL PRIMARY KEY, recharge_minor TEXT NOT NULL, bonus_minor TEXT NOT NULL, CONSTRAINT fk_store_balance_products_product FOREIGN KEY (product_id) REFERENCES store_products (id) ON DELETE CASCADE, CONSTRAINT ck_store_balance_products_recharge CHECK ({recharge_non_negative}), CONSTRAINT ck_store_balance_products_bonus CHECK ({bonus_non_negative}))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS store_plan_quotas (id TEXT NOT NULL PRIMARY KEY, product_id TEXT NOT NULL, window_kind TEXT NOT NULL, window_seconds BIGINT NOT NULL, quota_fen_cny TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0, CONSTRAINT fk_store_plan_quotas_product FOREIGN KEY (product_id) REFERENCES store_products (id) ON DELETE CASCADE, CONSTRAINT ck_store_plan_quotas_window_kind CHECK (window_kind IN ('5h', '12h', 'day', 'week', 'month', 'custom')), CONSTRAINT ck_store_plan_quotas_window CHECK ((window_kind = '5h' AND window_seconds = 18000) OR (window_kind = '12h' AND window_seconds = 43200) OR (window_kind = 'day' AND window_seconds = 86400) OR (window_kind = 'week' AND window_seconds = 604800) OR (window_kind = 'month' AND window_seconds = 2592000) OR (window_kind = 'custom' AND window_seconds BETWEEN 3600 AND 31536000 AND window_seconds % 3600 = 0)), CONSTRAINT ck_store_plan_quotas_amount CHECK ({quota_positive}))"
        ),
        "CREATE TABLE IF NOT EXISTS store_payment_channels (id TEXT NOT NULL PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, mode TEXT NOT NULL, endpoint TEXT, icon_kind TEXT NOT NULL, icon_value TEXT, config_secret TEXT, sort_order INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, CONSTRAINT ck_store_payment_channels_kind CHECK (kind IN ('alipay', 'wechat', 'custom')), CONSTRAINT ck_store_payment_channels_name CHECK (length(trim(name)) BETWEEN 1 AND 80), CONSTRAINT ck_store_payment_channels_mode CHECK (mode IN ('redirect', 'qr', 'manual')), CONSTRAINT ck_store_payment_channels_icon_kind CHECK (icon_kind IN ('builtin', 'url', 'upload')), CONSTRAINT ck_store_payment_channels_icon_url CHECK (icon_kind <> 'url' OR (icon_value IS NOT NULL AND icon_value LIKE 'https://%')), CONSTRAINT ck_store_payment_channels_enabled CHECK (enabled IN (0, 1)))".to_string(),
        format!(
            "CREATE TABLE IF NOT EXISTS store_orders (id TEXT NOT NULL PRIMARY KEY, order_number TEXT NOT NULL, user_id TEXT NOT NULL, product_id TEXT NOT NULL, product_kind TEXT NOT NULL, status TEXT NOT NULL, payment_channel_id TEXT NOT NULL, payment_currency TEXT NOT NULL, payment_minor TEXT NOT NULL, cny_per_usd TEXT NOT NULL, rate_source_updated_at TEXT NOT NULL, quote_json TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, completed_at TEXT, cancelled_at TEXT, CONSTRAINT fk_store_orders_product FOREIGN KEY (product_id) REFERENCES store_products (id) ON DELETE RESTRICT, CONSTRAINT fk_store_orders_channel FOREIGN KEY (payment_channel_id) REFERENCES store_payment_channels (id) ON DELETE RESTRICT, CONSTRAINT ck_store_orders_product_kind CHECK (product_kind IN ('balance', 'plan')), CONSTRAINT ck_store_orders_status CHECK (status IN ('pending', 'completed', 'cancelled')), CONSTRAINT ck_store_orders_currency CHECK (payment_currency IN ('CNY', 'USD')), CONSTRAINT ck_store_orders_payment CHECK ({payment_positive}), CONSTRAINT ck_store_orders_rate CHECK ({rate_positive}), CONSTRAINT ck_store_orders_state_time CHECK ((status = 'pending' AND completed_at IS NULL AND cancelled_at IS NULL) OR (status = 'completed' AND completed_at IS NOT NULL AND cancelled_at IS NULL) OR (status = 'cancelled' AND completed_at IS NULL AND cancelled_at IS NOT NULL)))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS store_plan_entitlement_generations (
                id TEXT NOT NULL PRIMARY KEY,
                user_id TEXT NOT NULL,
                generation BIGINT NOT NULL,
                product_id TEXT NOT NULL,
                product_name TEXT NOT NULL,
                starts_at TEXT NOT NULL,
                ends_at TEXT NOT NULL,
                rate_numerator TEXT NOT NULL,
                rate_denominator TEXT NOT NULL,
                group_ids TEXT NOT NULL,
                quota_json TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE (id, generation),
                UNIQUE (id, user_id, generation),
                CONSTRAINT ck_store_plan_entitlement_generation CHECK (generation > 0),
                CONSTRAINT ck_store_plan_entitlement_name CHECK (length(trim(product_name)) BETWEEN 1 AND 100),
                CONSTRAINT ck_store_plan_entitlement_time CHECK (ends_at > starts_at),
                CONSTRAINT ck_store_plan_entitlement_numerator CHECK ({numerator_positive}),
                CONSTRAINT ck_store_plan_entitlement_denominator CHECK ({denominator_positive}),
                CONSTRAINT ck_store_plan_entitlement_source_kind CHECK (source_kind IN ('order', 'redemption')),
                CONSTRAINT fk_store_plan_entitlement_user
                    FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE RESTRICT,
                CONSTRAINT fk_store_plan_entitlement_product
                    FOREIGN KEY (product_id) REFERENCES store_products (id) ON DELETE RESTRICT
            )"
        ),
        "CREATE TABLE IF NOT EXISTS store_plan_entitlement_current (
            user_id TEXT NOT NULL PRIMARY KEY,
            entitlement_id TEXT NOT NULL UNIQUE,
            generation BIGINT NOT NULL CHECK (generation > 0),
            updated_at TEXT NOT NULL,
            CONSTRAINT fk_store_plan_entitlement_current_generation
                FOREIGN KEY (entitlement_id, user_id, generation)
                REFERENCES store_plan_entitlement_generations (id, user_id, generation)
                ON DELETE RESTRICT
        )"
        .to_string(),
        "CREATE TABLE IF NOT EXISTS store_plan_entitlement_lifecycle (
            entitlement_id TEXT NOT NULL PRIMARY KEY,
            suspended_at TEXT,
            suspension_reason TEXT,
            revoked_at TEXT,
            revocation_reason TEXT,
            updated_at TEXT NOT NULL,
            CONSTRAINT fk_store_plan_entitlement_lifecycle_generation
                FOREIGN KEY (entitlement_id) REFERENCES store_plan_entitlement_generations (id)
                ON DELETE RESTRICT,
            CONSTRAINT ck_store_plan_entitlement_suspension CHECK (
                (suspended_at IS NULL AND suspension_reason IS NULL) OR
                (suspended_at IS NOT NULL AND suspension_reason IS NOT NULL)
            ),
            CONSTRAINT ck_store_plan_entitlement_revocation CHECK (
                (revoked_at IS NULL AND revocation_reason IS NULL) OR
                (revoked_at IS NOT NULL AND revocation_reason IS NOT NULL)
            )
        )"
        .to_string(),
        format!(
            "CREATE TABLE IF NOT EXISTS store_redemption_codes (id TEXT NOT NULL PRIMARY KEY, code_digest TEXT NOT NULL, code_hint TEXT NOT NULL, reward_kind TEXT NOT NULL, reward_json TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'unused', expires_at TEXT NOT NULL, redeemed_by_user_id TEXT, redeemed_at TEXT, created_by_user_id TEXT NOT NULL, created_at TEXT NOT NULL, CONSTRAINT ck_store_redemption_codes_digest CHECK ({digest_check}), CONSTRAINT ck_store_redemption_codes_hint CHECK (length(code_hint) = 4), CONSTRAINT ck_store_redemption_codes_reward_kind CHECK (reward_kind IN ('balance', 'plan')), CONSTRAINT ck_store_redemption_codes_status CHECK (status IN ('unused', 'used')), CONSTRAINT ck_store_redemption_codes_state CHECK ((status = 'unused' AND redeemed_by_user_id IS NULL AND redeemed_at IS NULL) OR (status = 'used' AND redeemed_by_user_id IS NOT NULL AND redeemed_at IS NOT NULL)))"
        ),
        "CREATE INDEX IF NOT EXISTS idx_store_products_catalog ON store_products (enabled, sort_order, created_at, id)".to_string(),
        "CREATE INDEX IF NOT EXISTS idx_store_plan_quotas_product ON store_plan_quotas (product_id, sort_order, id)".to_string(),
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_store_plan_quotas_product_window ON store_plan_quotas (product_id, window_seconds)".to_string(),
        "CREATE INDEX IF NOT EXISTS idx_store_payment_channels_catalog ON store_payment_channels (enabled, sort_order, created_at, id)".to_string(),
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_store_orders_order_number ON store_orders (order_number)".to_string(),
        "CREATE INDEX IF NOT EXISTS idx_store_orders_user_created ON store_orders (user_id, created_at DESC, id DESC)".to_string(),
        "CREATE INDEX IF NOT EXISTS idx_store_orders_status_created ON store_orders (status, created_at DESC, id DESC)".to_string(),
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_store_plan_entitlement_user_generation ON store_plan_entitlement_generations (user_id, generation)".to_string(),
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_store_plan_entitlement_source ON store_plan_entitlement_generations (source_kind, source_id)".to_string(),
        "CREATE INDEX IF NOT EXISTS idx_store_plan_entitlement_user_time ON store_plan_entitlement_generations (user_id, ends_at, generation)".to_string(),
        "CREATE UNIQUE INDEX IF NOT EXISTS uq_store_redemption_codes_digest ON store_redemption_codes (code_digest)".to_string(),
        "CREATE INDEX IF NOT EXISTS idx_store_redemption_codes_status_expires ON store_redemption_codes (status, expires_at, id)".to_string(),
        "INSERT INTO store_payment_channels (id, kind, name, mode, endpoint, icon_kind, icon_value, config_secret, sort_order, enabled, created_at, updated_at) VALUES ('store-channel-alipay', 'alipay', 'Alipay', 'manual', NULL, 'builtin', 'alipay', NULL, 10, 0, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z') ON CONFLICT (id) DO NOTHING".to_string(),
        "INSERT INTO store_payment_channels (id, kind, name, mode, endpoint, icon_kind, icon_value, config_secret, sort_order, enabled, created_at, updated_at) VALUES ('store-channel-wechat', 'wechat', 'WeChat Pay', 'manual', NULL, 'builtin', 'wechat', NULL, 20, 0, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z') ON CONFLICT (id) DO NOTHING".to_string(),
    ];
    statements.extend(match backend {
        DbBackend::Sqlite => vec![
            "CREATE TRIGGER trg_store_plan_entitlement_generation_no_update
             BEFORE UPDATE ON store_plan_entitlement_generations
             BEGIN SELECT RAISE(ABORT, 'immutable entitlement generation'); END"
                .to_string(),
            "CREATE TRIGGER trg_store_plan_entitlement_generation_no_delete
             BEFORE DELETE ON store_plan_entitlement_generations
             BEGIN SELECT RAISE(ABORT, 'immutable entitlement generation'); END"
                .to_string(),
        ],
        DbBackend::Postgres => vec![
            "CREATE FUNCTION store_guard_entitlement_generation_immutable()
             RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN RAISE EXCEPTION 'immutable entitlement generation'; END $$"
                .to_string(),
            "CREATE TRIGGER trg_store_plan_entitlement_generation_no_update
             BEFORE UPDATE OR DELETE ON store_plan_entitlement_generations
             FOR EACH ROW EXECUTE FUNCTION store_guard_entitlement_generation_immutable()"
                .to_string(),
        ],
        _ => Vec::new(),
    });
    statements
}

#[cfg(test)]
mod tests {
    use super::{Migration, up_statements};
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
    use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};

    const STORE_TABLES: &[&str] = &[
        "store_exchange_rates",
        "store_products",
        "store_balance_products",
        "store_plan_quotas",
        "store_payment_channels",
        "store_orders",
        "store_plan_entitlement_generations",
        "store_plan_entitlement_current",
        "store_plan_entitlement_lifecycle",
        "store_redemption_codes",
    ];

    const STORE_INDEXES: &[&str] = &[
        "idx_store_products_catalog",
        "idx_store_plan_quotas_product",
        "uq_store_plan_quotas_product_window",
        "idx_store_payment_channels_catalog",
        "uq_store_orders_order_number",
        "idx_store_orders_user_created",
        "idx_store_orders_status_created",
        "uq_store_plan_entitlement_user_generation",
        "uq_store_plan_entitlement_source",
        "idx_store_plan_entitlement_user_time",
        "uq_store_redemption_codes_digest",
        "idx_store_redemption_codes_status_expires",
    ];

    async fn migrated_database() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect SQLite");
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .expect("enable foreign keys");
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply Store migration");
        db
    }

    async fn sqlite_object_names(db: &DatabaseConnection, object_type: &str) -> Vec<String> {
        db.query_all(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = ? ORDER BY name",
            [object_type.into()],
        ))
        .await
        .expect("query SQLite schema")
        .into_iter()
        .map(|row| row.try_get::<String>("", "name").expect("schema name"))
        .collect()
    }

    #[tokio::test]
    async fn migration_creates_store_tables_indexes_and_disabled_builtin_channels() {
        let db = migrated_database().await;
        let tables = sqlite_object_names(&db, "table").await;
        let indexes = sqlite_object_names(&db, "index").await;

        for table in STORE_TABLES {
            assert!(tables.iter().any(|name| name == table), "missing {table}");
        }
        for index in STORE_INDEXES {
            assert!(indexes.iter().any(|name| name == index), "missing {index}");
        }

        let channels = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT kind, enabled FROM store_payment_channels ORDER BY kind".to_string(),
            ))
            .await
            .expect("query seeded channels");
        let seeded = channels
            .into_iter()
            .map(|row| {
                (
                    row.try_get::<String>("", "kind").expect("channel kind"),
                    row.try_get::<i64>("", "enabled").expect("channel enabled"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            seeded,
            vec![("alipay".to_string(), 0), ("wechat".to_string(), 0)]
        );
    }

    #[tokio::test]
    async fn products_reject_unknown_kind_and_currency() {
        let db = migrated_database().await;

        for (kind, currency) in [("unknown", "CNY"), ("balance", "EUR")] {
            let sql = format!(
                "INSERT INTO store_products (id, kind, name, description, price_currency, price_minor, duration_seconds, group_ids, sort_order, enabled, created_at, updated_at) VALUES ('product-{kind}-{currency}', '{kind}', 'Product', '', '{currency}', '1000', NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')"
            );
            assert!(
                db.execute_unprepared(&sql).await.is_err(),
                "{kind}/{currency} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn url_payment_channel_rejects_missing_icon_value() {
        let db = migrated_database().await;

        let result = db
            .execute_unprepared(
                "INSERT INTO store_payment_channels (id, kind, name, mode, endpoint, icon_kind, icon_value, config_secret, sort_order, enabled, created_at, updated_at) VALUES ('custom-url', 'custom', 'Custom', 'redirect', 'https://pay.example.test', 'url', NULL, NULL, 30, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            )
            .await;

        assert!(result.is_err(), "URL icons require a non-null HTTPS value");
    }

    #[tokio::test]
    async fn order_numbers_and_redemption_digests_are_unique() {
        let db = migrated_database().await;
        db.execute_unprepared(
            "INSERT INTO store_products (id, kind, name, description, price_currency, price_minor, duration_seconds, group_ids, sort_order, enabled, created_at, updated_at) VALUES ('product-1', 'balance', 'Balance', '', 'CNY', '1000', NULL, '[]', 0, 1, '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
        )
        .await
        .expect("insert product");

        for id in ["order-1", "order-2"] {
            let result = db
                .execute_unprepared(&format!(
                    "INSERT INTO store_orders (id, order_number, user_id, product_id, product_kind, status, payment_channel_id, payment_currency, payment_minor, cny_per_usd, rate_source_updated_at, quote_json, created_at, updated_at, completed_at, cancelled_at) VALUES ('{id}', 'LS-0001', 'user-1', 'product-1', 'balance', 'pending', 'store-channel-alipay', 'CNY', '1000', '6.7370', '2026-08-27T00:00:00Z', '{{}}', '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z', NULL, NULL)"
                ))
                .await;
            if id == "order-1" {
                result.expect("insert first order");
            } else {
                assert!(result.is_err(), "duplicate order number must fail");
            }
        }

        for id in ["code-1", "code-2"] {
            let result = db
                .execute_unprepared(&format!(
                    "INSERT INTO store_redemption_codes (id, code_digest, code_hint, reward_kind, reward_json, status, expires_at, redeemed_by_user_id, redeemed_at, created_by_user_id, created_at) VALUES ('{id}', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'AAAA', 'balance', '{{}}', 'unused', '2026-09-27T00:00:00Z', NULL, NULL, 'admin-1', '2026-08-27T00:00:00Z')"
                ))
                .await;
            if id == "code-1" {
                result.expect("insert first redemption code");
            } else {
                assert!(result.is_err(), "duplicate code digest must fail");
            }
        }
    }

    #[tokio::test]
    async fn down_removes_store_tables() {
        let db = migrated_database().await;
        Migration
            .down(&SchemaManager::new(&db))
            .await
            .expect("revert Store migration");
        let tables = sqlite_object_names(&db, "table").await;

        for table in STORE_TABLES {
            assert!(
                tables.iter().all(|name| name != table),
                "{table} remains after down"
            );
        }
    }

    #[tokio::test]
    async fn embedded_migrator_registers_store_schema() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect SQLite");
        crate::migration::Migrator::up(&db, None)
            .await
            .expect("apply embedded migrations");
        let tables = sqlite_object_names(&db, "table").await;

        for table in STORE_TABLES {
            assert!(tables.iter().any(|name| name == table), "missing {table}");
        }
    }

    #[test]
    fn postgres_ddl_uses_postgres_constraints_for_every_store_table() {
        let statements = up_statements(DbBackend::Postgres);
        let creates = statements
            .iter()
            .filter(|statement| statement.starts_with("CREATE TABLE"))
            .count();

        assert_eq!(creates, STORE_TABLES.len());
        assert!(
            statements
                .iter()
                .any(|statement| statement.contains("~ '^[1-9][0-9]*$'"))
        );
        assert!(
            statements
                .iter()
                .all(|statement| !statement.contains("GLOB"))
        );
    }
}
