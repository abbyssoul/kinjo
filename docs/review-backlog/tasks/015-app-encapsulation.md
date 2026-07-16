# 015: Finish App Encapsulation

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P2` |
| Workstream | Architecture |
| Depends on | 001–014, 016–021 |
| Likely conflicts | all prior tasks; run last |
| Owner | Unclaimed |

## Scope Change (2026-07-17)

This task originally paired App encapsulation with deleting `RuleEngine`. The
project owner decided to keep that seam as a supported extension point, recorded
in [ADR 0001](../../adr/0001-rule-engine-is-a-supported-extension-point.md).

The RuleEngine work is therefore **out of scope here** and moved to
[task 022](022-rule-engine-seam.md), which makes the retained seam implementable
from outside `Matcher`. This task keeps `Box<dyn RuleEngine>` exactly as it is
and does not touch the trait.

Everything below is the App encapsulation work, which never depended on that
decision.

## Why This Matters

`App` exposes roughly 30 public fields and `render` consumes the entire object.
Records, grouping, matches, modal state, selection, and layout invariants are
spread across App, render, and tests, so no module owns them.

Keeping `RuleEngine` raises the stakes rather than lowering them. `App::new` is
public and is the documented way to substitute a rule engine (ADR 0001), which
means App's public fields are *accidentally* part of the crate's public API. An
extension point is only usable if the type behind it has an interface someone can
program against, so encapsulating App is now a prerequisite for the seam being
worth keeping — not merely internal tidying.

Finish the deepening begun by earlier tasks without changing observable behavior.

## Evidence

Re-checked at the head of `main` on 2026-07-17; line numbers are anchors, not
authority.

- `src/ui/app.rs:122-195`: `App` exposes ~30 public fields — records, filter,
  visible_groups, selection, modal indices, status, scroll offsets, tick counter.
- `src/ui/app.rs:132,170`: `visible_groups` and `group_matches` are parallel
  vectors whose element correspondence is a comment, not an invariant any code
  enforces.
- `src/ui/app.rs:194`: `layout` is already `pub(crate)` — task 014's result and
  the precedent to follow for the rest.
- `src/ui/render.rs`: render's interface is the entire mutable `App`.
- `src/ui/app.rs:3399,3405,3448,3470,3526` and neighbours: tests reach into
  public fields to arrange and assert state.
- `src/ui/layout.rs` (task 014) and `src/ui/viewport.rs` (task 011) already own
  browse geometry and windowing as values computed before draw and input. That
  is the shape to extend.

## Required Outcome

- Make App fields private except for a genuinely supported crate interface.
  Anything left public is a deliberate API commitment; say why in a doc comment.
- Consolidate records, filters, grouping/projections, selected identity, cached
  matches, and modal reconciliation behind a deep browse-model interface.
- Event-loop App owns input/event orchestration, discovery session, config
  reload, and exec handoff; it asks the browse model for state transitions and
  view data rather than mutating parallel vectors directly.
- Render consumes an immutable, explicit view interface rather than the full App.
- Remove parallel-array invariants such as `visible_groups` plus `group_matches`,
  or make their consistency private and construction-atomic.
- Update crate/README architecture documentation to describe the actual App
  interface. Do not restate the seam question; ADR 0001 settled it.
- Preserve all behavior and regressions from tasks 001–014 and 016–021.

## Implementation Constraints

- This is a behavior-preserving refactor. Add characterization tests only where a
  missing assertion is necessary; do not combine feature work.
- Use architecture vocabulary from `CONTEXT.md` and apply the deletion test to
  each new module. Note that ADR 0001 scopes the deletion test to internal
  abstractions; it does not apply to `RuleEngine`.
- Keep `Box<dyn RuleEngine>` in `App` and `ReloadOutcome`. If encapsulation makes
  the trait's shape awkward, record the evidence for task 022 rather than
  changing the trait here.
- The browse model's interface is the test surface. Tests should not require
  public field mutation to arrange state.
- Avoid a monolithic replacement with an equally broad interface. Depth means App
  asks for meaningful operations and view projections, not getters for every old
  field.
- App's remaining public surface must stay sufficient for the ADR 0001 extension
  path: an external composition root builds an `App` via `App::new`, attaches a
  config loader and discovery factory, and runs it. Do not privatise that away —
  task 022 owns proving it still works.

## Suggested Implementation Sequence

1. Inventory which public fields are used outside `src/ui/app.rs`, and define
   characterization tests through current user operations.
2. Move record/filter/group/match/modal invariants behind the browse model
   created by earlier tasks.
3. Introduce immutable render view data and narrow render's interface.
4. Make fields private and migrate tests to constructors and operations.
5. Remove obsolete aliases, parallel state, comments, and architecture claims.
6. Run the complete regression/fuzz/feature gate.

## Non-Goals

- Any change to `RuleEngine`, its trait objects, or its interface (task 022).
- New discovery adapters or command syntax.
- Visual redesign or input behavior changes.
- Publishing a stable library interface beyond what current users require; any
  breaking public-crate decision should be documented explicitly.

## Acceptance Criteria / Definition of Done

- [ ] App's implementation state is private and mutated through meaningful
      operations; each surviving public item documents why it is public.
- [ ] Browse-model invariants have one owner and are tested through its interface.
- [ ] Render consumes immutable view/layout data, not the entire mutable App state.
- [ ] Parallel representation consistency is construction-atomic or eliminated.
- [ ] The ADR 0001 extension path still compiles: an `App` can be constructed and
      run from outside `src/lib.rs` without touching private state.
- [ ] README/crate docs describe the actual App interface.
- [ ] No behavior regression in tasks 001–014 and 016–021.
- [ ] Full validation and relevant fuzz smoke targets pass.

## Required Tests

- Existing App behavior suite migrated without direct public-field dependency
  where practical.
- Browse model: event application, filtering/grouping, selection preservation,
  action candidates, modal reconciliation, immutable view projection.
- Config reload/discovery refresh composition using concrete deep interfaces.
- Render snapshots for all modes using explicit view data.

## Validation

```sh
cargo test --locked ui::app
cargo test --locked plumber
cargo test --locked discovery
# Run relevant fuzz smoke targets per CONTRIBUTING.md.
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

Drive the real TUI as well; encapsulating browse state and the render interface
touches everything the user looks at:

```sh
scripts/drive-tui.sh run 'Tab Tab Down Down Down Enter'
```

## Completion Record

- **Implemented:**
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
