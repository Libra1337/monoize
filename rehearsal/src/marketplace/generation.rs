use sqlx::{Connection, Executor, PgConnection, Row, SqliteConnection};

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

pub async fn create_postgres_generation_schema(
    db: &mut PgConnection,
    generated_at_unix_us: i64,
) -> Result<(), sqlx::Error> {
    let mut transaction = db.begin().await?;
    transaction
        .execute(
            r#"CREATE TABLE marketplace_generation (
                singleton_id SMALLINT PRIMARY KEY CHECK(singleton_id = 1),
                revision BIGINT NOT NULL CHECK(revision BETWEEN 1 AND 9223372036854775807),
                generated_at_unix_us BIGINT NOT NULL CHECK(generated_at_unix_us BETWEEN 0 AND 253402300799999999)
            )"#,
        )
        .await?;
    sqlx::query("INSERT INTO marketplace_generation VALUES (1, 1, $1)")
        .bind(generated_at_unix_us)
        .execute(&mut *transaction)
        .await?;
    transaction
        .execute(
            r#"CREATE FUNCTION increment_marketplace_generation() RETURNS void AS $$
               BEGIN
                 UPDATE marketplace_generation
                    SET revision = revision + 1,
                        generated_at_unix_us = GREATEST(
                          (EXTRACT(EPOCH FROM clock_timestamp()) * 1000000)::BIGINT,
                          generated_at_unix_us + 1
                        )
                  WHERE singleton_id = 1;
                 IF NOT FOUND THEN RAISE EXCEPTION 'marketplace_generation_missing'; END IF;
                 RETURN;
               END;
               $$ LANGUAGE plpgsql"#,
        )
        .await?;
    transaction
        .execute(
            r#"CREATE FUNCTION advance_marketplace_generation() RETURNS trigger AS $$
               BEGIN PERFORM increment_marketplace_generation(); RETURN NULL; END;
               $$ LANGUAGE plpgsql"#,
        )
        .await?;
    transaction
        .execute(
            r#"CREATE FUNCTION guard_marketplace_generation() RETURNS trigger AS $$
               BEGIN
                 IF TG_OP = 'DELETE' THEN RAISE EXCEPTION 'marketplace_generation_delete_forbidden'; END IF;
                 IF NEW.singleton_id <> OLD.singleton_id
                    OR NEW.revision <> OLD.revision + 1
                    OR NEW.generated_at_unix_us <= OLD.generated_at_unix_us THEN
                   RAISE EXCEPTION 'marketplace_generation_invalid_update';
                 END IF;
                 RETURN NEW;
               END;
               $$ LANGUAGE plpgsql"#,
        )
        .await?;
    transaction
        .execute("CREATE TRIGGER marketplace_generation_guard BEFORE UPDATE OR DELETE ON marketplace_generation FOR EACH ROW EXECUTE FUNCTION guard_marketplace_generation()")
        .await?;
    transaction
        .execute(
            r#"CREATE FUNCTION block_marketplace_generation_truncate() RETURNS trigger AS $$
               BEGIN RAISE EXCEPTION 'marketplace_generation_truncate_forbidden'; END;
               $$ LANGUAGE plpgsql"#,
        )
        .await?;
    transaction
        .execute("CREATE TRIGGER marketplace_generation_no_truncate BEFORE TRUNCATE ON marketplace_generation FOR EACH STATEMENT EXECUTE FUNCTION block_marketplace_generation_truncate()")
        .await?;

    for table in FULL_SOURCES {
        for (operation, timing) in [
            ("insert", "AFTER INSERT"),
            ("update", "AFTER UPDATE"),
            ("delete", "AFTER DELETE"),
            ("truncate", "AFTER TRUNCATE"),
        ] {
            let trigger = format!(
                "CREATE TRIGGER marketplace_{table}_{operation} {timing} ON {table} FOR EACH STATEMENT EXECUTE FUNCTION advance_marketplace_generation()"
            );
            transaction.execute(trigger.as_str()).await?;
        }
    }
    transaction
        .execute(
            r#"CREATE FUNCTION advance_marketplace_setting_insert() RETURNS trigger AS $$
               BEGIN
                 IF EXISTS (SELECT 1 FROM new_rows WHERE key = 'reasoning_suffix_map')
                 THEN PERFORM increment_marketplace_generation(); END IF;
                 RETURN NULL;
               END; $$ LANGUAGE plpgsql;
               CREATE FUNCTION advance_marketplace_setting_delete() RETURNS trigger AS $$
               BEGIN
                 IF EXISTS (SELECT 1 FROM old_rows WHERE key = 'reasoning_suffix_map')
                 THEN PERFORM increment_marketplace_generation(); END IF;
                 RETURN NULL;
               END; $$ LANGUAGE plpgsql;
               CREATE FUNCTION advance_marketplace_setting_update() RETURNS trigger AS $$
               BEGIN
                 IF EXISTS (
                   SELECT 1 FROM old_rows o FULL OUTER JOIN new_rows n USING (key)
                    WHERE (o.key = 'reasoning_suffix_map' OR n.key = 'reasoning_suffix_map')
                      AND (o.key IS DISTINCT FROM n.key OR o.value IS DISTINCT FROM n.value)
                 ) THEN PERFORM increment_marketplace_generation(); END IF;
                 RETURN NULL;
               END; $$ LANGUAGE plpgsql"#,
        )
        .await?;
    transaction
        .execute("CREATE TRIGGER marketplace_system_settings_insert AFTER INSERT ON system_settings REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION advance_marketplace_setting_insert()")
        .await?;
    transaction
        .execute("CREATE TRIGGER marketplace_system_settings_update AFTER UPDATE ON system_settings REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION advance_marketplace_setting_update()")
        .await?;
    transaction
        .execute("CREATE TRIGGER marketplace_system_settings_delete AFTER DELETE ON system_settings REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION advance_marketplace_setting_delete()")
        .await?;
    transaction
        .execute("CREATE TRIGGER marketplace_system_settings_truncate AFTER TRUNCATE ON system_settings FOR EACH STATEMENT EXECUTE FUNCTION advance_marketplace_generation()")
        .await?;
    transaction.commit().await
}

pub async fn postgres_generation_revision(db: &mut PgConnection) -> Result<i64, sqlx::Error> {
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
