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

The summary does not approve product integration or deployment. Full Gate B through Gate E
still require the maximum-envelope benchmarks, the redacted production copy, the complete
status fault profiles, public-name approval, and topology preflight.
