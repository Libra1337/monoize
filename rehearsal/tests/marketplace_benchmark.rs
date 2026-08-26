use monoize_lynshen_rehearsal::marketplace::{
    BenchmarkConfig, BenchmarkMode, Envelope, compare_benchmark_reports, run_postgres_benchmark,
    run_sqlite_benchmark, write_benchmark_report,
};
use sqlx::{Connection, Executor, PgConnection, Row};
use std::process::Command;
use tempfile::TempDir;

#[tokio::test]
async fn sqlite_smoke_runs_real_queries_and_reports_non_qualification() {
    let report = run_sqlite_benchmark(BenchmarkConfig {
        seed: 0x004c_594e_5348_454e,
        envelope: Envelope::Smoke,
        mode: BenchmarkMode::Smoke,
        query_limit: Some(40),
        git_commit: "test-commit".to_owned(),
    })
    .await
    .unwrap();

    assert_eq!(report.backend, "sqlite");
    assert_eq!(report.git_commit, "test-commit");
    assert_eq!(report.samples, 40);
    assert_eq!(report.loaded_groups, 8);
    assert_eq!(report.loaded_providers, 128);
    assert_eq!(report.loaded_provider_models, 4_096);
    assert_eq!(report.loaded_rate_rows, 8_192);
    assert_eq!(report.loaded_metadata_rows, 2_048);
    assert_eq!(report.declared_offer_rate_entries, 32_768);
    assert_eq!(report.materialized_offer_rate_entries, 32_768);
    assert_eq!(report.failed_samples, 0);
    assert!(report.p50_microseconds <= report.p95_microseconds);
    assert!(report.p95_microseconds <= report.p99_microseconds);
    assert!(report.statement_count >= report.samples);
    assert!(report.response_bytes > 0);
    assert_eq!(report.list.samples, 32);
    assert_eq!(report.offers.samples, 8);
    assert_eq!(report.list.failed_samples, 0);
    assert_eq!(report.offers.failed_samples, 0);
    assert!(report.list.p95_microseconds <= report.list.p99_microseconds);
    assert!(report.offers.p95_microseconds <= report.offers.p99_microseconds);
    assert!(report.list.max_response_bytes <= 1_048_576);
    assert!(report.offers.max_response_bytes <= 1_048_576);
    assert!(!report.gate_b_qualified);
    assert!(
        report
            .qualification_blockers
            .contains(&"smoke_mode".to_owned())
    );
    assert!(
        report
            .qualification_blockers
            .contains(&"insufficient_workers".to_owned())
    );
}

#[tokio::test]
async fn postgres_smoke_matches_the_fixture_and_exact_query_contract() {
    let Ok(url) = std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL") else {
        return;
    };
    let report = run_postgres_benchmark(
        &url,
        BenchmarkConfig {
            seed: 0x004c_594e_5348_454e,
            envelope: Envelope::Smoke,
            mode: BenchmarkMode::Smoke,
            query_limit: Some(40),
            git_commit: "test-commit".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.backend, "postgres");
    assert_eq!(report.loaded_groups, 8);
    assert_eq!(report.loaded_providers, 128);
    assert_eq!(report.loaded_provider_models, 4_096);
    assert_eq!(report.loaded_rate_rows, 8_192);
    assert_eq!(report.loaded_metadata_rows, 2_048);
    assert_eq!(report.materialized_offer_rate_entries, 32_768);
    assert_eq!(report.failed_samples, 0);
    assert_eq!(report.list.samples, 32);
    assert_eq!(report.offers.samples, 8);
    assert!(!report.gate_b_qualified);
}

