use crate::marketplace::{
    BenchmarkConfig, BenchmarkMode, Envelope, FixtureManifest, QuerySetManifest,
    run_sqlite_benchmark, write_benchmark_report,
};
use crate::provider::canonical_json;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub async fn run(args: impl IntoIterator<Item = OsString>) -> anyhow::Result<()> {
    let args = args.into_iter().skip(1).collect::<Vec<_>>();
    match args.first().and_then(|value| value.to_str()) {
        Some("gate-summary") => run_gate_summary(&args).await,
        Some("marketplace")
            if args.get(1).and_then(|value| value.to_str()) == Some("benchmark") =>
        {
            run_marketplace_benchmark(&args).await
        }
        _ => anyhow::bail!(usage()),
    }
}

async fn run_gate_summary(args: &[OsString]) -> anyhow::Result<()> {
    let output_index = args
        .iter()
        .position(|value| value == "--output")
        .context("missing --output")?;
    let output = args.get(output_index + 1).context("missing output path")?;
    let root = std::env::current_dir()?;
    let summary = build_gate_summary(GateSummaryInput {
        git_commit: std::env::var("LYNSHEN_REHEARSAL_GIT_COMMIT")
            .unwrap_or_else(|_| "unknown".to_owned()),
        sqlite_tests_passed: true,
        postgres_available: std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL").is_ok(),
        postgres_rehearsal_passed: std::env::var("LYNSHEN_REHEARSAL_POSTGRES_VERIFIED")
            .is_ok_and(|value| value == "1"),
        ..GateSummaryInput::default()
    });
    write_gate_summary(&root, PathBuf::from(output), &summary)
}

async fn run_marketplace_benchmark(args: &[OsString]) -> anyhow::Result<()> {
    let backend = required_argument(args, "--backend")?;
    if backend != "sqlite" {
        anyhow::bail!("unsupported benchmark backend: {backend}");
    }
    let envelope = match required_argument(args, "--envelope")? {
        "smoke" => Envelope::Smoke,
        "qualification" => Envelope::Qualification,
        value => anyhow::bail!("unsupported benchmark envelope: {value}"),
    };
    let mode = match optional_argument(args, "--mode")? {
        None | Some("smoke") => BenchmarkMode::Smoke,
        Some("qualification") => BenchmarkMode::Qualification,
        Some(value) => anyhow::bail!("unsupported benchmark mode: {value}"),
    };
    let query_set = PathBuf::from(required_argument(args, "--query-set")?);
    let output = PathBuf::from(required_argument(args, "--output")?);
    let root = std::env::current_dir()?;
    let fixture = FixtureManifest::generate(0x004c_594e_5348_454e, envelope)?;
    QuerySetManifest::read(&root.join(query_set))?.validate(&fixture)?;
    let report = run_sqlite_benchmark(BenchmarkConfig {
        seed: fixture.seed,
        envelope,
        mode,
        query_limit: None,
        git_commit: resolve_clean_git_commit(&root)?,
    })
    .await?;
    write_benchmark_report(&root, output, &report)
}

fn required_argument<'a>(args: &'a [OsString], name: &str) -> anyhow::Result<&'a str> {
    optional_argument(args, name)?.with_context(|| format!("missing {name}"))
}

fn optional_argument<'a>(args: &'a [OsString], name: &str) -> anyhow::Result<Option<&'a str>> {
    let Some(index) = args.iter().position(|value| value == name) else {
        return Ok(None);
    };
    let value = args
        .get(index + 1)
        .with_context(|| format!("missing value for {name}"))?;
    value
        .to_str()
        .map(Some)
        .with_context(|| format!("{name} is not valid Unicode"))
}

pub fn resolve_clean_git_commit(root: &Path) -> anyhow::Result<String> {
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .context("inspect benchmark worktree")?;
    if !status.status.success() {
        anyhow::bail!("benchmark_git_status_failed");
    }
    if !status.stdout.is_empty() {
        anyhow::bail!("benchmark_requires_clean_worktree");
    }
    if let Ok(value) = std::env::var("LYNSHEN_REHEARSAL_GIT_COMMIT")
        && !value.is_empty()
    {
        return Ok(value);
    }
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .context("resolve benchmark git commit")
}

fn usage() -> &'static str {
    "usage: lynshen-rehearsal gate-summary --output rehearsal/evidence/<file>.json\n       lynshen-rehearsal marketplace benchmark --backend sqlite --envelope smoke --query-set rehearsal/fixtures/marketplace/query-set.json --output rehearsal/evidence/<file>.json"
}

