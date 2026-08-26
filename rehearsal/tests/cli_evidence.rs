use monoize_lynshen_rehearsal::cli::{GateSummaryInput, build_gate_summary, write_gate_summary};
use tempfile::TempDir;

#[test]
fn summary_distinguishes_passed_executed_and_blocked_gates() {
    let summary = build_gate_summary(GateSummaryInput {
        git_commit: "abc123".to_owned(),
        sqlite_tests_passed: true,
        postgres_available: false,
        postgres_rehearsal_passed: false,
        root_tests_passed: false,
        docs_build_passed: false,
        marketplace_qualification_passed: false,
        status_qualification_passed: false,
        production_copy_rehearsed: false,
        public_name_manifest_approved: false,
        topology_preflight_recorded: false,
    });

    assert!(summary.components.provider_transform.passed);
    assert!(summary.components.sqlite_migration.passed);
    assert!(!summary.gates.gate_b.passed);
    assert!(
        summary
            .gates
            .gate_b
            .blockers
            .contains(&"postgres_not_available".to_owned())
    );
    assert!(!summary.gates.gate_e.passed);
    assert!(
        summary
            .gates
            .gate_e
            .blockers
            .contains(&"public_name_manifest_not_approved".to_owned())
    );
    assert!(!summary.product_integration_authorized);
    assert!(!summary.production_deployment_authorized);
    assert!(
        !summary
            .components
            .status_primitives
            .note
            .contains("full directory fsync")
    );
    assert!(
        summary
            .components
            .status_primitives
            .note
            .contains("directory fsync probe")
    );
}

#[test]
fn evidence_must_be_written_inside_rehearsal_evidence() {
    let directory = TempDir::new().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("rehearsal/evidence")).unwrap();
    let summary = build_gate_summary(GateSummaryInput::default());

    assert!(write_gate_summary(root, root.join("outside.json"), &summary).is_err());
    let target = root.join("rehearsal/evidence/gate-summary.json");
    write_gate_summary(root, &target, &summary).unwrap();
    let bytes = std::fs::read(&target).unwrap();
    assert!(bytes.ends_with(b"\n"));
}
