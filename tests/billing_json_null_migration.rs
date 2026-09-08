#[path = "../src/migration/m20260809_000030_normalize_billing_json_nulls.rs"]
mod migration_under_test;

use std::collections::BTreeMap;

use migration_under_test::{Migration, normalization_sql};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use sea_orm_migration::{MigrationTrait, SchemaManager};

#[test]
fn postgres_and_sqlite_use_the_same_json_whitespace_code_points() {
    let sqlite = normalization_sql(DbBackend::Sqlite).expect("SQLite SQL");
    let postgres = normalization_sql(DbBackend::Postgres).expect("PostgreSQL SQL");

    for code_point in [9, 10, 13, 32] {
        assert!(sqlite.contains(&format!("char({code_point})")));
        assert!(postgres.contains(&format!("chr({code_point})")));
    }
    assert_eq!(sqlite.matches("= 'null'").count(), 4);
    assert_eq!(postgres.matches("= 'null'").count(), 4);
    assert!(normalization_sql(DbBackend::MySql).is_none());
}

#[tokio::test]
async fn sqlite_normalizes_only_json_null_scalars_and_is_idempotent() {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "CREATE TABLE billing_rate_records (id TEXT PRIMARY KEY, match_json TEXT NOT NULL, raw_json TEXT NOT NULL)"
            .to_string(),
    ))
    .await
    .expect("create billing-rate fixture table");

    let fixtures = [
        ("exact", "null", "null"),
        ("json-whitespace", " \t\n\rnull\r\n\t ", "\r null \t"),
        ("independent", "[]", " null "),
        ("objects", "{}", "{\"value\":null}"),
        ("arrays", "[null]", "[]"),
        ("malformed", "nul", "null trailing"),
        ("scalars", "\"null\"", "true"),
        ("case-sensitive", "NULL", "Null"),
        (
            "non-json-whitespace",
            "\u{000c}null\u{000c}",
            "\u{00a0}null\u{00a0}",
        ),
    ];
    for (id, match_json, raw_json) in fixtures {
        db.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO billing_rate_records (id, match_json, raw_json) VALUES (?1, ?2, ?3)",
            [id.into(), match_json.into(), raw_json.into()],
        ))
        .await
        .expect("insert billing-rate fixture");
    }

    let manager = SchemaManager::new(&db);
    Migration.up(&manager).await.expect("run migration once");
    Migration.up(&manager).await.expect("run migration twice");

    let rows = db
        .query_all(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id, match_json, raw_json FROM billing_rate_records ORDER BY id".to_string(),
        ))
        .await
        .expect("read migrated fixtures");
    let actual = rows
        .into_iter()
        .map(|row| {
            let id: String = row.try_get("", "id").expect("decode id");
            let match_json: String = row.try_get("", "match_json").expect("decode match_json");
            let raw_json: String = row.try_get("", "raw_json").expect("decode raw_json");
            (id, (match_json, raw_json))
        })
        .collect::<BTreeMap<_, _>>();

    let expected = BTreeMap::from([
        (
            "arrays".to_string(),
            ("[null]".to_string(), "[]".to_string()),
        ),
        (
            "case-sensitive".to_string(),
            ("NULL".to_string(), "Null".to_string()),
        ),
        ("exact".to_string(), ("{}".to_string(), "{}".to_string())),
        (
            "independent".to_string(),
            ("[]".to_string(), "{}".to_string()),
        ),
        (
            "json-whitespace".to_string(),
            ("{}".to_string(), "{}".to_string()),
        ),
        (
            "malformed".to_string(),
            ("nul".to_string(), "null trailing".to_string()),
        ),
        (
            "non-json-whitespace".to_string(),
            (
                "\u{000c}null\u{000c}".to_string(),
                "\u{00a0}null\u{00a0}".to_string(),
            ),
        ),
        (
            "objects".to_string(),
            ("{}".to_string(), "{\"value\":null}".to_string()),
        ),
        (
            "scalars".to_string(),
            ("\"null\"".to_string(), "true".to_string()),
        ),
    ]);
    assert_eq!(actual, expected);
}
