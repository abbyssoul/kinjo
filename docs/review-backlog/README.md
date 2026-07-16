# Kinjo Review Backlog

This directory turns the July 2026 code review into agent-ready implementation
tasks. It is intentionally more explicit than a normal issue list: each task
records why the behavior matters, source evidence, constraints, tests, and its
Definition of Done (DoD).

Read [`CONTEXT.md`](CONTEXT.md) before claiming any task.

## Agent Workflow

1. Check this index and choose a `ready` task whose dependencies are `done`.
2. Read the entire task and the shared context, then re-check its source evidence
   against the current branch. Line numbers are evidence anchors, not authority.
3. Inspect `git status --short`. Preserve unrelated work and coordinate before
   editing files listed by another `in-progress` task.
4. Change the task status to `in-progress` and add an owner/branch in its task
   metadata before implementation.
5. Keep the change inside the stated scope. If an assumption is invalid, record
   the evidence in the task and stop rather than silently broadening it.
6. Add regression tests through the affected module's interface, update relevant
   user documentation, and run the task's validation commands.
7. Fill in the completion record, change status to `done`, and update this table.

Status values:

- `ready`: can be claimed now.
- `blocked`: waiting on listed dependencies or a recorded decision.
- `in-progress`: claimed; inspect its owner and conflicts before touching overlap.
- `done`: DoD satisfied and completion evidence recorded.

## Task Index

| ID | Priority | Status | Task | Depends on | Likely conflicts |
|---|---|---|---|---|---|
| 001 | P0 | done | [Discovery occurrence identity](tasks/001-discovery-occurrence-identity.md) | — | 002, 006, 008, 010, 014, 015 |
| 002 | P0 | done | [Discovery session and failure semantics](tasks/002-discovery-session-and-failure-semantics.md) | 001, 016 | 003, 008, 015 |
| 003 | P0 | done | [Discovery option validation](tasks/003-discovery-option-validation.md) | 002 | 009, 015 |
| 004 | P0 | done | [Address-predicate correctness](tasks/004-address-predicate-correctness.md) | — | 005, 006 |
| 005 | P1 | done | [Compiled command rules](tasks/005-compiled-command-rules.md) | 004 | 006, 007, 015 |
| 006 | P0 | done | [Grouped action targeting](tasks/006-grouped-action-targeting.md) | 005 | 007, 008, 010 |
| 007 | P1 | ready | [Transactional config reload](tasks/007-transactional-config-reload.md) | 005 | 006, 015 |
| 008 | P0 | done | [Stale picker revalidation](tasks/008-stale-picker-revalidation.md) | 001, 002, 006 | 010, 011, 014, 015 |
| 009 | P1 | ready | [CLI config-directory handling](tasks/009-cli-config-directory-handling.md) | — | 003 |
| 010 | P1 | done | [Mode-aware aggregate views](tasks/010-mode-aware-aggregate-views.md) | 001 | 006, 008, 011, 012, 013, 014, 015 |
| 011 | P1 | ready | [Scrollable pickers](tasks/011-scrollable-pickers.md) | 008, 010 | 012, 013, 014, 015 |
| 012 | P0 | done | [Safe terminal rendering](tasks/012-safe-terminal-rendering.md) | — | 010, 011, 013, 014 |
| 013 | P1 | done | [Keybindings and search consistency](tasks/013-keybindings-and-search-consistency.md) | — | 011, 012, 014 |
| 014 | P2 | ready | [Selection, filter, and layout state](tasks/014-selection-filter-and-layout-state.md) | 010 | 008, 011, 012, 013, 015 |
| 015 | P2 | blocked | [App and RuleEngine refactoring](tasks/015-app-and-rule-engine-refactoring.md) | 001–014, 016 | all prior tasks; run last |
| 016 | P0 | done | [Remove implicit fake fallback](tasks/016-remove-implicit-fake-fallback.md) | — | 002 |
| 017 | P2 | done | [Fake backend heterogeneous SSH row](tasks/017-fake-backend-heterogeneous-ssh-row.md) | 006 | — |
| 018 | P2 | ready | [TUI smoke test in CI](tasks/018-tui-smoke-test-in-ci.md) | — | 019 |
| 019 | P2 | ready | [Fake as a selectable backend](tasks/019-fake-as-a-selectable-backend.md) | — | 018 |

Priority meanings:

- **P0**: correctness or safety; schedule before feature work.
- **P1**: validation, UX correctness, or a refactor needed by later work.
- **P2**: maintainability/deepening after behavior has regression coverage.

## Workstreams and Ordering

```text
Discovery:     016 (minimal fallback removal, immediately shippable)
               001 → 002 → 003   (002 also depends on 016)

Command rules: 004 → 005 → 006 → 008
                         └──────→ 007
                            006 → 017 (makes 006 verifiable by hand)

Verification:  017 → 018 (smoke-test the rendered UI in CI)
               019 (fake becomes a --backend, feature-gated; moves 018's
                    invocation, so serialize the two)

CLI:           009

UI model:      001 → 010 → 014
              008 + 010 → 011
              012
              013

Final depth:   all tasks 001–014, 016 → 015
```

Tasks in separate workstreams may run concurrently when the task metadata does
not list a likely conflict. A likely conflict is not a dependency; it means the
agents should coordinate file ownership or serialize their merges.

## Completion Gate

Every task must satisfy its specific DoD and, unless it documents an environment
limitation, finish with:

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

Tasks changing parsing, entry identity, grouping, or interpolation must also
consider the fuzz targets described in [`CONTRIBUTING.md`](../../CONTRIBUTING.md).
Run or extend the relevant target when the changed behavior is within its oracle.

## Backlog Maintenance

- Keep a task self-contained even when it links to shared context. Future agents
  may receive only the task file.
- Update dependencies/statuses when implementation changes the expected order.
- Record newly discovered scope as a follow-up task instead of hiding it in a
  completion note.
- When a task makes an enduring architectural choice not already in context,
  add an ADR under `docs/adr/` and link it from the task.
