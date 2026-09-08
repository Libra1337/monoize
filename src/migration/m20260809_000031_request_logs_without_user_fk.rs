use std::collections::BTreeSet;

use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(Clone, Copy)]
struct RequestLogColumn {
    name: &'static str,
    sqlite_definition: &'static str,
    postgres_definition: &'static str,
    required: bool,
}

const REQUEST_LOG_COLUMNS: [RequestLogColumn; 42] = [
    RequestLogColumn {
        name: "id",
        sqlite_definition: "TEXT NOT NULL PRIMARY KEY",
        postgres_definition: "TEXT",
        required: true,
    },
    RequestLogColumn {
        name: "request_id",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "user_id",
        sqlite_definition: "TEXT NOT NULL",
        postgres_definition: "TEXT",
        required: true,
    },
    RequestLogColumn {
        name: "api_key_id",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "model",
        sqlite_definition: "TEXT NOT NULL",
        postgres_definition: "TEXT",
        required: true,
    },
    RequestLogColumn {
        name: "provider_id",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "upstream_model",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "channel_id",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "is_stream",
        sqlite_definition: "INTEGER NOT NULL DEFAULT 0",
        postgres_definition: "INTEGER",
        required: true,
    },
    RequestLogColumn {
        name: "input_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "output_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "cache_read_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "cache_creation_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "tool_prompt_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "reasoning_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "accepted_prediction_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "rejected_prediction_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "provider_multiplier",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "charge_nano_usd",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "status",
        sqlite_definition: "TEXT NOT NULL",
        postgres_definition: "TEXT",
        required: true,
    },
    RequestLogColumn {
        name: "usage_breakdown_json",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "billing_breakdown_json",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "error_code",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "error_message",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "error_http_status",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "duration_ms",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "ttfb_ms",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "first_visible_output_ms",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "last_visible_output_ms",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "visible_generation_ms",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "visible_output_tokens",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
    RequestLogColumn {
        name: "tps_mode",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "request_ip",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "reasoning_effort",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "tried_providers_json",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "request_kind",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "effective_provider_type",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "affinity_hit",
        sqlite_definition: "INTEGER",
        postgres_definition: "INTEGER",
        required: false,
    },
    RequestLogColumn {
        name: "affinity_key_hash",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "affinity_target",
        sqlite_definition: "TEXT",
        postgres_definition: "TEXT",
        required: false,
    },
    RequestLogColumn {
        name: "created_at",
        sqlite_definition: "TEXT NOT NULL",
        postgres_definition: "TEXT",
        required: true,
    },
    RequestLogColumn {
        name: "created_at_unix_ms",
        sqlite_definition: "INTEGER",
        postgres_definition: "BIGINT",
        required: false,
    },
];

const LEGACY_TOKEN_COLUMNS: [(&str, &str); 3] = [
    ("input_tokens", "prompt_tokens"),
    ("output_tokens", "completion_tokens"),
    ("cache_read_tokens", "cached_tokens"),
];

const REQUEST_LOG_INDEX_SQL: [&str; 4] = [
    "CREATE INDEX idx_request_logs_user_created_at ON request_logs (user_id, created_at_unix_ms DESC)",
    "CREATE INDEX idx_request_logs_created_at ON request_logs (created_at_unix_ms DESC)",
    "CREATE INDEX idx_request_logs_model ON request_logs (model)",
    "CREATE INDEX idx_request_logs_legacy_created_at ON request_logs (created_at) WHERE created_at_unix_ms IS NULL",
];

const POSTGRES_DROP_USER_CONSTRAINTS: [&str; 2] = [
    "ALTER TABLE request_logs DROP CONSTRAINT IF EXISTS request_logs_user_id_fkey",
    "ALTER TABLE request_logs DROP CONSTRAINT IF EXISTS fk_request_logs_user_id",
];

const POSTGRES_ORDINARY_INDEX_DROP_QUERY: &str = r#"
SELECT format('DROP INDEX IF EXISTS %I.%I', ns.nspname, index_class.relname) AS drop_sql
FROM pg_class AS table_class
JOIN pg_namespace AS ns ON ns.oid = table_class.relnamespace
JOIN pg_index AS index_meta ON index_meta.indrelid = table_class.oid
JOIN pg_class AS index_class ON index_class.oid = index_meta.indexrelid
LEFT JOIN pg_constraint AS constraint_meta ON constraint_meta.conindid = index_class.oid
WHERE ns.nspname = current_schema()
  AND table_class.relname = 'request_logs'
  AND table_class.relkind IN ('r', 'p')
  AND constraint_meta.oid IS NULL
ORDER BY index_class.relname
"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }

        let tx = manager.get_connection().begin().await?;
        let result = match backend {
            DbBackend::Sqlite => migrate_sqlite(&tx).await,
            DbBackend::Postgres => migrate_postgres(&tx).await,
            _ => unreachable!(),
        };
        if let Err(error) = result {
            let _ = tx.rollback().await;
            return Err(error);
        }
        tx.commit().await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

async fn migrate_sqlite(tx: &DatabaseTransaction) -> Result<(), DbErr> {
    let source_columns = request_log_columns(tx, DbBackend::Sqlite).await?;
    validate_required_columns(&source_columns)?;

    let definitions = REQUEST_LOG_COLUMNS
        .iter()
        .map(|column| {
            format!(
                "{} {}",
                quote_identifier(column.name),
                column.sqlite_definition
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let target_columns = REQUEST_LOG_COLUMNS
        .iter()
        .map(|column| quote_identifier(column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let projections = REQUEST_LOG_COLUMNS
        .iter()
        .map(|column| source_projection(column.name, &source_columns))
        .collect::<Vec<_>>()
        .join(", ");

    let mut statements = vec![
        "DROP TABLE IF EXISTS request_logs_without_user_fk".to_string(),
        format!("CREATE TABLE request_logs_without_user_fk ({definitions})"),
        format!(
            "INSERT INTO request_logs_without_user_fk ({target_columns}) SELECT {projections} FROM request_logs"
        ),
        "DROP TABLE request_logs".to_string(),
        "ALTER TABLE request_logs_without_user_fk RENAME TO request_logs".to_string(),
    ];
    statements.extend(REQUEST_LOG_INDEX_SQL.iter().map(|sql| (*sql).to_string()));
    execute_statements(tx, DbBackend::Sqlite, statements).await
}

async fn migrate_postgres(tx: &DatabaseTransaction) -> Result<(), DbErr> {
    let source_columns = request_log_columns(tx, DbBackend::Postgres).await?;
    let plan = postgres_plan(&source_columns)?;

    execute_statements(tx, DbBackend::Postgres, plan.prepare).await?;

    let index_drop_statements = tx
        .query_all(Statement::from_string(
            DbBackend::Postgres,
            POSTGRES_ORDINARY_INDEX_DROP_QUERY.to_string(),
        ))
        .await?
        .into_iter()
        .map(|row| row.try_get::<String>("", "drop_sql"))
        .collect::<Result<Vec<_>, _>>()?;
    execute_statements(tx, DbBackend::Postgres, index_drop_statements).await?;
    execute_statements(tx, DbBackend::Postgres, plan.drop_columns).await?;
    execute_statements(
        tx,
        DbBackend::Postgres,
        REQUEST_LOG_INDEX_SQL
            .iter()
            .map(|sql| (*sql).to_string())
            .collect(),
    )
    .await
}

struct PostgresPlan {
    prepare: Vec<String>,
    drop_columns: Vec<String>,
}

fn postgres_plan(source_columns: &BTreeSet<String>) -> Result<PostgresPlan, DbErr> {
    validate_required_columns(source_columns)?;
    let canonical_columns = canonical_column_names();
    let mut prepare = POSTGRES_DROP_USER_CONSTRAINTS
        .iter()
        .map(|sql| (*sql).to_string())
        .collect::<Vec<_>>();

    for column in REQUEST_LOG_COLUMNS {
        if !source_columns.contains(column.name) {
            prepare.push(format!(
                "ALTER TABLE request_logs ADD COLUMN {} {}",
                quote_identifier(column.name),
                column.postgres_definition
            ));
        }
    }

    for (canonical, legacy) in LEGACY_TOKEN_COLUMNS {
        if source_columns.contains(legacy) {
            prepare.push(format!(
                "UPDATE request_logs SET {canonical} = COALESCE({canonical}, {legacy}) WHERE {canonical} IS NULL AND {legacy} IS NOT NULL",
                canonical = quote_identifier(canonical),
                legacy = quote_identifier(legacy),
            ));
        }
    }

    let drop_columns = source_columns
        .difference(&canonical_columns)
        .map(|column| {
            format!(
                "ALTER TABLE request_logs DROP COLUMN {}",
                quote_identifier(column)
            )
        })
        .collect();

    Ok(PostgresPlan {
        prepare,
        drop_columns,
    })
}

fn validate_required_columns(source_columns: &BTreeSet<String>) -> Result<(), DbErr> {
    let missing = REQUEST_LOG_COLUMNS
        .iter()
        .filter(|column| column.required && !source_columns.contains(column.name))
        .map(|column| column.name)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(DbErr::Custom(format!(
            "request_logs is missing required canonical columns: {}",
            missing.join(", ")
        )))
    }
}

fn source_projection(column: &str, source_columns: &BTreeSet<String>) -> String {
    let legacy = LEGACY_TOKEN_COLUMNS
        .iter()
        .find_map(|(canonical, legacy)| (*canonical == column).then_some(*legacy));
    match (source_columns.contains(column), legacy) {
        (true, Some(legacy)) if source_columns.contains(legacy) => format!(
            "COALESCE({}, {})",
            quote_identifier(column),
            quote_identifier(legacy)
        ),
        (true, _) => quote_identifier(column),
        (false, Some(legacy)) if source_columns.contains(legacy) => quote_identifier(legacy),
        (false, _) => "NULL".to_string(),
    }
}

fn canonical_column_names() -> BTreeSet<String> {
    REQUEST_LOG_COLUMNS
        .iter()
        .map(|column| column.name.to_string())
        .collect()
}

async fn request_log_columns(
    tx: &DatabaseTransaction,
    backend: DbBackend,
) -> Result<BTreeSet<String>, DbErr> {
    let sql = match backend {
        DbBackend::Sqlite => "PRAGMA table_info(request_logs)",
        DbBackend::Postgres => {
            "SELECT column_name AS name FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'request_logs' ORDER BY ordinal_position"
        }
        _ => return Ok(BTreeSet::new()),
    };
    Ok(tx
        .query_all(Statement::from_string(backend, sql.to_string()))
        .await?
        .into_iter()
        .map(|row| row.try_get::<String>("", "name"))
        .collect::<Result<_, _>>()?)
}

async fn execute_statements(
    tx: &DatabaseTransaction,
    backend: DbBackend,
    statements: Vec<String>,
) -> Result<(), DbErr> {
    for sql in statements {
        tx.execute(Statement::from_string(backend, sql)).await?;
    }
    Ok(())
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, QueryResult};

    const EXPECTED_SQLITE_SCHEMA: [(&str, &str, i64, Option<&str>, i64); 42] = [
        ("id", "TEXT", 1, None, 1),
        ("request_id", "TEXT", 0, None, 0),
        ("user_id", "TEXT", 1, None, 0),
        ("api_key_id", "TEXT", 0, None, 0),
        ("model", "TEXT", 1, None, 0),
        ("provider_id", "TEXT", 0, None, 0),
        ("upstream_model", "TEXT", 0, None, 0),
        ("channel_id", "TEXT", 0, None, 0),
        ("is_stream", "INTEGER", 1, Some("0"), 0),
        ("input_tokens", "INTEGER", 0, None, 0),
        ("output_tokens", "INTEGER", 0, None, 0),
        ("cache_read_tokens", "INTEGER", 0, None, 0),
        ("cache_creation_tokens", "INTEGER", 0, None, 0),
        ("tool_prompt_tokens", "INTEGER", 0, None, 0),
        ("reasoning_tokens", "INTEGER", 0, None, 0),
        ("accepted_prediction_tokens", "INTEGER", 0, None, 0),
        ("rejected_prediction_tokens", "INTEGER", 0, None, 0),
        ("provider_multiplier", "TEXT", 0, None, 0),
        ("charge_nano_usd", "TEXT", 0, None, 0),
        ("status", "TEXT", 1, None, 0),
        ("usage_breakdown_json", "TEXT", 0, None, 0),
        ("billing_breakdown_json", "TEXT", 0, None, 0),
        ("error_code", "TEXT", 0, None, 0),
        ("error_message", "TEXT", 0, None, 0),
        ("error_http_status", "INTEGER", 0, None, 0),
        ("duration_ms", "INTEGER", 0, None, 0),
        ("ttfb_ms", "INTEGER", 0, None, 0),
        ("first_visible_output_ms", "INTEGER", 0, None, 0),
        ("last_visible_output_ms", "INTEGER", 0, None, 0),
        ("visible_generation_ms", "INTEGER", 0, None, 0),
        ("visible_output_tokens", "INTEGER", 0, None, 0),
        ("tps_mode", "TEXT", 0, None, 0),
        ("request_ip", "TEXT", 0, None, 0),
        ("reasoning_effort", "TEXT", 0, None, 0),
        ("tried_providers_json", "TEXT", 0, None, 0),
        ("request_kind", "TEXT", 0, None, 0),
        ("effective_provider_type", "TEXT", 0, None, 0),
        ("affinity_hit", "INTEGER", 0, None, 0),
        ("affinity_key_hash", "TEXT", 0, None, 0),
        ("affinity_target", "TEXT", 0, None, 0),
        ("created_at", "TEXT", 1, None, 0),
        ("created_at_unix_ms", "INTEGER", 0, None, 0),
    ];

    async fn sqlite() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.expect("SQLite connects");
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .expect("foreign keys enable");
        db
    }

    fn full_canonical_columns() -> BTreeSet<String> {
        EXPECTED_SQLITE_SCHEMA
            .iter()
            .map(|(name, ..)| (*name).to_string())
            .collect()
    }

    fn required_columns() -> BTreeSet<String> {
        [
            "id",
            "user_id",
            "model",
            "is_stream",
            "status",
            "created_at",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    async fn create_source_schema(
        db: &DatabaseConnection,
        canonical_columns: &BTreeSet<String>,
        include_legacy_tokens: bool,
        include_user_fk: bool,
        include_indexes: bool,
    ) {
        if include_user_fk {
            db.execute_unprepared("CREATE TABLE users (id TEXT NOT NULL PRIMARY KEY)")
                .await
                .expect("users table creates");
        }
        let mut definitions = REQUEST_LOG_COLUMNS
            .iter()
            .filter(|column| canonical_columns.contains(column.name))
            .map(|column| format!("{} {}", column.name, column.sqlite_definition))
            .collect::<Vec<_>>();
        if include_legacy_tokens {
            definitions.extend([
                "prompt_tokens INTEGER".to_string(),
                "completion_tokens INTEGER".to_string(),
                "cached_tokens INTEGER".to_string(),
            ]);
        }
        if include_user_fk {
            definitions.push(
                "CONSTRAINT fk_request_logs_user_id FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE"
                    .to_string(),
            );
        }
        db.execute_unprepared(&format!(
            "CREATE TABLE request_logs ({})",
            definitions.join(", ")
        ))
        .await
        .expect("source request_logs table creates");
        if include_indexes {
            for index in REQUEST_LOG_INDEX_SQL {
                db.execute_unprepared(index)
                    .await
                    .expect("canonical request-log index creates");
            }
            db.execute_unprepared(
                "CREATE INDEX idx_request_logs_user ON request_logs (user_id, created_at DESC)",
            )
            .await
            .expect("legacy request-log index creates");
        }
    }

    async fn canonical_row_json(db: &DatabaseConnection, id: &str) -> String {
        let columns = EXPECTED_SQLITE_SCHEMA
            .iter()
            .map(|(name, ..)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        db.query_one(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            format!("SELECT json_array({columns}) AS row_json FROM request_logs WHERE id = ?"),
            [id.into()],
        ))
        .await
        .expect("request-log row queries")
        .expect("request-log row exists")
        .try_get("", "row_json")
        .expect("request-log row JSON decodes")
    }

    fn string_column(row: &QueryResult, column: &str) -> String {
        row.try_get("", column).expect("string column decodes")
    }

    async fn assert_canonical_sqlite_schema(db: &DatabaseConnection) {
        let actual = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA table_info(request_logs)".to_string(),
            ))
            .await
            .expect("table columns query")
            .iter()
            .map(|row| {
                (
                    string_column(row, "name"),
                    string_column(row, "type"),
                    row.try_get::<i64>("", "notnull").unwrap(),
                    row.try_get::<Option<String>>("", "dflt_value").unwrap(),
                    row.try_get::<i64>("", "pk").unwrap(),
                )
            })
            .collect::<Vec<_>>();
        let expected = EXPECTED_SQLITE_SCHEMA
            .iter()
            .map(|(name, data_type, not_null, default, primary_key)| {
                (
                    (*name).to_string(),
                    (*data_type).to_string(),
                    *not_null,
                    default.map(str::to_string),
                    *primary_key,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    async fn assert_canonical_indexes(db: &DatabaseConnection) {
        let indexes = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'request_logs' AND name NOT LIKE 'sqlite_autoindex_%' ORDER BY name".to_string(),
            ))
            .await
            .expect("indexes query")
            .iter()
            .map(|row| string_column(row, "name"))
            .collect::<Vec<_>>();
        assert_eq!(
            indexes,
            vec![
                "idx_request_logs_created_at",
                "idx_request_logs_legacy_created_at",
                "idx_request_logs_model",
                "idx_request_logs_user_created_at",
            ]
        );
    }

    async fn assert_no_foreign_keys(db: &DatabaseConnection) {
        let foreign_keys = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA foreign_key_list(request_logs)".to_string(),
            ))
            .await
            .expect("foreign keys query");
        assert!(foreign_keys.is_empty());
    }

    #[test]
    fn postgres_plan_handles_fresh_dual_and_legacy_token_schemas() {
        let fresh = postgres_plan(&full_canonical_columns()).expect("fresh plan builds");
        assert_eq!(fresh.prepare, POSTGRES_DROP_USER_CONSTRAINTS);
        assert!(fresh.drop_columns.is_empty());

        let mut dual = full_canonical_columns();
        dual.extend(
            ["prompt_tokens", "completion_tokens", "cached_tokens"]
                .into_iter()
                .map(str::to_string),
        );
        let dual = postgres_plan(&dual).expect("dual plan builds");
        let prepare = dual.prepare.join("; ");
        assert!(prepare.contains("COALESCE(\"input_tokens\", \"prompt_tokens\")"));
        assert!(prepare.contains("COALESCE(\"output_tokens\", \"completion_tokens\")"));
        assert!(prepare.contains("COALESCE(\"cache_read_tokens\", \"cached_tokens\")"));
        assert_eq!(dual.drop_columns.len(), 3);

        let mut legacy = required_columns();
        legacy.extend(
            ["prompt_tokens", "completion_tokens", "cached_tokens"]
                .into_iter()
                .map(str::to_string),
        );
        let legacy = postgres_plan(&legacy).expect("legacy plan builds");
        let prepare = legacy.prepare.join("; ");
        assert!(prepare.contains("ADD COLUMN \"input_tokens\" BIGINT"));
        assert!(prepare.contains("ADD COLUMN \"request_id\" TEXT"));
        assert!(prepare.contains("COALESCE(\"input_tokens\", \"prompt_tokens\")"));
        assert_eq!(legacy.drop_columns.len(), 3);
        assert!(POSTGRES_ORDINARY_INDEX_DROP_QUERY.contains("constraint_meta.oid IS NULL"));
    }

    #[tokio::test]
    async fn sqlite_dual_schema_prefers_canonical_values_and_is_idempotent() {
        let db = sqlite().await;
        create_source_schema(&db, &full_canonical_columns(), true, true, true).await;
        db.execute_unprepared("INSERT INTO users (id) VALUES ('deleted-user')")
            .await
            .expect("user inserts");
        let columns = EXPECTED_SQLITE_SCHEMA
            .iter()
            .map(|(name, ..)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        db.execute_unprepared(&format!(
            r#"INSERT INTO request_logs ({columns}, prompt_tokens, completion_tokens, cached_tokens) VALUES (
                'log-1', 'request-1', 'deleted-user', 'key-1', 'model-1', 'provider-1',
                'upstream-1', 'channel-1', 1, 101, 102, 103, 104, 105, 106, 107, 108,
                '1.25', '123456789012345678901', 'error', '{{"input":101}}',
                '{{"charge":"123456789012345678901"}}', 'upstream_error', 'sentinel error',
                502, 201, 202, 203, 204, 1, 205, 'exact', '203.0.113.7', 'high',
                '[{{"provider_id":"provider-0"}}]', 'client', 'responses', 1, 'affinity-hash',
                'provider-1/channel-1', '2026-08-09T12:34:56Z', 1786278896000,
                901, 902, 903
            )"#
        ))
        .await
        .expect("request log inserts");
        db.execute_unprepared(
            "INSERT INTO request_logs (id, user_id, model, is_stream, status, created_at, prompt_tokens, completion_tokens, cached_tokens) VALUES ('log-null-canonical', 'deleted-user', 'model-2', 0, 'success', '2026-08-09T12:34:57Z', 401, 402, 403)",
        )
        .await
        .expect("null canonical request log inserts");
        let before = canonical_row_json(&db, "log-1").await;

        for _ in 0..2 {
            Migration
                .up(&SchemaManager::new(&db))
                .await
                .expect("migration succeeds");
            assert_eq!(canonical_row_json(&db, "log-1").await, before);
            assert_canonical_sqlite_schema(&db).await;
            assert_canonical_indexes(&db).await;
            assert_no_foreign_keys(&db).await;
            let fallback = db
                .query_one(Statement::from_string(
                    DbBackend::Sqlite,
                    "SELECT input_tokens, output_tokens, cache_read_tokens FROM request_logs WHERE id = 'log-null-canonical'".to_string(),
                ))
                .await
                .expect("fallback values query")
                .expect("fallback values exist");
            assert_eq!(fallback.try_get::<i64>("", "input_tokens").unwrap(), 401);
            assert_eq!(fallback.try_get::<i64>("", "output_tokens").unwrap(), 402);
            assert_eq!(
                fallback.try_get::<i64>("", "cache_read_tokens").unwrap(),
                403
            );
        }

        db.execute_unprepared("DELETE FROM users WHERE id = 'deleted-user'")
            .await
            .expect("user deletes");
        db.execute_unprepared(
            "INSERT INTO request_logs (id, user_id, model, is_stream, status, created_at) VALUES ('orphan-log', 'missing-user', 'model-2', 0, 'error', '2026-08-09T12:35:00Z')",
        )
        .await
        .expect("orphan request log inserts");
        let count: i64 = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS count FROM request_logs".to_string(),
            ))
            .await
            .expect("row count queries")
            .expect("row count exists")
            .try_get("", "count")
            .expect("row count decodes");
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn sqlite_legacy_only_tokens_backfill_and_missing_nullable_columns_become_null() {
        let db = sqlite().await;
        create_source_schema(&db, &required_columns(), true, false, false).await;
        db.execute_unprepared(
            "INSERT INTO request_logs (id, user_id, model, is_stream, status, created_at, prompt_tokens, completion_tokens, cached_tokens) VALUES ('legacy-log', 'gone-user', 'legacy-model', 1, 'success', '2026-08-09T00:00:00Z', 301, 302, 303)",
        )
        .await
        .expect("legacy request log inserts");

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("legacy migration succeeds");

        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT input_tokens, output_tokens, cache_read_tokens, request_id, tps_mode FROM request_logs WHERE id = 'legacy-log'".to_string(),
            ))
            .await
            .expect("migrated values query")
            .expect("migrated values exist");
        assert_eq!(row.try_get::<i64>("", "input_tokens").unwrap(), 301);
        assert_eq!(row.try_get::<i64>("", "output_tokens").unwrap(), 302);
        assert_eq!(row.try_get::<i64>("", "cache_read_tokens").unwrap(), 303);
        assert!(
            row.try_get::<Option<String>>("", "request_id")
                .unwrap()
                .is_none()
        );
        assert!(
            row.try_get::<Option<String>>("", "tps_mode")
                .unwrap()
                .is_none()
        );
        assert_canonical_sqlite_schema(&db).await;
        assert_canonical_indexes(&db).await;
    }

    #[tokio::test]
    async fn sqlite_fresh_42_column_schema_remains_unchanged() {
        let db = sqlite().await;
        create_source_schema(&db, &full_canonical_columns(), false, false, true).await;
        db.execute_unprepared(
            "INSERT INTO request_logs (id, user_id, model, is_stream, provider_multiplier, status, created_at, created_at_unix_ms) VALUES ('fresh-log', 'fresh-user', 'fresh-model', 0, '2.5', 'success', '2026-08-09T00:00:00Z', 1786233600000)",
        )
        .await
        .expect("fresh request log inserts");
        let before = canonical_row_json(&db, "fresh-log").await;

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("fresh migration succeeds");

        assert_eq!(canonical_row_json(&db, "fresh-log").await, before);
        assert_canonical_sqlite_schema(&db).await;
        assert_canonical_indexes(&db).await;
        assert_no_foreign_keys(&db).await;
    }

    #[tokio::test]
    async fn sqlite_missing_required_column_rolls_back_without_changes() {
        let db = sqlite().await;
        let mut missing_status = required_columns();
        missing_status.remove("status");
        create_source_schema(&db, &missing_status, true, false, false).await;
        db.execute_unprepared(
            "INSERT INTO request_logs (id, user_id, model, is_stream, created_at, prompt_tokens) VALUES ('log-1', 'user-1', 'model-1', 0, '2026-08-09T00:00:00Z', 7)",
        )
        .await
        .expect("incomplete row inserts");

        assert!(Migration.up(&SchemaManager::new(&db)).await.is_err());

        let tables = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'request_logs%' ORDER BY name".to_string(),
            ))
            .await
            .expect("tables query")
            .iter()
            .map(|row| string_column(row, "name"))
            .collect::<Vec<_>>();
        assert_eq!(tables, vec!["request_logs"]);
        let columns = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA table_info(request_logs)".to_string(),
            ))
            .await
            .expect("original columns query")
            .iter()
            .map(|row| string_column(row, "name"))
            .collect::<Vec<_>>();
        assert!(!columns.iter().any(|column| column == "status"));
        assert!(columns.iter().any(|column| column == "prompt_tokens"));
        let id: String = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT id FROM request_logs".to_string(),
            ))
            .await
            .expect("original row queries")
            .expect("original row exists")
            .try_get("", "id")
            .expect("original id decodes");
        assert_eq!(id, "log-1");
    }
}
