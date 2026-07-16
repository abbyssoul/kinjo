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

printf 'tui-smoke: PASS at %sx%s\n' "${KINJO_COLS:-100}" "${KINJO_ROWS:-30}"
