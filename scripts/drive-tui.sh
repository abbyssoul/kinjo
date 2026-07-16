#!/usr/bin/env bash
#
# Drive the kinjo TUI in a detached tmux pane and print what it renders.
#
# Tests assert what the app computes; only running it shows what a person
# actually sees. This makes that cheap and repeatable — for a human, or for an
# agent that cannot look at a screen.
#
# Usage:
#   scripts/drive-tui.sh start [-- kinjo-args...]        # build, then start
#   scripts/drive-tui.sh keys <key> [key ...]            # send keys, then settle
#   scripts/drive-tui.sh shot                            # print the screen
#   scripts/drive-tui.sh wait-text <text> [seconds]      # wait for rendered text
#   scripts/drive-tui.sh wait-exit [seconds]             # require a clean exit
#   scripts/drive-tui.sh stop                            # end the session
#   scripts/drive-tui.sh run '<keys>' [-- kinjo-args...] # all of the above
#
# Keys are tmux key names: Tab BTab Enter Escape Down Up Left Right Space, or a
# literal character such as q or /. `sleep:N` pauses N seconds mid-sequence, for
# a UI that is still waiting on discovery.
#
# Examples:
#   # Does an aggregate row ask which host to act on, or pick one silently?
#   scripts/drive-tui.sh run 'Tab Tab Down Down Down Enter'
#
#   # Or step through it, looking after each move:
#   scripts/drive-tui.sh start
#   scripts/drive-tui.sh keys Tab Tab
#   scripts/drive-tui.sh shot
#   scripts/drive-tui.sh stop
#
#   # A real backend, a custom rule set, a narrow terminal:
#   KINJO_COLS=60 KINJO_ROWS=20 scripts/drive-tui.sh run 'Down' -- --config-dir /tmp/rules
#
# Environment:
#   KINJO_SESSION   tmux session name (default: kinjo-drive)
#   KINJO_COLS      pane width  (default: 100)
#   KINJO_ROWS      pane height (default: 30)
#   KINJO_SETTLE    seconds to wait after a key batch (default: 0.6)
#   KINJO_STARTUP   seconds to wait after start, for discovery (default: 2.5)
#   KINJO_TIMEOUT   maximum wait-text/wait-exit seconds (default: 10)
#
# The default arguments are `--backend fake --config-dir actions`: the sample
# backend plus the bundled rules, which is the reproducible way to exercise the
# UI without a live network. The driver builds with the off-by-default `fake`
# feature. Pass your own after `--` to override the runtime arguments.
set -euo pipefail

cd "$(dirname "$0")/.."

SESSION="${KINJO_SESSION:-kinjo-drive}"
COLS="${KINJO_COLS:-100}"
ROWS="${KINJO_ROWS:-30}"
SETTLE="${KINJO_SETTLE:-0.6}"
STARTUP="${KINJO_STARTUP:-2.5}"
TIMEOUT="${KINJO_TIMEOUT:-10}"
BIN="target/debug/kinjo"
DEFAULT_ARGS=(--backend fake --config-dir actions)

die() {
    echo "drive-tui: $*" >&2
    exit 1
}

usage() {
    sed -n '3,/^set -euo pipefail$/p' "$0" | sed '$d; s/^# \{0,1\}//'
}

require_session() {
    tmux has-session -t "$SESSION" 2>/dev/null ||
        die "no session '$SESSION' — run: scripts/drive-tui.sh start"
}

cmd_start() {
    local args=("$@")
    if [[ ${#args[@]} -eq 0 ]]; then
        args=("${DEFAULT_ARGS[@]}")
    fi

    command -v tmux >/dev/null || die "tmux is required"
    cargo build --locked -q --features fake || die "build failed"
    [[ -x "$BIN" ]] || die "no binary at $BIN"

    tmux kill-session -t "$SESSION" 2>/dev/null || true
    # Quote every argument: a rule path or service type may contain anything.
    local command
    command=$(printf '%q ' "$BIN" "${args[@]}")
    tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" "$command"
    # Keep the pane after the app exits, so the last frame — a handoff, a crash,
    # an error — is still there to look at.
    tmux set-option -t "$SESSION" remain-on-exit on >/dev/null
    sleep "$STARTUP"
}

cmd_keys() {
    require_session
    local key
    for key in "$@"; do
        case "$key" in
            sleep:*) sleep "${key#sleep:}" ;;
            *) tmux send-keys -t "$SESSION" "$key" ;;
        esac
    done
    sleep "$SETTLE"
}

cmd_shot() {
    require_session
    tmux capture-pane -p -t "$SESSION"
}

wait_timeout() {
    local value="${1:-$TIMEOUT}"
    if ! [[ "$value" =~ ^[0-9]+$ ]] || ((value <= 0)); then
        die "timeout must be a positive whole number of seconds"
    fi
    printf '%s' "$value"
}

pane_dead() {
    tmux display-message -p -t "$SESSION" '#{pane_dead}'
}

pane_status() {
    tmux display-message -p -t "$SESSION" '#{pane_dead_status}'
}

cmd_wait_text() {
    require_session
    local expected="${1-}"
    [[ -n "$expected" ]] || die "wait-text requires text to find"
    local timeout
    timeout=$(wait_timeout "${2-}")
    local deadline=$((SECONDS + timeout))
    local screen

    while ((SECONDS <= deadline)); do
        screen=$(cmd_shot)
        if [[ "$screen" == *"$expected"* ]]; then
            printf '%s\n' "$screen"
            return 0
        fi
        if [[ "$(pane_dead)" == "1" ]]; then
            printf '%s\n' "$screen" >&2
            die "pane exited with status $(pane_status) before rendering '$expected'"
        fi
        sleep 0.1
    done

    printf '%s\n' "$screen" >&2
    die "timed out after ${timeout}s waiting for '$expected'"
}

cmd_wait_exit() {
    require_session
    local timeout
    timeout=$(wait_timeout "${1-}")
    local deadline=$((SECONDS + timeout))

    while ((SECONDS <= deadline)); do
        if [[ "$(pane_dead)" == "1" ]]; then
            local status
            status=$(pane_status)
            if [[ "$status" != "0" ]]; then
                cmd_shot >&2
                die "pane exited with status $status"
            fi
            printf '%s\n' "$status"
            return 0
        fi
        sleep 0.1
    done

    cmd_shot >&2
    die "timed out after ${timeout}s waiting for the pane to exit"
}

cmd_stop() {
    tmux kill-session -t "$SESSION" 2>/dev/null || true
}

cmd_run() {
    local keys_string="${1-}"
    shift || true
    if [[ "${1-}" == "--" ]]; then
        shift
    fi

    # `run` owns the session it makes, however it ends.
    trap cmd_stop EXIT
    cmd_start "$@"

    local keys=()
    read -ra keys <<<"$keys_string"
    if [[ ${#keys[@]} -gt 0 ]]; then
        cmd_keys "${keys[@]}"
    fi
    cmd_shot
}

subcommand="${1-}"
shift || true
case "$subcommand" in
    start)
        if [[ "${1-}" == "--" ]]; then shift; fi
        cmd_start "$@"
        ;;
    keys) cmd_keys "$@" ;;
    shot) cmd_shot ;;
    wait-text) cmd_wait_text "$@" ;;
    wait-exit) cmd_wait_exit "$@" ;;
    stop) cmd_stop ;;
    run) cmd_run "$@" ;;
    -h | --help | help | "") usage ;;
    *) die "unknown subcommand '$subcommand' (try --help)" ;;
esac
