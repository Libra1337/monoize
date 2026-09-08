#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
declare -a TEST_ROOTS=()
declare -a TEST_PROCESS_GROUPS=()

fail() {
    printf 'not ok - %s\n' "$1" >&2
    exit 1
}

cleanup() {
    local process_group
    for process_group in "${TEST_PROCESS_GROUPS[@]}"; do
        if [[ "$process_group" =~ ^[0-9]+$ ]] && [ "$process_group" -gt 1 ]; then
            kill -KILL -- "-$process_group" 2>/dev/null || true
        fi
    done

    local test_root
    for test_root in "${TEST_ROOTS[@]}"; do
        case "$test_root" in
            "$REPO_ROOT"/.deploy-watchdog-test.*)
                rm -rf -- "$test_root"
                ;;
        esac
    done
}
trap cleanup EXIT

wait_for_file() {
    local path="$1"
    local attempt
    for ((attempt = 0; attempt < 200; attempt++)); do
        [ -f "$path" ] && return 0
        sleep 0.05
    done
    return 1
}

wait_for_empty_directory() {
    local path="$1"
    local attempt
    for ((attempt = 0; attempt < 200; attempt++)); do
        if [ -d "$path" ] && [ -z "$(find "$path" -mindepth 1 -maxdepth 1 -print -quit)" ]; then
            return 0
        fi
        sleep 0.05
    done
    return 1
}

assert_process_group_dead() {
    local process_group="$1"
    if kill -0 -- "-$process_group" 2>/dev/null; then
        fail "process group $process_group is still alive"
    fi
}

wait_for_process_group_exit() {
    local process_group="$1"
    local attempt
    for ((attempt = 0; attempt < 100; attempt++)); do
        if ! kill -0 -- "-$process_group" 2>/dev/null; then
            return 0
        fi
        sleep 0.02
    done
    return 1
}

setup_fixture() {
    FIXTURE_ROOT="$(mktemp -d "$REPO_ROOT/.deploy-watchdog-test.XXXXXX")"
    TEST_ROOTS+=("$FIXTURE_ROOT")
    mkdir -p \
        "$FIXTURE_ROOT/frontend" \
        "$FIXTURE_ROOT/target/release" \
        "$FIXTURE_ROOT/deploy" \
        "$FIXTURE_ROOT/fake-bin"
    cp "$REPO_ROOT/deploy.sh" "$FIXTURE_ROOT/deploy.sh"
    chmod +x "$FIXTURE_ROOT/deploy.sh"
    printf 'old-binary\n' > "$FIXTURE_ROOT/deploy/monoize"
    printf 'new-binary\n' > "$FIXTURE_ROOT/target/release/monoize"
    : > "$FIXTURE_ROOT/commands.log"
    : > "$FIXTURE_ROOT/pm2.log"

    cat > "$FIXTURE_ROOT/fake-bin/bun" <<'EOF'
#!/usr/bin/env bash
printf 'bun %s\n' "$*" >> "${MONOIZE_DEPLOY_TEST_COMMAND_LOG:?}"
EOF
    cat > "$FIXTURE_ROOT/fake-bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >> "${MONOIZE_DEPLOY_TEST_COMMAND_LOG:?}"
EOF
    cat > "$FIXTURE_ROOT/fake-bin/pm2" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${MONOIZE_DEPLOY_TEST_PM2_LOG:?}"
EOF
    chmod +x \
        "$FIXTURE_ROOT/fake-bin/bun" \
        "$FIXTURE_ROOT/fake-bin/cargo" \
        "$FIXTURE_ROOT/fake-bin/pm2"
}

run_fixture_deploy() {
    local fixture_root="$1"
    local timeout_seconds="$2"
    shift 2
    env \
        PATH="$fixture_root/fake-bin:$PATH" \
        MONOIZE_DEPLOY_TEST_MODE=1 \
        MONOIZE_DEPLOY_TEST_ROOT="$fixture_root" \
        MONOIZE_DEPLOY_TEST_WATCHDOG_TIMEOUT_SECONDS="$timeout_seconds" \
        MONOIZE_DEPLOY_TEST_COMMAND_LOG="$fixture_root/commands.log" \
        MONOIZE_DEPLOY_TEST_PM2_LOG="$fixture_root/pm2.log" \
        "$fixture_root/deploy.sh" "$@"
}

