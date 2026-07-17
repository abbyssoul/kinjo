# Task 109 — Library re-entrancy of `run()`

- **Priority**: P2 (library-contract bug)
- **Status**: ready
- **Depends on**: none
- **Likely conflicts**: none

## Problem

`kinjo` is published as a library (`src/lib.rs` exposes `discovery`, `plumber`,
`ui`, and `run`/`process_main`). The composition root `run()` has two
process-global side effects that make a second call in one process
misbehave, even though the `kinjo` binary — which calls it exactly once — never
notices:

1. **`color_eyre::install()`** (`src/lib.rs:72`) returns `Err` if a report
   hook is already installed. `process_exit_code` treats that error as a hard
   failure and returns `1` (`lib.rs:72-75`). So a program that calls
   `kinjo::run()` twice — or calls it after installing its own `color_eyre`
   hook — gets a spurious failure on the second call.
2. **The SIGHUP `RELOAD_REQUESTED` `OnceLock`** (`src/lib.rs:234`, set in
   `sighup::install`, `lib.rs:288-289`). `OnceLock::set` silently no-ops on the
   second call, so a second `App`'s reload flag is never wired up — SIGHUP
   keeps toggling the **first** app's (dropped) flag. A second run in the same
   process silently loses config-reload-on-SIGHUP.

The `run()` doc comment presents it as "the library entrypoint" without stating
it is single-shot per process. Either the contract should say so, or the code
should tolerate re-entry.

## Goal

Make `run()`'s process-global assumptions either honoured (tolerate re-entry)
or explicitly documented as a single-call-per-process contract — chosen
deliberately, not left implicit.

## Suggested approach (agent + owner to pick)

- **Minimal / document (acceptable):** state in the `run` and `process_main`
  doc comments that they own process-global state (the panic/report hook and
  the SIGHUP handler) and must be called at most once per process; direct
  library consumers who want finer control should build an `App` themselves via
  the documented `ui::App` surface (already the recommended extension path,
  `app.rs:141-165`). Cheapest, and arguably correct for a "composition root".
- **Tolerate re-entry (more work):** treat "hook already installed" as success
  rather than failure in `process_exit_code` (a second `color_eyre::install`
  error is not a reason to exit 1 — the hook the caller wanted is present).
  For SIGHUP, replace the `OnceLock` with something that can be re-pointed at
  the current app's flag (e.g. an `Arc<Mutex<Option<Arc<AtomicBool>>>>` the
  handler reads), so a second run rewires reload correctly. Note the handler
  runs in a signal context, so whatever replaces the `OnceLock` must stay
  async-signal-safe to read — an atomic pointer swap, not a mutex the handler
  locks.

Recommendation: at minimum do the first (document); the `color_eyre` half of
the second is a one-line, low-risk improvement worth taking regardless.

## Constraints

- The `kinjo` binary's behaviour must not change: single call, SIGHUP reload
  still works, error reporting unchanged.
- Signal-handler code must remain async-signal-safe (round-1 SIGHUP work,
  `lib.rs:225-297`).
- Do not weaken the hangup-vs-reload discrimination.

## Tests

- If the `color_eyre` change is made: a test that a second `run_with_args`-style
  invocation in one process does not fail *solely* because the hook is already
  installed. (Existing tests already call `run_with_args` multiple times in one
  test binary — confirm none relies on the current exit-1-on-double-install
  behaviour.)
- If SIGHUP is made re-pointable: a unit test that installing a second flag
  routes the signal to the second flag, not the first (extend the existing
  `sighup` test module, `lib.rs:299-333`).

## Definition of Done

- `run()`'s process-global assumptions are either documented as a single-shot
  contract or made re-entrant, deliberately.
- Binary behaviour unchanged; signal safety preserved.
- Completion gate green.
