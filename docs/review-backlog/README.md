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
| 007 | P1 | done | [Transactional config reload](tasks/007-transactional-config-reload.md) | 005 | 006, 015, 020 |
| 008 | P0 | done | [Stale picker revalidation](tasks/008-stale-picker-revalidation.md) | 001, 002, 006 | 010, 011, 014, 015 |
| 009 | P1 | done | [CLI config-directory handling](tasks/009-cli-config-directory-handling.md) | — | 003, 020 |
| 010 | P1 | done | [Mode-aware aggregate views](tasks/010-mode-aware-aggregate-views.md) | 001 | 006, 008, 011, 012, 013, 014, 015 |
| 011 | P1 | done | [Scrollable modal content and pickers](tasks/011-scrollable-pickers.md) | 008, 010 | 012, 013, 014, 015 |
| 012 | P0 | done | [Safe terminal rendering](tasks/012-safe-terminal-rendering.md) | — | 010, 011, 013, 014, 020 |
| 013 | P1 | done | [Keybindings and search consistency](tasks/013-keybindings-and-search-consistency.md) | — | 011, 012, 014 |
| 014 | P2 | done | [Selection, filter, and layout state](tasks/014-selection-filter-and-layout-state.md) | 010 | 008, 011, 012, 013, 015, 021 |
| 015 | P2 | ready | [App encapsulation](tasks/015-app-encapsulation.md) | 001–014, 016–021 | all prior tasks; run last |
| 016 | P0 | done | [Remove implicit fake fallback](tasks/016-remove-implicit-fake-fallback.md) | — | 002 |
| 017 | P2 | done | [Fake backend heterogeneous SSH row](tasks/017-fake-backend-heterogeneous-ssh-row.md) | 006 | — |
| 018 | P2 | done | [TUI smoke test in CI](tasks/018-tui-smoke-test-in-ci.md) | 017, 019 | 015 |
| 019 | P2 | done | [Fake as a selectable backend](tasks/019-fake-as-a-selectable-backend.md) | 017 | — |
| 020 | P0 | done | [Safe process-owned terminal output](tasks/020-safe-process-terminal-output.md) | 012 | 007, 009, 015 |
| 021 | P2 | done | [Session-aware activity indicator](tasks/021-session-aware-activity-indicator.md) | 002, 010, 014 | 014, 015 |
| 022 | P2 | done | [Make the `RuleEngine` seam implementable](tasks/022-rule-engine-seam.md) | — | 015 |

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

Verification:  017 → 019 → 018
               (preserve the heterogeneous fake sample, move it to
                --backend fake, then smoke-test that final interface in CI)

CLI:           009

UI model:      001 → 010 → 014
              008 + 010 → 011
              014 → 021
              013

Terminal:      012 → 020
               (serialize 020 with 007 and 009 where composition/list output
                overlap)