test_watchdog_survives_caller_session_and_rolls_back() {
    setup_fixture
    local fixture_root="$FIXTURE_ROOT"
    local caller_pid_file="$fixture_root/caller.pid"
    local caller_ready_file="$fixture_root/caller.ready"

    env \
        PATH="$fixture_root/fake-bin:$PATH" \
        MONOIZE_DEPLOY_TEST_MODE=1 \
        MONOIZE_DEPLOY_TEST_ROOT="$fixture_root" \
        MONOIZE_DEPLOY_TEST_WATCHDOG_TIMEOUT_SECONDS=2 \
        MONOIZE_DEPLOY_TEST_COMMAND_LOG="$fixture_root/commands.log" \
        MONOIZE_DEPLOY_TEST_PM2_LOG="$fixture_root/pm2.log" \
        CALLER_PID_FILE="$caller_pid_file" \
        CALLER_READY_FILE="$caller_ready_file" \
        DEPLOY_SCRIPT="$fixture_root/deploy.sh" \
        setsid bash -c '
            printf "%s\n" "$BASHPID" > "$CALLER_PID_FILE"
            "$DEPLOY_SCRIPT" deploy-watchdog > "${CALLER_READY_FILE}.output" 2>&1
            : > "$CALLER_READY_FILE"
            while :; do sleep 1; done
        ' &
    local caller_launcher_pid="$!"

    wait_for_file "$caller_ready_file" || fail "deploy did not arm watchdog"
    wait_for_file "$fixture_root/deploy/.deploy-watchdog/current_pid" \
        || fail "watchdog PID was not published"
    local caller_pid watchdog_pid caller_session watchdog_session watchdog_group
    caller_pid="$(<"$caller_pid_file")"
    watchdog_pid="$(<"$fixture_root/deploy/.deploy-watchdog/current_pid")"
    read -r caller_session < <(ps -o sid= -p "$caller_pid")
    read -r watchdog_session watchdog_group \
        < <(ps -o sid=,pgid= -p "$watchdog_pid")
    [ "$watchdog_session" = "$watchdog_pid" ] \
        || fail "watchdog is not a session leader"
    [ "$watchdog_group" = "$watchdog_pid" ] \
        || fail "watchdog is not a process-group leader"
    [ "$watchdog_session" != "$caller_session" ] \
        || fail "watchdog remained in the caller session"
    TEST_PROCESS_GROUPS+=("$caller_pid" "$watchdog_pid")

    kill -HUP -- "-$caller_pid"
    wait "$caller_launcher_pid" 2>/dev/null || true
    sleep 0.1
    kill -0 "$watchdog_pid" 2>/dev/null \
        || fail "watchdog died when the caller session closed"

    wait_for_empty_directory "$fixture_root/deploy/.deploy-watchdog" \
        || fail "watchdog did not finish rollback and clear state"
    wait_for_process_group_exit "$watchdog_pid" \
        || fail "watchdog process group did not exit after rollback"
    assert_process_group_dead "$watchdog_pid"
    [ "$(<"$fixture_root/deploy/monoize")" = "old-binary" ] \
        || fail "watchdog did not restore the backup binary"
    [ "$(grep -c '^restart monoize$' "$fixture_root/pm2.log")" -eq 2 ] \
        || fail "automatic rollback did not restart PM2 exactly once"
    printf 'ok - watchdog survives caller session and rolls back\n'
}

test_cancel_kills_watchdog_and_timer() {
    setup_fixture
    local fixture_root="$FIXTURE_ROOT"
    run_fixture_deploy "$fixture_root" 3 deploy-watchdog > "$fixture_root/deploy.output"

    local watchdog_pid timer_pid
    watchdog_pid="$(<"$fixture_root/deploy/.deploy-watchdog/current_pid")"
    timer_pid="$(pgrep -P "$watchdog_pid" | head -n 1)"
    [ -n "$timer_pid" ] || fail "watchdog timer child was not running"
    TEST_PROCESS_GROUPS+=("$watchdog_pid")

    run_fixture_deploy "$fixture_root" 3 cancel-watchdog \
        > "$fixture_root/cancel.output"
    wait_for_empty_directory "$fixture_root/deploy/.deploy-watchdog" \
        || fail "cancel did not clear watchdog state"
    assert_process_group_dead "$watchdog_pid"
    kill -0 "$timer_pid" 2>/dev/null \
        && fail "cancel left the timer child alive"
    sleep 4
    [ "$(<"$fixture_root/deploy/monoize")" = "new-binary" ] \
        || fail "cancel changed the deployed binary"
    [ "$(grep -c '^restart monoize$' "$fixture_root/pm2.log")" -eq 1 ] \
        || fail "cancel allowed a later rollback restart"
    printf 'ok - cancel kills watchdog and timer\n'
}

