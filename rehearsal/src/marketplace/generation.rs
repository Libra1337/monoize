use sqlx::{Connection, Executor, Row, SqliteConnection};

const FULL_SOURCES: &[&str] = &[
    "monoize_groups",
    "monoize_providers",
    "monoize_provider_models",
    "billing_rate_records",
    "model_metadata_records",
];

pub async fn create_sqlite_generation_schema(
    db: &mut SqliteConnection,
    generated_at_unix_us: i64,
) -> Result<(), sqlx::Error> {
    let mut transaction = db.begin().await?;
    transaction
        .execute(
            r#"CREATE TABLE marketplace_generation (
                singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
                revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9223372036854775807),
                generated_at_unix_us INTEGER NOT NULL CHECK(generated_at_unix_us BETWEEN 0 AND 253402300799999999)
            )"#,
        )
        .await?;
    sqlx::query("INSERT INTO marketplace_generation VALUES (1, 1, ?)")
        .bind(generated_at_unix_us)
        .execute(&mut *transaction)
        .await?;
    transaction
        .execute(
            r#"CREATE TRIGGER marketplace_generation_no_delete
               BEFORE DELETE ON marketplace_generation
               BEGIN SELECT RAISE(ABORT, 'marketplace_generation_delete_forbidden'); END"#,
        )
        .await?;
    transaction
        .execute(
            r#"CREATE TRIGGER marketplace_generation_valid_update
               BEFORE UPDATE ON marketplace_generation
               WHEN NEW.singleton_id != OLD.singleton_id
                 OR NEW.revision != OLD.revision + 1
                 OR NEW.generated_at_unix_us <= OLD.generated_at_unix_us
               BEGIN SELECT RAISE(ABORT, 'marketplace_generation_invalid_update'); END"#,
        )
        .await?;

    for table in FULL_SOURCES {
        for (operation, timing) in [
            ("insert", "AFTER INSERT"),
            ("update", "AFTER UPDATE"),
            ("delete", "AFTER DELETE"),
        ] {
            let sql = trigger_sql(
                &format!("marketplace_{table}_{operation}"),
                timing,
                table,
                None,
            );
            transaction.execute(sql.as_str()).await?;
        }
    }
    let settings_insert = trigger_sql(
        "marketplace_system_settings_insert",
        "AFTER INSERT",
        "system_settings",
        Some("NEW.key = 'reasoning_suffix_map'"),
    );
    transaction.execute(settings_insert.as_str()).await?;
    let settings_delete = trigger_sql(
        "marketplace_system_settings_delete",
        "AFTER DELETE",
        "system_settings",
        Some("OLD.key = 'reasoning_suffix_map'"),
    );
    transaction.execute(settings_delete.as_str()).await?;
    let settings_update = trigger_sql(
        "marketplace_system_settings_update",
        "AFTER UPDATE",
        "system_settings",
        Some(
            "(OLD.key = 'reasoning_suffix_map' OR NEW.key = 'reasoning_suffix_map') AND (OLD.key != NEW.key OR OLD.value != NEW.value)",
        ),
    );
    transaction.execute(settings_update.as_str()).await?;
    transaction.commit().await
}

pub async fn generation_revision(db: &mut SqliteConnection) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT revision FROM marketplace_generation WHERE singleton_id = 1")
        .fetch_one(db)
        .await?;
    row.try_get("revision")
}

fn trigger_sql(name: &str, timing: &str, table: &str, condition: Option<&str>) -> String {
    let when = condition.map_or_else(String::new, |value| format!(" WHEN {value}"));
    format!(
        "CREATE TRIGGER {name} {timing} ON {table}{when} BEGIN UPDATE marketplace_generation SET revision = revision + 1, generated_at_unix_us = max(CAST((julianday('now') - 2440587.5) * 86400000000 AS INTEGER), generated_at_unix_us + 1) WHERE singleton_id = 1; END"
    )
}