Final depth:   all tasks 001–014, 016–021 → 015
Rule seam:     022 (independent; ADR 0001 replaced 015's deletion half)
```

Tasks in separate workstreams may run concurrently when the task metadata does
not list a likely conflict. A likely conflict is not a dependency; it means the
agents should coordinate file ownership or serialize their merges.

## Midpoint Validation Baseline

The backlog was re-reviewed on 2026-07-16 at commit
`638dc9097fa052b4629629ec0e7bbb63d2217a44`, before these backlog-only edits.
Completed work remained green:

```text
cargo fmt -- --check                                      pass
cargo clippy --locked --all-targets --all-features -- -D warnings
                                                            pass
cargo test --locked --all-targets                           320 passed
cargo test --locked --all-targets --all-features            335 passed
cargo +nightly fuzz build                                   5 targets built
```

The real TUI driver also passed at 100×30 and 60×18, including opening the
correct heterogeneous SSH picker. No regression in a completed task was found.
The review did identify remaining or newly exposed work, now recorded rather
than hidden in completion notes:

- tasks 007 and 009 reproduced at midpoint and have since been completed;
- task 011 took ownership of short-terminal help clipping as well as picker
  visibility and is now done;
- task 020 followed task 012 for direct stdout/stderr and post-TUI error safety
  and is now done;
- task 021 followed task 014 for the still-unconditional session spinner and is
  now done;
- tasks 019 and 018 are now done, so CI covers the final fake-backend CLI at
  default and narrow terminal sizes;
- stale fuzz warnings in tasks 003 and 005 are marked resolved.

## Current Validation

Re-validated on 2026-07-17 at the head of `main` with tasks 014, 021, and 022
done:

```text
cargo fmt -- --check                                      pass
cargo clippy --locked --all-targets --all-features -- -D warnings
                                                            pass
cargo test --locked --all-targets            410 lib + 6 integration
cargo test --locked --all-targets --all-features
                                             436 lib + 7 integration
cargo +nightly fuzz build                                   5 targets built
```

Task 022 added the integration tests; the library counts are unchanged by it.
The `409`/`435` recorded above were off by one — the actual figures before task
022 were 410 and 436, confirmed by stashing its source change and re-running.
No test has been removed or weakened.

Up from 320/335 at the midpoint. The real TUI
driver passed at 100×30, 100×14, 60×18, and 20×8, on both `--backend fake` (a
finite stream settling to a still `✓`) and `--backend mdns-sd` (a live browse
animating). No regression in a completed task was found.

Tasks 014 and 021 are now done, and every dependency task 015 named is complete.

## Task 015 Rescoped (2026-07-17)

Task 015 originally bundled two changes: App encapsulation, and deleting
`RuleEngine` as a hypothetical seam. The project owner reviewed the second half
and **decided to keep the seam** as a supported extension point, on the grounds
that kinjo publishes `plumber` as public API — so its adapter count in this tree
is not evidence about its users. The reasoning and the obligations that decision
carries are in
[ADR 0001](../adr/0001-rule-engine-is-a-supported-extension-point.md).

The backlog now reflects that:

- **015** keeps the App encapsulation work, which never depended on the seam
  question, and is the larger half. It is unchanged in substance, and is now the
  only task left.
- **022** is new and **done**: the retained seam had to be *implementable* by
  someone who is not `Matcher`, which the forwarding trait was not. Deletion
  would have discarded that work; keeping the seam created it.

Implementing 022 turned up an error in ADR 0001 and its source. The ADR argued
that `RuleEngine` mirrored a `Discovery` trait seam, echoing `README.md`'s claim
of "two trait seams". **There is no `Discovery` trait.** Discovery dispatches on
a closed enum and deliberately keeps its adapter seam *inside* the module; it
has three adapters and no public trait, while `RuleEngine` has one adapter and
is a public trait. Those are opposite decisions. The ADR now records the bad
argument alongside the real reason the seam is kept, and the README claim is
fixed. Nothing here reopens the decision — but a future reader should know the
analogy it was originally justified with does not hold.

`CONTEXT.md` previously stated the one-adapter deletion rule without qualifying
it, and named 015 as the review that would settle `RuleEngine`. Both are now
corrected there, so the question is not silently reopened by a future agent.

Two things task 015 should inherit rather than revisit:

- Task 014 removed the last of the renderer's write-back into `App`: no
  `Cell`-backed layout fields remain, and `src/ui/layout.rs` owns the browse
  screen's geometry as a value the event loop computes before both drawing and
  input. With `ui::viewport` from task 011, that is the shape to extend.
- Task 021 found a second, unrecorded spinner in `render_services` alongside the
  one its evidence named, and that one treated `Complete` as *listening*. Both
  now render from one `Activity` mapping. The lesson generalises for 015: the
  duplicate was invisible because the two call sites each did their own
  arithmetic on `ticks`.

The original midpoint normalization changed backlog documentation only.
Subsequent implementations are reflected in the task index and completion
records above.

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