test_stale_reused_pid_is_not_signalled() {
    setup_fixture
    local fixture_root="$FIXTURE_ROOT"
    local unrelated_pid_file="$fixture_root/unrelated.pid"
    setsid bash -c '
        printf "%s\n" "$BASHPID" > "$1"
        exec sleep 30
    ' _ "$unrelated_pid_file" &
    local unrelated_launcher_pid="$!"
    wait_for_file "$unrelated_pid_file" || fail "unrelated process did not start"
    local unrelated_pid unrelated_identity
    unrelated_pid="$(<"$unrelated_pid_file")"
    unrelated_identity="$(awk '{ print $22 }' "/proc/$unrelated_pid/stat")"
    TEST_PROCESS_GROUPS+=("$unrelated_pid")

    local watchdog_dir="$fixture_root/deploy/.deploy-watchdog"
    mkdir -p "$watchdog_dir"
    printf 'stale-deploy-id\n' > "$watchdog_dir/current_id"
    printf '%s\n' "$unrelated_pid" > "$watchdog_dir/current_pid"
    printf '%s\n' "$unrelated_identity" > "$watchdog_dir/current_identity"
    printf '%s\n' "$fixture_root/deploy/monoize.bak.stale" \
        > "$watchdog_dir/current_backup"

    run_fixture_deploy "$fixture_root" 3 cancel-watchdog \
        > "$fixture_root/stale-cancel.output"
    kill -0 "$unrelated_pid" 2>/dev/null \
        || fail "cancel signalled an unrelated reused PID"
    wait_for_empty_directory "$watchdog_dir" \
        || fail "cancel did not clear stale watchdog state"

    kill -KILL -- "-$unrelated_pid" 2>/dev/null || true
    wait "$unrelated_launcher_pid" 2>/dev/null || true
    printf 'ok - stale reused PID is not signalled\n'
}

test_short_timeout_requires_explicit_test_mode() {
    setup_fixture
    local fixture_root="$FIXTURE_ROOT"
    if env \
        PATH="$fixture_root/fake-bin:$PATH" \
        MONOIZE_DEPLOY_TEST_ROOT="$fixture_root" \
        MONOIZE_DEPLOY_TEST_WATCHDOG_TIMEOUT_SECONDS=1 \
        MONOIZE_DEPLOY_TEST_COMMAND_LOG="$fixture_root/commands.log" \
        MONOIZE_DEPLOY_TEST_PM2_LOG="$fixture_root/pm2.log" \
        "$fixture_root/deploy.sh" > "$fixture_root/unsafe.output" 2>&1; then
        fail "unsafe timeout override was accepted"
    fi
    [ ! -s "$fixture_root/commands.log" ] \
        || fail "unsafe timeout override reached a build command"
    [ ! -s "$fixture_root/pm2.log" ] \
        || fail "unsafe timeout override reached PM2"
    printf 'ok - short timeout requires explicit test mode\n'
}

test_default_deploy_does_not_arm_watchdog() {
    setup_fixture
    local fixture_root="$FIXTURE_ROOT"
    run_fixture_deploy "$fixture_root" 3 > "$fixture_root/deploy.output"

    wait_for_empty_directory "$fixture_root/deploy/.deploy-watchdog" \
        || fail "default deploy left watchdog state"
    [ "$(<"$fixture_root/deploy/monoize")" = "new-binary" ] \
        || fail "default deploy did not install the new binary"
    [ "$(grep -c '^restart monoize$' "$fixture_root/pm2.log")" -eq 1 ] \
        || fail "default deploy did not restart PM2 exactly once"
    printf 'ok - default deploy does not arm watchdog\n'
}

test_watchdog_survives_caller_session_and_rolls_back
test_cancel_kills_watchdog_and_timer
test_stale_reused_pid_is_not_signalled
test_short_timeout_requires_explicit_test_mode
test_default_deploy_does_not_arm_watchdog
