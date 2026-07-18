#!/usr/bin/env bash
# Turn any core dumps left by a smoke-test crash into readable backtraces, plus
# the kernel's own fault lines. Meant to run as an `if: failure()` CI step.
#
# Never fails the job: this is diagnostic output, not a gate. Pass the binary to
# symbolicate as $1 (defaults to the debug build the smoke driver builds).
set -uo pipefail

binary="${1:-target/debug/kinjo}"
cores_dir="${CORE_DUMP_DIR:-/tmp/cores}"

echo "::group::signal / kill evidence"
echo "core_pattern now: $(cat /proc/sys/kernel/core_pattern 2>/dev/null)"
echo "core ulimit a pane process saw: $(cat /tmp/cores/tmux-core-ulimit.txt 2>/dev/null || echo '?')"
echo "-- dmesg faults/kills --"
sudo dmesg 2>/dev/null | grep -iE 'segfault|general protection|trap|core dump|killed process|out of memory|oom' | tail -40 \
    || echo "(no kernel fault lines; dmesg may be restricted)"
echo "-- journal: anything that killed kinjo (OOM, etc.) --"
sudo journalctl --no-pager --since '-5min' 2>/dev/null | grep -iE 'kinjo|killed process|oom|segfault|traps' | tail -20 \
    || echo "(journal unavailable)"
echo "::endgroup::"

# On systemd runners the kernel may route cores to systemd-coredump regardless of
# core_pattern; it records the terminating signal and can hand a core to gdb.
echo "::group::systemd-coredump (coredumpctl)"
if command -v coredumpctl >/dev/null; then
    sudo coredumpctl list --no-pager 2>&1 | tail -15 || echo "(coredumpctl: nothing)"
    # `info` prints the terminating signal and, with debuginfo, a stack trace.
    sudo coredumpctl info kinjo 2>&1 | grep -iE 'signal|command|executable|storage|^ *#[0-9]' | head -40 \
        || echo "(no systemd core for kinjo)"
    if sudo coredumpctl --output="$cores_dir/core.systemd-kinjo" dump kinjo 2>/dev/null; then
        echo "extracted a systemd-managed core to $cores_dir/core.systemd-kinjo"
    fi
else
    echo "(coredumpctl not present)"
fi
echo "::endgroup::"

echo "::group::captured app stderr"
# The smoke driver writes the TUI's stderr here; a panic/abort message would show.
cat "${KINJO_ERRLOG:-/tmp/kinjo-app-stderr.log}" 2>/dev/null || echo "(none)"
echo "::endgroup::"

shopt -s nullglob
cores=("$cores_dir"/core.*)
if [ "${#cores[@]}" -eq 0 ]; then
    echo "no core dumps in $cores_dir — the process may have been killed by a"
    echo "non-dumping signal (e.g. SIGKILL) or core dumps were not enabled."
    exit 0
fi

if ! command -v gdb >/dev/null; then
    echo "::error::gdb not installed; cannot backtrace ${#cores[@]} core(s)"
    exit 0
fi

for core in "${cores[@]}"; do
    # Filename is core.<exe>.<pid>.<signal>; surface the terminating signal.
    sig="${core##*.}"
    echo "::group::core $core (terminating signal $sig)"
    file "$core" || true
    gdb --batch --nx \
        -ex 'set pagination off' \
        -ex 'info registers rip' \
        -ex 'thread apply all backtrace' \
        "$binary" "$core" 2>&1 | head -300
    echo "::endgroup::"
done
