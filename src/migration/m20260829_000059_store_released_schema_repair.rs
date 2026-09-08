use crate::store_billing::money::ExchangeRateRational;
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbBackend, QueryResult, Statement, TransactionTrait,
    TryGetable,
};
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

        let legacy = manager.has_table("store_plan_entitlements").await?;
        let generations = manager
            .has_table("store_plan_entitlement_generations")
            .await?;
        let current = manager.has_table("store_plan_entitlement_current").await?;
        let lifecycle = manager
            .has_table("store_plan_entitlement_lifecycle")
            .await?;
        let repair_legacy = match (legacy, generations, current, lifecycle) {
            (true, false, false, false) => true,
            (false, true, true, true) => false,
            _ => {
                return Err(DbErr::Custom(
                    "store_entitlement_schema_is_partial_or_mixed".to_string(),
                ));
            }
        };

        let tx = manager.get_connection().begin().await?;
        if repair_legacy {
            let (create_statements, finish_statements) = entitlement_repair_statements(backend);
            for sql in create_statements {
                tx.execute(Statement::from_string(backend, sql)).await?;
            }
            migrate_legacy_entitlements(&tx, backend).await?;
            for sql in finish_statements {
                tx.execute(Statement::from_string(backend, sql)).await?;
            }
        }
        tx.execute(Statement::from_string(
            backend,
            normalize_order_expiry_statement(backend),
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

fn entitlement_repair_statements(backend: DbBackend) -> (Vec<String>, Vec<String>) {
    let positive_numerator = canonical_positive("rate_numerator", backend);
    let positive_denominator = canonical_positive("rate_denominator", backend);
    let create_statements = vec![
        format!(
            "CREATE TABLE store_plan_entitlement_generations (
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
                CONSTRAINT ck_store_plan_entitlement_name
                    CHECK (length(trim(product_name)) BETWEEN 1 AND 100),
                CONSTRAINT ck_store_plan_entitlement_time CHECK (ends_at > starts_at),
                CONSTRAINT ck_store_plan_entitlement_numerator CHECK ({positive_numerator}),
                CONSTRAINT ck_store_plan_entitlement_denominator CHECK ({positive_denominator}),
                CONSTRAINT ck_store_plan_entitlement_source_kind
                    CHECK (source_kind IN ('order', 'redemption')),
                CONSTRAINT fk_store_plan_entitlement_user
                    FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE RESTRICT,
                CONSTRAINT fk_store_plan_entitlement_product
                    FOREIGN KEY (product_id) REFERENCES store_products (id) ON DELETE RESTRICT
            )"
        ),
        "CREATE TABLE store_plan_entitlement_current (
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
        "CREATE TABLE store_plan_entitlement_lifecycle (
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
    ];
    let mut finish_statements = vec![
        "INSERT INTO store_plan_entitlement_current
            (user_id, entitlement_id, generation, updated_at)
         SELECT user_id, id, 1, starts_at FROM store_plan_entitlements"
            .to_string(),
        "INSERT INTO store_plan_entitlement_lifecycle
            (entitlement_id, suspended_at, suspension_reason, revoked_at,
             revocation_reason, updated_at)
         SELECT id, NULL, NULL, NULL, NULL, starts_at FROM store_plan_entitlements"
            .to_string(),
        "DROP TABLE store_plan_entitlements".to_string(),
        "CREATE UNIQUE INDEX uq_store_plan_entitlement_user_generation
            ON store_plan_entitlement_generations (user_id, generation)"
            .to_string(),
        "CREATE UNIQUE INDEX uq_store_plan_entitlement_source
            ON store_plan_entitlement_generations (source_kind, source_id)"
            .to_string(),
        "CREATE INDEX idx_store_plan_entitlement_user_time
            ON store_plan_entitlement_generations (user_id, ends_at, generation)"
            .to_string(),
    ];
    finish_statements.extend(match backend {
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
    (create_statements, finish_statements)
}

fn canonical_positive(column: &str, backend: DbBackend) -> String {
    match backend {
        DbBackend::Postgres => format!("{column} ~ '^[1-9][0-9]*$'"),
        _ => format!("{column} <> '' AND {column} NOT GLOB '*[^0-9]*' AND {column} NOT LIKE '0%'"),
    }
}

async fn migrate_legacy_entitlements(
    tx: &DatabaseTransaction,
    backend: DbBackend,
) -> Result<(), DbErr> {
    let rows = tx
        .query_all(Statement::from_string(
            backend,
            "SELECT id, user_id, product_id, product_name, starts_at, ends_at,
                    cny_per_usd, group_ids, quota_json, source_kind, source_id
             FROM store_plan_entitlements
             ORDER BY id"
                .to_string(),
        ))
        .await?;
    let placeholders = match backend {
        DbBackend::Postgres => "$1, $2, 1, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $5",
        _ => "?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?5",
    };
    let insert_sql = format!(
        "INSERT INTO store_plan_entitlement_generations
            (id, user_id, generation, product_id, product_name, starts_at, ends_at,
             rate_numerator, rate_denominator, group_ids, quota_json, source_kind,
             source_id, created_at)
         VALUES ({placeholders})"
    );
    for row in rows {
        let rate_decimal = legacy_string(&row, "cny_per_usd")?;
        let rate = ExchangeRateRational::parse(&rate_decimal)
            .map_err(|_| DbErr::Custom("store_entitlement_exchange_rate_invalid".to_string()))?;
        tx.execute(Statement::from_sql_and_values(
            backend,
            insert_sql.clone(),
            [
                legacy_string(&row, "id")?.into(),
                legacy_string(&row, "user_id")?.into(),
                legacy_string(&row, "product_id")?.into(),
                legacy_string(&row, "product_name")?.into(),
                legacy_string(&row, "starts_at")?.into(),
                legacy_string(&row, "ends_at")?.into(),
                rate.numerator().to_string().into(),
                rate.denominator().to_string().into(),
                legacy_string(&row, "group_ids")?.into(),
                legacy_string(&row, "quota_json")?.into(),
                legacy_string(&row, "source_kind")?.into(),
                legacy_string(&row, "source_id")?.into(),
            ],
        ))
        .await?;
    }
    Ok(())
}

fn legacy_string(row: &QueryResult, column: &str) -> Result<String, DbErr> {
    String::try_get(row, "", column).map_err(|error| {
        DbErr::Custom(format!(
            "store_entitlement_legacy_column_invalid:{column}:{error:?}"
        ))
    })
}

fn normalize_order_expiry_statement(backend: DbBackend) -> String {
    match backend {
        DbBackend::Postgres => "UPDATE store_orders
            SET expires_at = to_char(
                expires_at::timestamptz AT TIME ZONE 'UTC',
                'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'
            )
            WHERE substring(expires_at FROM 11 FOR 1) = ' '"
            .to_string(),
        _ => "UPDATE store_orders
            SET expires_at = strftime('%Y-%m-%dT%H:%M:%SZ', expires_at)
            WHERE length(expires_at) = 19 AND substr(expires_at, 11, 1) = ' '"
            .to_string(),
    }
}
