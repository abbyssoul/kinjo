#!/usr/bin/env bash
# Configure the CI runner to write core dumps as plain files, and prime the tmux
# server so processes it launches (the smoke-tested TUI) dump core on a fatal
# signal. Pairs with report-crash.sh, which turns those cores into backtraces.
#
# CI-only: needs passwordless sudo and rewrites kernel.core_pattern for the job.
set -euo pipefail

cores_dir="${CORE_DUMP_DIR:-/tmp/cores}"

sudo mkdir -p "$cores_dir"
sudo chmod 1777 "$cores_dir"

# Override apport/systemd-coredump so cores land where we can read them.
# %e = executable name, %p = pid, %s = terminating signal number.
echo "$cores_dir/core.%e.%p.%s" | sudo tee /proc/sys/kernel/core_pattern >/dev/null

# The tmux server daemon inherits this soft limit at spawn time and passes it to
# every pane process, so the smoke driver (which reuses this server) launches the
# TUI with cores enabled. Setting it only in a later step would be too late: the
# server would already be running with the default limit of 0.
ulimit -c unlimited
tmux kill-server 2>/dev/null || true
tmux new-session -d -s _coredump_keepalive 'sleep 3600'

echo "core_pattern=$(cat /proc/sys/kernel/core_pattern)"
echo "core ulimit: $(ulimit -c)"