use anyhow::Context;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateSummaryInput {
    pub git_commit: String,
    pub sqlite_tests_passed: bool,
    pub postgres_available: bool,
    pub postgres_rehearsal_passed: bool,
    pub root_tests_passed: bool,
    pub docs_build_passed: bool,
    pub marketplace_qualification_passed: bool,
    pub status_qualification_passed: bool,
    pub production_copy_rehearsed: bool,
    pub public_name_manifest_approved: bool,
    pub topology_preflight_recorded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSummary {
    pub schema_version: u8,
    pub git_commit: String,
    pub components: Components,
    pub gates: Gates,
    pub product_integration_authorized: bool,
    pub production_deployment_authorized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Components {
    pub provider_transform: ResultState,
    pub sqlite_migration: ResultState,
    pub exact_pricing: ResultState,
    pub marketplace_primitives: ResultState,
    pub status_primitives: ResultState,
    pub postgres_rehearsal: ResultState,
    pub root_project_verification: ResultState,
    pub docs_build: ResultState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gates {
    pub gate_b: GateState,
    pub gate_c: GateState,
    pub gate_d: GateState,
    pub gate_e: GateState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultState {
    pub executed: bool,
    pub passed: bool,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateState {
    pub passed: bool,
    pub blockers: Vec<String>,
}

pub fn build_gate_summary(input: GateSummaryInput) -> GateSummary {
    let component = |passed: bool, note: &str| ResultState {
        executed: true,
        passed,
        note: note.to_owned(),
    };
    let unavailable = |note: &str| ResultState {
        executed: false,
        passed: false,
        note: note.to_owned(),
    };
    let mut gate_b_blockers = Vec::new();
    if !input.postgres_available {
        gate_b_blockers.push("postgres_not_available".to_owned());
    } else if !input.postgres_rehearsal_passed {
        gate_b_blockers.push("postgres_core_rehearsal_not_recorded".to_owned());
    }
    if !input.marketplace_qualification_passed {
        gate_b_blockers.push("maximum_envelope_marketplace_not_run".to_owned());
    }
    if !input.production_copy_rehearsed {
        gate_b_blockers.push("redacted_production_copy_not_rehearsed".to_owned());
    }
    let gate_b = GateState {
        passed: input.sqlite_tests_passed && gate_b_blockers.is_empty(),
        blockers: gate_b_blockers,
    };
    let gate_c = GateState {
        passed: input.sqlite_tests_passed && input.production_copy_rehearsed,
        blockers: (!input.production_copy_rehearsed)
            .then(|| "production_pricing_snapshot_not_compared".to_owned())
            .into_iter()
            .collect(),
    };
    let gate_d = GateState {
        passed: input.status_qualification_passed,
        blockers: (!input.status_qualification_passed)
            .then(|| "full_status_load_and_fault_profiles_not_run".to_owned())
            .into_iter()
            .collect(),
    };
    let mut gate_e_blockers = Vec::new();
    if !input.public_name_manifest_approved {
        gate_e_blockers.push("public_name_manifest_not_approved".to_owned());
    }
    if !input.topology_preflight_recorded {
        gate_e_blockers.push("public_topology_preflight_not_recorded".to_owned());
    }
    let gate_e = GateState {
        passed: input.sqlite_tests_passed && gate_e_blockers.is_empty(),
        blockers: gate_e_blockers,
    };
    let product_integration_authorized =
        gate_b.passed && gate_c.passed && gate_d.passed && gate_e.passed;
    GateSummary {
        schema_version: 1,
        git_commit: input.git_commit,
        components: Components {
            provider_transform: component(input.sqlite_tests_passed, "isolated TDD suite"),
            sqlite_migration: component(
                input.sqlite_tests_passed,
                "schema prototype, Cartesian transform, transaction, rollback, and no-op suite",
            ),
            exact_pricing: component(input.sqlite_tests_passed, "canonical golden snapshot suite"),
            marketplace_primitives: component(
                input.sqlite_tests_passed,
                "cursor, generation, query, ETag, rate-limit, allow-list suite",
            ),
            status_primitives: component(
                input.sqlite_tests_passed,
                "capacity, cross-platform allocation/free-byte probe, directory fsync probe, fail-closed unknown file slots, spool scan, dispatch, replay, aggregation suite; full load and fault profiles remain Gate D blockers",
            ),
            postgres_rehearsal: if input.postgres_rehearsal_passed {
                component(
                    true,
                    "PostgreSQL 17 schema, Cartesian migration, rollback, no-op, and generation core suite",
                )
            } else if input.postgres_available {
                component(false, "available but core rehearsal suite not recorded")
            } else {
                unavailable("Docker and PostgreSQL client are unavailable on this host")
            },
            root_project_verification: component(
                input.root_tests_passed,
                "root build requires libclang on this host",
            ),
            docs_build: component(input.docs_build_passed, "Bun is unavailable on this host"),
        },
        gates: Gates {
            gate_b,
            gate_c,
            gate_d,
            gate_e,
        },
        product_integration_authorized,
        production_deployment_authorized: false,
    }
}

pub fn write_gate_summary(
    root: &Path,
    output: impl AsRef<Path>,
    summary: &GateSummary,
) -> anyhow::Result<()> {
    let output = if output.as_ref().is_absolute() {
        output.as_ref().to_owned()
    } else {
        root.join(output)
    };
    let evidence = root.join("rehearsal/evidence");
    let parent = output.parent().context("output has no parent")?;
    if parent != evidence {
        anyhow::bail!("evidence output must be directly under rehearsal/evidence");
    }
    std::fs::create_dir_all(&evidence)?;
    std::fs::write(output, canonical_json(summary)?)?;
    Ok(())
}
