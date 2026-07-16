# 018: Smoke-Test the Rendered UI in CI

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P2` |
| Workstream | UI / CI |
| Depends on | 017, 019 |
| Likely conflicts | 015 |
| Owner | Unclaimed |

## Why This Matters

Nothing in CI ever runs the application. The gate compiles it, unit-tests it,
and fuzzes its parsers, but no job starts the binary and looks at a frame. A
change that renders nothing, panics on first draw, or leaves the terminal
unusable would pass every check.

`scripts/drive-tui.sh` closed the local half of this gap: it runs the app in a
detached tmux pane against the sample backend and prints the rendered screen.
That is now reproducible enough to automate, and CI is where a rendering
regression should be caught rather than in a reviewer's memory.

This is a *nice to have*. It buys a shallow but broad guarantee — the app starts,
draws, responds to a key, and exits cleanly — that no current job provides.

## Evidence

- `.github/workflows/ci-test.yml`: the test workflow builds, lints, and tests;
  no job executes the binary's TUI.
- `.github/workflows/fuzz.yml`: exercises parsers, not rendering.
- `scripts/drive-tui.sh`: the local driver, defaulting to `--backend fake
  --config-dir actions`, with `KINJO_COLS`/`KINJO_ROWS` for size.
- Tasks 006 and 008 shipped verified by regression tests alone; task 017 added
  the sample-set affordance that makes their behavior reachable at all.
- `docs/review-backlog/CONTEXT.md`, *Baseline and Validation*: driving the real
  app is now expected of UI-affecting tasks. CI does not enforce it.

Midpoint validation on 2026-07-16 successfully drove the committed helper at
100×30 and 60×18, including opening the heterogeneous SSH picker added by task
017. The local seam is healthy; CI still never invokes it. Task 019 now precedes
this task so the smoke job is written once against the final `--backend fake`
interface instead of immediately rewriting a legacy-flag invocation.

## Required Outcome

- A CI job runs the real binary against the sample backend and asserts on what it
  renders, failing the build when it does not.
- The assertions are shallow and stable by design. Suggested floor:
  - the app starts, draws a frame, and the frame contains the expected chrome;
  - sample services appear in the list;
  - a key press changes the screen (for example switching view tabs);
  - the app exits cleanly on the quit key, with a zero status.
- At least one non-default terminal size is exercised, since layout bugs live in
  narrow and short terminals.
- The job is not flaky. Waits are bounded and explicit; a slow runner must not
  produce a red build on a healthy tree.
- A failure prints the captured screen, so the log alone explains the failure.
- The job reuses `scripts/drive-tui.sh` rather than reimplementing pty driving,
  so local and CI verification cannot drift.

## Implementation Constraints

- Install `tmux` in the job explicitly; do not assume the runner image has it.
- Never assert against a live network. The sample backend is the only supported
  source for this job.
- Keep it quick: this is a smoke test, not a UI test suite. Do not encode exact
  pixel-for-pixel frames — a brittle golden frame would be reverted within a
  month and teach everyone to distrust the job.
- Assert on stable substrings, not on decoration that a styling change would move.
- `scripts/drive-tui.sh` is the shared seam. Extend it if the job needs something
  it cannot express (an exit-status mode, a bounded wait-for-text); do not fork
  its logic into YAML.
- Lint any workflow change with the existing `actionlint` workflow.

## Suggested Implementation Sequence

1. Decide whether the assertions belong in the workflow or in a small committed
   script the workflow calls. Prefer the script: it can be run locally too.
2. Add the smoke run to CI with tmux installed and a bounded timeout.
3. Add a second run at a small terminal size.
4. Prove it fails: break rendering locally, confirm red, restore.
5. Note the job in `CONTRIBUTING.md` beside the driver it uses.

## Non-Goals

- Golden-frame or snapshot testing of the whole UI.
- Testing against real mDNS traffic in CI.
- Replacing the unit/integration tests that already cover app behavior.
- Building a general TUI test framework.

## Acceptance Criteria / Definition of Done

- [ ] CI runs the real binary and asserts on rendered output.
- [ ] The job fails when rendering is broken, demonstrated deliberately.
- [ ] A non-default terminal size is covered.
- [ ] Failure output includes the captured screen.
- [ ] Waits are bounded; the job does not flake on a healthy tree.
- [ ] `actionlint` passes and `CONTRIBUTING.md` mentions the job.

## Required Tests

The job is the test. Verify it by hand both ways before marking this done: a
healthy tree goes green, and a deliberately broken frame goes red with the
captured screen in the log.

## Validation

```sh
scripts/drive-tui.sh run 'Tab Tab Down Down Down Enter'
KINJO_COLS=60 KINJO_ROWS=18 scripts/drive-tui.sh run 'Down'
# Then the job itself, on a branch, both green and deliberately red.
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
