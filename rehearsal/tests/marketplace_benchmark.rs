use monoize_lynshen_rehearsal::marketplace::{
    BenchmarkConfig, BenchmarkMode, Envelope, run_sqlite_benchmark, write_benchmark_report,
};
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

fn run_git(root: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}
