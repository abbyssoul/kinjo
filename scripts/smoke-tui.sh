#!/usr/bin/env bash
# Shallow, stable smoke assertions against the real rendered Kinjo TUI.
set -euo pipefail

cd "$(dirname "$0")/.."

DRIVER="scripts/drive-tui.sh"
export KINJO_SESSION="${KINJO_SESSION:-kinjo-smoke-$$}"
export KINJO_STARTUP="${KINJO_STARTUP:-0}"
export KINJO_SETTLE="${KINJO_SETTLE:-0}"
export KINJO_TIMEOUT="${KINJO_TIMEOUT:-15}"

cleanup() {
    "$DRIVER" stop >/dev/null 2>&1 || true
}
trap cleanup EXIT

fail() {
    printf 'tui-smoke: %s\n' "$*" >&2
    "$DRIVER" shot >&2 || true
    exit 1
}

assert_contains() {
    local screen="$1"
    local expected="$2"
    [[ "$screen" == *"$expected"* ]] ||
        fail "rendered screen did not contain '$expected'"
}

"$DRIVER" start
initial=$("$DRIVER" wait-text "services 1-5/5")
assert_contains "$initial" "kinjo"
assert_contains "$initial" "services 1-5/5"
assert_contains "$initial" "workstation"

"$DRIVER" keys Tab Tab
types=$("$DRIVER" wait-text "types 1-4/4")
assert_contains "$types" "_http._tcp"
[[ "$types" != "$initial" ]] || fail "switching view tabs did not change the screen"

"$DRIVER" keys q
"$DRIVER" wait-exit >/dev/null

# A closed terminal must take the app with it.
#
# Kinjo handles SIGHUP so it can reload its command files, which also replaces
# SIGHUP's default action of terminating. That makes "my terminal hung up" and
# "re-read your config" the same signal, and reading a hangup as a reload is not
# survivable: the event loop returns to a terminal that no longer exists, and
# crossterm's input source busy-reads the EOF a dead tty reports and never
# returns. Nothing downstream gets to notice, so the app is left orphaned,
# spinning at 100% CPU, outliving the terminal that started it.
#
# Quitting with `q` above cannot catch that — it exits through the normal path.
# Only closing the terminal underneath the app does.
"$DRIVER" start
app_pid=$("$DRIVER" pid)
[[ -n "$app_pid" ]] || fail "could not resolve the app's pid"
"$DRIVER" stop

deadline=$((SECONDS + 10))
while true; do
    # `|| true`: once the process is gone `ps` exits non-zero, and that is the
    # answer this is waiting for rather than an error to die on.
    state=$(ps -o stat= -p "$app_pid" 2>/dev/null | tr -d ' ' || true)
    # A reaped process has no state at all; a zombie has already exited and is
    # only waiting to be collected. Either is dead for our purposes.
    if [[ -z "$state" || "$state" == Z* ]]; then
        break
    fi
    ((SECONDS < deadline)) ||
        fail "kinjo (pid $app_pid, state $state) outlived its terminal"
    sleep 0.1
done

printf 'tui-smoke: PASS at %sx%s\n' "${KINJO_COLS:-100}" "${KINJO_ROWS:-30}"