#[tokio::test]
async fn postgres_benchmark_rejects_a_non_rehearsal_database_before_connecting() {
    for url in [
        "postgres://postgres@127.0.0.1:1/monoize",
        "not a postgres URL",
    ] {
        let error = run_postgres_benchmark(
            url,
            BenchmarkConfig {
                seed: 0x004c_594e_5348_454e,
                envelope: Envelope::Smoke,
                mode: BenchmarkMode::Smoke,
                query_limit: Some(1),
                git_commit: "test-commit".to_owned(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "postgres_rehearsal_database_required");
    }
}

#[tokio::test]
async fn postgres_benchmark_rejects_a_remote_host_before_connecting() {
    let error = run_postgres_benchmark(
        "postgres://postgres@example.invalid/lynshen_rehearsal",
        BenchmarkConfig {
            seed: 0x004c_594e_5348_454e,
            envelope: Envelope::Smoke,
            mode: BenchmarkMode::Smoke,
            query_limit: Some(1),
            git_commit: "test-commit".to_owned(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "postgres_rehearsal_host_required");
}

#[tokio::test]
async fn postgres_and_sqlite_reports_identify_the_same_loaded_fixture() {
    let Ok(url) = std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL") else {
        return;
    };
    let config = BenchmarkConfig {
        seed: 0x004c_594e_5348_454e,
        envelope: Envelope::Smoke,
        mode: BenchmarkMode::Smoke,
        query_limit: Some(1),
        git_commit: "test-commit".to_owned(),
    };
    let sqlite = run_sqlite_benchmark(config.clone()).await.unwrap();
    let postgres = run_postgres_benchmark(&url, config).await.unwrap();
    assert_eq!(postgres.fixture_recipe_sha256, sqlite.fixture_recipe_sha256);
    assert_eq!(postgres.loaded_source_sha256, sqlite.loaded_source_sha256);
    assert_eq!(postgres.query_set_sha256, sqlite.query_set_sha256);
    assert_eq!(postgres.loaded_groups, sqlite.loaded_groups);
    assert_eq!(postgres.loaded_providers, sqlite.loaded_providers);
    assert_eq!(
        postgres.loaded_provider_models,
        sqlite.loaded_provider_models
    );
    assert_eq!(postgres.loaded_rate_rows, sqlite.loaded_rate_rows);
    assert_eq!(postgres.loaded_metadata_rows, sqlite.loaded_metadata_rows);
    assert_eq!(
        postgres.materialized_offer_rate_entries,
        sqlite.materialized_offer_rate_entries
    );
}

#[tokio::test]
async fn postgres_benchmark_preserves_the_public_schema() {
    let Ok(url) = std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL") else {
        return;
    };
    let mut database = PgConnection::connect(&url).await.unwrap();
    database
        .execute("CREATE TABLE IF NOT EXISTS public.lynshen_rehearsal_public_marker (id INTEGER)")
        .await
        .unwrap();
    run_postgres_benchmark(
        &url,
        BenchmarkConfig {
            seed: 0x004c_594e_5348_454e,
            envelope: Envelope::Smoke,
            mode: BenchmarkMode::Smoke,
            query_limit: Some(1),
            git_commit: "test-commit".to_owned(),
        },
    )
    .await
    .unwrap();
    let row = sqlx::query(
        "SELECT to_regclass('public.lynshen_rehearsal_public_marker')::TEXT AS table_name",
    )
    .fetch_one(&mut database)
    .await
    .unwrap();
    assert_eq!(
        row.try_get::<Option<String>, _>("table_name").unwrap(),
        Some("lynshen_rehearsal_public_marker".to_owned())
    );
    database
        .execute("DROP TABLE public.lynshen_rehearsal_public_marker")
        .await
        .unwrap();
}

#[tokio::test]
async fn postgres_benchmark_refuses_cross_schema_dependencies() {
    let Ok(url) = std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL") else {
        return;
    };
    let mut database = PgConnection::connect(&url).await.unwrap();
    database
        .execute("DROP VIEW IF EXISTS public.lynshen_rehearsal_public_dependency")
        .await
        .unwrap();
    run_postgres_benchmark(
        &url,
        BenchmarkConfig {
            seed: 0x004c_594e_5348_454e,
            envelope: Envelope::Smoke,
            mode: BenchmarkMode::Smoke,
            query_limit: Some(1),
            git_commit: "test-commit".to_owned(),
        },
    )
    .await
    .unwrap();
    database
        .execute(
            "CREATE VIEW public.lynshen_rehearsal_public_dependency AS SELECT id FROM lynshen_marketplace_benchmark.monoize_groups",
        )
        .await
        .unwrap();
    let error = run_postgres_benchmark(
        &url,
        BenchmarkConfig {
            seed: 0x004c_594e_5348_454e,
            envelope: Envelope::Smoke,
            mode: BenchmarkMode::Smoke,
            query_limit: Some(1),
            git_commit: "test-commit".to_owned(),
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("depend"));
    for qualified_name in [
        "public.lynshen_rehearsal_public_dependency",
        "lynshen_marketplace_benchmark.monoize_groups",
    ] {
        let row = sqlx::query("SELECT to_regclass($1)::TEXT AS object_name")
            .bind(qualified_name)
            .fetch_one(&mut database)
            .await
            .unwrap();
        assert!(
            row.try_get::<Option<String>, _>("object_name")
                .unwrap()
                .is_some()
        );
    }
    database
        .execute("DROP VIEW public.lynshen_rehearsal_public_dependency")
        .await
        .unwrap();
}

#[test]
fn qualification_config_rejects_a_smoke_envelope() {
    let error = BenchmarkConfig {
        seed: 0x004c_594e_5348_454e,
        envelope: Envelope::Smoke,
        mode: BenchmarkMode::Qualification,
        query_limit: None,
        git_commit: "test-commit".to_owned(),
    }
    .validate()
    .unwrap_err();
    assert_eq!(error.to_string(), "qualification_requires_maximum_envelope");
}

#[test]
fn benchmark_evidence_is_restricted_to_its_evidence_directory() {
    let root = TempDir::new().unwrap();
    std::fs::create_dir_all(root.path().join("rehearsal/evidence")).unwrap();
    let report = monoize_lynshen_rehearsal::marketplace::BenchmarkReport {
        schema_version: 1,
        backend: "sqlite".to_owned(),
        mode: BenchmarkMode::Smoke,
        envelope: Envelope::Smoke,
        git_commit: "test".to_owned(),
        fixture_recipe_sha256: "fixture-recipe".to_owned(),
        loaded_source_sha256: "loaded-source".to_owned(),
        query_set_sha256: "queries".to_owned(),
        loaded_groups: 8,
        loaded_providers: 128,
        loaded_provider_models: 4_096,
        loaded_rate_rows: 8_192,
        loaded_metadata_rows: 2_048,
        declared_offer_rate_entries: 32_768,
        materialized_offer_rate_entries: 32_768,
        samples: 1,
        failed_samples: 0,
        cache_hits: 0,
        cache_misses: 1,
        statement_count: 1,
        response_bytes: 1,
        p50_microseconds: 1,
        p95_microseconds: 1,
        p99_microseconds: 1,
        elapsed_milliseconds: 1,
        cpu_milliseconds: 1,
        rss_before_bytes: 1,
        rss_after_bytes: 1,
        rss_delta_bytes: 0,
        workers: 1,
        warmup_seconds: 0,
        measured_seconds: 0,
        gate_b_qualified: false,
        qualification_blockers: vec!["smoke_mode".to_owned()],
        list: monoize_lynshen_rehearsal::marketplace::OperationMetrics {
            samples: 1,
            failed_samples: 0,
            statement_count: 3,
            response_bytes: 1,
            max_response_bytes: 1,
            p50_microseconds: 1,
            p95_microseconds: 1,
            p99_microseconds: 1,
        },
        offers: monoize_lynshen_rehearsal::marketplace::OperationMetrics {
            samples: 1,
            failed_samples: 0,
            statement_count: 3,
            response_bytes: 1,
            max_response_bytes: 1,
            p50_microseconds: 1,
            p95_microseconds: 1,
            p99_microseconds: 1,
        },
    };
    assert!(write_benchmark_report(root.path(), "outside.json", &report).is_err());
    let target = root.path().join("rehearsal/evidence/smoke.json");
    write_benchmark_report(root.path(), &target, &report).unwrap();
    assert!(std::fs::read(target).unwrap().ends_with(b"\n"));
}

#[tokio::test]
async fn paired_report_requires_identical_backend_sources() {
    let mut sqlite = run_sqlite_benchmark(BenchmarkConfig {
        seed: 0x004c_594e_5348_454e,
        envelope: Envelope::Smoke,
        mode: BenchmarkMode::Smoke,
        query_limit: Some(1),
        git_commit: "test-commit".to_owned(),
    })
    .await
    .unwrap();
    let mut postgres = sqlite.clone();
    postgres.backend = "postgres".to_owned();
    let comparison = compare_benchmark_reports(sqlite.clone(), postgres.clone()).unwrap();
    assert!(comparison.comparison_passed);
    assert!(!comparison.gate_b_qualified);

    sqlite.loaded_source_sha256 = "different".to_owned();
    let error = compare_benchmark_reports(sqlite, postgres).unwrap_err();
    assert_eq!(
        error.to_string(),
        "benchmark_pair_mismatch:loaded_source_sha256"
    );
}

#[test]
fn benchmark_commit_requires_a_clean_worktree() {
    use monoize_lynshen_rehearsal::cli::resolve_clean_git_commit;

    let root = TempDir::new().unwrap();
    run_git(root.path(), &["init"]);
    std::fs::write(root.path().join("tracked.txt"), b"one").unwrap();
    run_git(root.path(), &["add", "tracked.txt"]);
    run_git(
        root.path(),
        &[
            "-c",
            "user.name=Rehearsal",
            "-c",
            "user.email=rehearsal@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    assert_eq!(resolve_clean_git_commit(root.path()).unwrap().len(), 40);

    std::fs::write(root.path().join("tracked.txt"), b"two").unwrap();
    assert_eq!(
        resolve_clean_git_commit(root.path())
            .unwrap_err()
            .to_string(),
        "benchmark_requires_clean_worktree"
    );
}

#[test]
fn postgres_cli_requires_the_isolated_database_url() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    for backend in ["postgres", "paired"] {
        let output = Command::new(env!("CARGO_BIN_EXE_lynshen-rehearsal"))
            .current_dir(root)
            .args([
                "marketplace",
                "benchmark",
                "--backend",
                backend,
                "--envelope",
                "smoke",
                "--query-set",
                "rehearsal/fixtures/marketplace/query-set.json",
                "--output",
                "rehearsal/evidence/unused.json",
            ])
            .env_remove("LYNSHEN_REHEARSAL_POSTGRES_URL")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("missing LYNSHEN_REHEARSAL_POSTGRES_URL")
        );
    }
}

fn run_git(root: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}
