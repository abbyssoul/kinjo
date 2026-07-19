# TUI crash-capture toolkit

Reusable diagnostics for hunting a crash or signal death in the `tui-smoke`
job. None of it runs by default — the smoke test is green on tmux ≥ 3.5 (see
`docs/tui-smoke-crash-handover.md`). Wire it in only when a new flake appears.

## What each piece does

- **`run-fg.py`** — a foreground-preserving launcher. It becomes the tmux pane
  process, forks the real program into the tty foreground (so a TUI's reads do
  not raise `SIGTTIN`), and records how *both* the child **and** the pane process
  terminated. That distinction is what separates "the app crashed" from "the app
  exited cleanly and tmux signalled the pane during teardown".
- **`enable-core-dumps.sh`** — rewrites `kernel.core_pattern` to plain files and
  primes the tmux server with `ulimit -c unlimited` so pane processes it spawns
  dump core. Run it *before* the smoke test (the server inherits the limit at
  spawn time). CI-only: needs passwordless sudo.
- **`report-crash.sh`** — turns any captured cores into gdb backtraces, and dumps
  `coredumpctl` / `dmesg` / journal fault lines and the app's stderr. Run as an
  `if: failure()` step; never fails the job.

## Wiring it into `tui-smoke` (temporary, for an investigation)

```yaml
    - name: Enable core dumps
      run: scripts/ci/enable-core-dumps.sh
    - name: Smoke-test …
      env:
        KINJO_WRAP: "scripts/ci/run-fg.py /tmp/cores/kinjo-rc.txt --"
      run: scripts/smoke-tui.sh
    - name: Report crash
      if: failure()
      run: scripts/ci/report-crash.sh
```

`KINJO_WRAP` is honoured by `scripts/drive-tui.sh`: it is prepended verbatim
before the binary, so the launcher records the terminating signal. Note that
adding pane-signal handlers via `run-fg.py` can shift timing enough to mask a
delicate race — if the crash vanishes under the wrapper, that itself is a clue.
