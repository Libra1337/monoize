# LynShen Rehearsal

This crate is an isolated migration and public-contract rehearsal.

It does not register a Monoize migration. It does not register an HTTP route. It does not
run from Monoize startup.

Run the SQLite and pure-component suite:

```powershell
cargo test --manifest-path rehearsal/Cargo.toml --all-targets
```

Run the SQLite and PostgreSQL core suite on Windows:

```powershell
powershell -ExecutionPolicy Bypass -File rehearsal/scripts/run-core-gates.ps1
```

The script creates PostgreSQL data only under `rehearsal/.postgres-data`. It listens only
on `127.0.0.1`. It writes the conservative result to
`rehearsal/evidence/gate-summary.json`.

Run a PostgreSQL Marketplace benchmark only against the isolated database:

```powershell
$env:LYNSHEN_REHEARSAL_POSTGRES_URL = "postgres://postgres@127.0.0.1:55434/lynshen_rehearsal"
cargo run --manifest-path rehearsal/Cargo.toml --bin lynshen-rehearsal -- marketplace benchmark --backend postgres --envelope smoke --query-set rehearsal/fixtures/marketplace/query-set.json --output rehearsal/evidence/marketplace-postgres-smoke.json
```

Use `--backend paired` to run both databases with one config. The command writes both
reports and their equality result to one evidence file.

The command rejects any PostgreSQL database name other than `lynshen_rehearsal` before it
connects. Run the command from a clean Git worktree.

The summary does not approve product integration or deployment. Full Gate B through Gate E
still require the maximum-envelope benchmarks, the redacted production copy, the complete
status fault profiles, public-name approval, and topology preflight.
