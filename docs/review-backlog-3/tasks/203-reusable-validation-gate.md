# Task 203 — Make CI checks reusable as a release gate

| Field | Value |
|---|---|
| Status | `in-progress` |
| Priority | `P1` |
| Workstream | Validation |
| Depends on | — |
| Likely conflicts | 204, 207, 208 |
| Owner | Codex (repository implementation) |

## Required Outcome

Add `workflow_call` to Rust CI, audit, Nix, and actionlint without changing
their normal triggers. A release caller must wait for all four suites on its
exact SHA. Extract release validation into locally testable shell helpers.

Expand Nix path coverage to build-relevant source/configuration and build the
package natively on both Linux architectures. Add explicit timeouts and retain
all matrix results with `fail-fast: false`.

## Required Tests

- `actionlint` accepts all direct and reusable triggers;
- helper fixtures cover version/ref/SHA failures;
- an ordinary PR still receives the same checks;
- a release dry run waits for each called suite;
- native x86_64 and AArch64 Nix builds execute `kinjo --version`.

## Definition of Done

- [x] No validation suite is copied into the release workflow.
- [x] Called workflows use the caller SHA.
- [x] Timeouts are explicit.
- [x] Nix sandbox regressions from source changes cannot bypass Nix CI.

## Completion Record

- **Implemented:** Rust CI, audit, Nix, and actionlint now accept
  `workflow_call`; Release waits for each. Nix has broad build-path triggers
  and native x86_64/AArch64 jobs.
- **Correction (review of this task):** the four suites originally keyed their
  concurrency on `github.event_name == 'workflow_call'`, which never matches —
  inside a called workflow the `github` context is the caller's, so
  `event_name` is the caller's trigger. The guard was inert and each called run
  joined `<suite>-refs/heads/main` with `cancel-in-progress: true`, so a push to
  `main` mid-release could cancel the release's own gate. Each suite now takes a
  `caller-run-id` input and Release passes `github.run_id`.
- **Tests:** Release identity/version logic is covered by local shell tests.
- **Validation:** Local actionlint, ShellCheck, clippy, and default/all-feature
  tests pass. Called-workflow and AArch64 Nix execution require a GitHub run.
