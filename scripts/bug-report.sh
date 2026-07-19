#!/usr/bin/env bash
# Run kinjo under light instrumentation and collect what a good bug report needs
# — without sudo, tmux, or changing anything on your system.
#
# It sets RUST_BACKTRACE=full, points kinjo's crash-report writer at an output
# folder, captures stderr, records how kinjo exited, and gathers a little
# environment. Use it when kinjo crashes or misbehaves and you want to file an
# issue: reproduce the problem under this wrapper, then attach the folder.
#
# Usage:  scripts/bug-report.sh [kinjo args...]
#         KINJO_BIN=./target/release/kinjo scripts/bug-report.sh --backend zeroconf
#
# Privacy: kinjo browses your local network, so the captured output can contain
# hostnames, service names, and IP addresses from your LAN. Review the files
# before attaching them to a public issue.
set -euo pipefail

issues_url="${KINJO_ISSUES_URL:-https://github.com/abbyssoul/kinjo/issues}"

bin="${KINJO_BIN:-kinjo}"
if ! command -v "$bin" >/dev/null 2>&1; then
    echo "error: cannot find kinjo (looked for '$bin')." >&2
    echo "Install kinjo, or set KINJO_BIN to its path, e.g." >&2
    echo "    KINJO_BIN=./target/release/kinjo $0 $*" >&2
    exit 127
fi

outdir="kinjo-bug-report-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$outdir"
errlog="$outdir/stderr.log"

# Collect environment up front, so we still have it if the run wedges.
{
    echo "# kinjo bug report — environment"
    echo "date: $(date -Is 2>/dev/null || date)"
    echo "command: $bin $*"
    echo
    echo "## kinjo"
    "$bin" --version 2>&1 || echo "(kinjo --version failed)"
    echo
    echo "## system"
    uname -a 2>/dev/null || true
    echo "TERM=${TERM:-}"
    echo "SHELL=${SHELL:-}"
    echo "locale: LANG=${LANG:-} LC_ALL=${LC_ALL:-}"
    if [ -n "${TMUX:-}" ]; then
        printf 'tmux: '
        tmux -V 2>/dev/null || echo "(unknown)"
    fi
    echo
    echo "## avahi (mDNS daemon)"
    if command -v systemctl >/dev/null 2>&1; then
        echo "avahi-daemon: $(systemctl is-active avahi-daemon 2>/dev/null || echo unknown)"
    else
        echo "(systemctl not present; cannot query avahi-daemon)"
    fi
} >"$outdir/environment.txt" 2>&1

echo "Running: $bin $*"
echo "Reproduce the problem, then quit kinjo as usual."
echo

# Enable core dumps for THIS process only — a per-process soft limit, no global
# core_pattern change and no sudo. On systemd distros the kernel may still route
# the core to systemd-coredump; the notes below say where to find it.
ulimit -c unlimited 2>/dev/null || true

# Let a panic print a full backtrace, and land kinjo's own crash report in the
# output folder next to everything else (see src/crash.rs).
export RUST_BACKTRACE=full
export KINJO_CRASH_DIR="$outdir"

# kinjo drives the terminal on stdin/stdout; capture only stderr so the TUI is
# untouched. A non-zero exit is expected here, so do not let `set -e` abort on it.
set +e
"$bin" "$@" 2>"$errlog"
status=$?
set -e

{
    echo "exit status: $status"
    if [ "$status" -gt 128 ]; then
        signal=$((status - 128))
        echo "terminated by signal: $signal (SIG$(kill -l "$signal" 2>/dev/null || echo '?'))"
    fi
} >"$outdir/exit.txt"

# Surface the captured stderr (a panic report and kinjo's crash-file pointer).
if [ -s "$errlog" ]; then
    echo
    echo "----- captured stderr -----"
    cat "$errlog"
    echo "---------------------------"
fi

echo
echo "Collected a bug report in: $outdir/"
for file in "$outdir"/*; do
    if [ -f "$file" ]; then
        echo "    $(basename "$file")"
    fi
done
echo
echo "Please review these files — they can contain local hostnames, service"
echo "names, and IPs from your network — then attach the folder to a bug report:"
echo "    $issues_url"
if [ "$status" -gt 128 ] && command -v coredumpctl >/dev/null 2>&1; then
    echo
    echo "kinjo was killed by a signal. If your system uses systemd-coredump, a"
    echo "core may be available:  coredumpctl info $bin"
fi
