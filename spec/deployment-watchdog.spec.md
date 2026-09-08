# Deployment Watchdog Specification

## 0A. Destructive Provider migration exception

DW-MIG-1. The Provider pricing release MUST NOT use `deploy-watchdog`. The command MUST
fail before build, copy, restart, or watchdog arming when the candidate declares this
destructive schema release.

DW-MIG-2. Binary-only rollback is forbidden after this migration starts. The old image
MUST NOT open a migrated database.

DW-MIG-3. Release deployment MUST freeze real writes, stop every Replica and old process,
create and restore-test a current database backup, run migration against a restored copy,
and receive separate deployment approval before production cutover.

DW-MIG-4. Before release acceptance, rollback MUST stop the candidate, preserve the failed
database, restore the verified pre-cutover database, then start image `monoize:cf36bd8`.
After real writes resume, restoring that snapshot in place is forbidden without a separate
data-reconciliation and data-loss plan.

## 0. Scope

- This specification defines the behavior of the repository-local `deploy.sh` script.
- The deployment target directory is `/opt/monoize`.
- The deployed process name is `monoize` under PM2.

## 1. Default deploy workflow

D1. Running `./deploy.sh` with no subcommand MUST execute the following steps in order:

1. Build the frontend with `bun run build` from `frontend/`.
2. Build the release binary with `cargo build --release` from the repository root.
3. If `/opt/monoize/monoize` exists, copy it to `/opt/monoize/monoize.bak.<timestamp>` before replacing it.
4. Copy `target/release/monoize` to `/opt/monoize/monoize.next` and atomically move it to `/opt/monoize/monoize`.
5. Restart PM2 process `monoize`.
6. Save PM2 state.

D1a. The default deploy workflow MUST NOT arm a rollback watchdog.

D2. Before step D1.4 completes, the script MUST cancel any previously armed deployment watchdog state recorded under `/opt/monoize/.deploy-watchdog/`.

D2a. If step D1.5 fails and a backup path from D1.3 exists, the script MUST synchronously restore that backup binary to `/opt/monoize/monoize`, attempt to restart PM2 process `monoize` using the restored binary, and then exit with failure.

## 2. Watchdog arming behavior

D3. Running `./deploy.sh deploy-watchdog` MUST execute D1.1 through D1.6. After step D1.6 completes, if a backup path from D1.3 exists, the script MUST arm a rollback watchdog with a timeout of exactly 300 seconds. Production execution MUST NOT accept an environment override for this timeout.

D3a. A repository-local test MAY select a timeout from 1 through 30 seconds only when all of the following inputs are present and valid:

- `MONOIZE_DEPLOY_TEST_MODE` is exactly `1`;
- `MONOIZE_DEPLOY_TEST_ROOT` resolves to the canonical directory that contains the executing copy of `deploy.sh`, and that directory's basename matches `.deploy-watchdog-test.*`;
- `MONOIZE_DEPLOY_TEST_WATCHDOG_TIMEOUT_SECONDS` is an integer from 1 through 30 inclusive.

In this mode, the deployment target MUST be `<MONOIZE_DEPLOY_TEST_ROOT>/deploy`. Supplying either test input without `MONOIZE_DEPLOY_TEST_MODE=1`, or supplying an invalid test input, MUST terminate the script before a build, binary copy, or PM2 command occurs.

D4. The watchdog MUST persist its state under `/opt/monoize/.deploy-watchdog/` using files that identify:

- the currently armed deploy identifier;
- the PID of the background watchdog process;
- the Linux process start-time identity associated with that PID;
- the backup binary path to restore.

D4a. The PID in `current_pid` MUST identify both the watchdog session leader and the watchdog process-group leader. The timer child MUST remain in that process group. The value in `current_identity` MUST equal Linux `/proc/<current_pid>/stat` field 22 for that process.

D4b. The watchdog MUST run in a session distinct from the invoking deployment shell. Its standard input, standard output, and standard error MUST NOT reference the invoking terminal. Closing the invoking shell, PTY, or SSH session, or sending `SIGHUP` to the invoking session, MUST NOT terminate the watchdog or its timer.

D4c. `./deploy.sh` MUST return success from watchdog arming only after `current_pid` identifies a live process group satisfying D4a and `/proc/<current_pid>/cmdline` contains the exact executing script path, `__watchdog`, the current deploy identifier, and the current backup path as consecutive arguments.

D5. While the watchdog is armed, the repository operator MUST be able to disarm it by running `./deploy.sh cancel-watchdog`.

D6. `./deploy.sh cancel-watchdog` MUST:

- terminate the entire process group identified by `current_pid` if and only if its session identity, process start-time identity, command arguments, deploy identifier, and backup path satisfy D4a through D4c;
- poll for process-group exit at most 100 times with 0.02 seconds between polls after `SIGTERM`, then send `SIGKILL` if the process group remains, then repeat the same bounded poll;
- remove the armed deploy identifier and metadata files;
- leave the current deployed binary unchanged.

If the identity checks fail, cancellation MUST treat the files as stale state, remove them, and MUST NOT signal the recorded PID or process group. If the verified process group remains after the bounded `SIGKILL` wait, cancellation MUST remove the state files and exit with failure. A successful cancellation MUST leave no running watchdog or timer process.

D6a. Running `./deploy.sh cancel-watchdog` when no watchdog is armed MUST succeed and leave the deployed binary unchanged.

## 3. Automatic rollback behavior

D7. When the 300-second watchdog timeout expires, the watchdog MUST check whether the same deploy identifier is still armed.

D8. If the deploy identifier is still armed and the recorded backup binary still exists, the watchdog MUST:

1. copy the recorded backup binary to `/opt/monoize/monoize.rollback`;
2. atomically move `/opt/monoize/monoize.rollback` to `/opt/monoize/monoize`;
3. restart PM2 process `monoize`;
4. save PM2 state;
5. clear the watchdog armed state files.

D9. If the timeout expires but either the deploy identifier is no longer armed or the recorded backup binary does not exist, the watchdog MUST exit without modifying the deployed binary.

D10. Automatic rollback MUST NOT reverse database migrations. A binary retained as a rollback target for a deployment that can apply newer migrations MUST implement the forward-compatible startup rule in `database-configuration.spec.md` DB16a through DB16d before that migration-bearing deployment begins.
