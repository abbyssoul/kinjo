#!/usr/bin/env bash
# Turn any core dumps left by a smoke-test crash into readable backtraces, plus
# the kernel's own fault lines. Meant to run as an `if: failure()` CI step.
#
# Never fails the job: this is diagnostic output, not a gate. Pass the binary to
# symbolicate as $1 (defaults to the debug build the smoke driver builds).
set -uo pipefail

binary="${1:-target/debug/kinjo}"
cores_dir="${CORE_DUMP_DIR:-/tmp/cores}"

echo "::group::kernel fault log (dmesg)"
sudo dmesg 2>/dev/null | grep -iE 'segfault|general protection|trap|core dump' | tail -40 \
    || echo "(no kernel fault lines; dmesg may be restricted)"
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
