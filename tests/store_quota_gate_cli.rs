use chrono::{TimeZone, Utc};
use monoize::db::DbPool;
use monoize::migration::Migrator;
use monoize::store_billing::quota_gate::{QuotaGateStore, QuotaManifest};
use monoize::store_billing::quota_gate_cli::execute_from;
use sea_orm::ConnectionTrait;
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn offline_cli_imports_next_and_promotes_it_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let database_path = temp.path().join("quota-gate.sqlite");
    let manifest_path = temp.path().join("quota-manifest.json");
    let dsn = format!("sqlite://{}", database_path.display());
    let db = DbPool::connect(&dsn).await.unwrap();
    Migrator::up(&*db.write().await, None).await.unwrap();
    let gate = QuotaGateStore::new(db.clone());
    let environment = gate.live_environment().await.unwrap();
    let manifest = QuotaManifest::passed(
        environment,
        "1.6.0",
        "drill-output-digest",
        Utc.with_ymd_and_hms(2026, 8, 29, 4, 0, 0).unwrap(),
        "operator-1",
    )
    .unwrap();
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let imported = execute_from(
        [
            "monoize-store-ops",
            "quota-gate",
            "import",
            "--slot",
            "next",
            "--manifest",
            manifest_path.to_str().unwrap(),
        ],
        Some(&dsn),
    )
    .await
    .unwrap();
    assert_eq!(imported.operation, "import");
    assert_eq!(imported.slot.as_deref(), Some("next"));
    assert_eq!(
        imported.compatibility_fingerprint,
        manifest.compatibility_fingerprint
    );

    let promoted = execute_from(
        [
            "monoize-store-ops",
            "quota-gate",
            "promote",
            "--expected-fingerprint",
            &manifest.compatibility_fingerprint,
        ],
        Some(&dsn),
    )
    .await
    .unwrap();
    assert_eq!(promoted.operation, "promote");
    assert_eq!(promoted.slot, None);
    assert_eq!(
        gate.current_manifest()
            .await
            .unwrap()
            .unwrap()
            .application_version,
        "1.6.0"
    );
    let next = db
        .read()
        .query_one(db.stmt(
            "SELECT state, compatibility_fingerprint FROM store_quota_gates
             WHERE backend = 'sqlite' AND slot = 'next'",
            vec![],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.try_get::<String>("", "state").unwrap(), "pending");
    assert_eq!(
        next.try_get::<String>("", "compatibility_fingerprint")
            .unwrap(),
        ""
    );
}

#[tokio::test]
async fn offline_cli_rejects_unsafe_inputs_before_database_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let oversized = temp.path().join("oversized.json");
    std::fs::write(&oversized, vec![b'x'; 65_537]).unwrap();
    let args = [
        "monoize-store-ops",
        "quota-gate",
        "import",
        "--slot",
        "current",
        "--manifest",
        oversized.to_str().unwrap(),
    ];

    assert_eq!(
        execute_from(args, None).await.unwrap_err().code(),
        "database_dsn_missing"
    );
    assert_eq!(
        execute_from(args, Some("postgres://localhost/monoize"))
            .await
            .unwrap_err()
            .code(),
        "database_backend_invalid"
    );
    assert_eq!(
        execute_from(args, Some("sqlite::memory:"))
            .await
            .unwrap_err()
            .code(),
        "quota_manifest_too_large"
    );
}
