# 015: Finish App Encapsulation and Remove the Hypothetical RuleEngine Seam

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P2` |
| Workstream | Architecture |
| Depends on | All tasks 001–014, 016 |
| Likely conflicts | All architectural work; run last |
| Owner | Unclaimed |

## Why This Matters

After behavior is stabilized, two shallow interfaces remain:

1. `RuleEngine` mirrors `Matcher` collection methods but has only one adapter.
   Deleting it makes boxing/dynamic-dispatch complexity vanish rather than
   reappearing in callers.
2. `App` exposes 30 public fields and render consumes the entire object.
   Records, grouping, matches, modal state, selection, and layout invariants are
   spread across App/render/tests, limiting locality and making the public crate
   interface unnecessarily fragile.

Finish the deepening begun by earlier tasks without changing observable behavior.

## Evidence

- `src/plumber/mod.rs:195-219`: `RuleEngine` exactly forwards matching, commands,
  and count to `Matcher`; `Matcher` is the only adapter.
- `src/ui/app.rs:33,58-108`: config loader and App box the trait and expose broad
  mutable/public state.
- `src/lib.rs:22,48`: composition performs trait-object conversion for the sole
  adapter.
- `src/ui/render.rs:33`: render's interface is the entire `App`.
- `README.md:191-215`: architecture documentation describes `RuleEngine` as an
  intended extension seam despite no second matching implementation.
- Earlier tasks establish deep discovery session, compiled command-rule, browse
  projection, modal-revalidation, and layout interfaces; this task should simplify
  around those results rather than invent parallel abstractions.

## Required Outcome

- Remove `RuleEngine` and its trait-object plumbing. Store/use the one concrete
  compiled rule-set type produced by task 005.
- Removing `RuleEngine` reverses a documented design intent: `README.md:191-215`
  presents the trait as an intended extension seam. Before deleting it, record
  the decision as an ADR under `docs/adr/` (deletion test, one adapter, pure
  forwarding — versus keeping a speculative seam) and obtain explicit project
  owner sign-off on that ADR. If the owner prefers to keep the seam, keep the
  trait, document it as speculative, and limit this task to the App
  encapsulation work.
- Keep substitution seams only where two adapters genuinely vary. Tests use
  constructors/fixtures or private internal seams rather than a public hypothetical
  matcher seam.
- Make App fields private except for genuinely supported crate interface.
- Consolidate records, filters, grouping/projections, selected identity, cached
  matches, and modal reconciliation behind a deep browse-model interface.
- Event-loop App owns input/event orchestration, discovery session, config reload,
  and exec handoff; it asks the browse model for state transitions/view data rather
  than mutating parallel vectors directly.
- Render consumes an immutable, explicit view/interface rather than the full App.
- Remove parallel-array invariants such as `visible_groups` plus `group_matches` or
  make their consistency private and construction-atomic.
- Update crate/README architecture documentation to describe actual supported seams.
- Preserve all behavior/regressions from tasks 001–014.

## Implementation Constraints

- This is a behavior-preserving refactor. Add characterization tests only where a
  missing assertion is necessary; do not combine feature work.
- Use architecture vocabulary from `CONTEXT.md` and apply the deletion test to each
  new module.
- The browse model's interface is the test surface. Tests should not require public
  field mutation to arrange state.
- Avoid a monolithic replacement with an equally broad interface. Depth means App
  asks for meaningful operations/view projections, not getters for every old field.
- Do not reintroduce a one-adapter public trait under a new name.

## Suggested Implementation Sequence

1. Inventory which old public fields are used outside their defining module and
   define characterization tests through current user operations.
2. Replace `RuleEngine` with the concrete validated rule-set throughout config/App.
3. Move record/filter/group/match/modal invariants behind the browse model created
   by earlier tasks.
4. Introduce immutable render view data and narrow render's interface.
5. Make fields private and migrate tests to constructors/operations.
6. Remove obsolete aliases, parallel state, comments, and architecture claims.
7. Run the complete regression/fuzz/feature gate.

## Non-Goals

- A plugin system for third-party matchers.
- New discovery adapters or command syntax.
- Visual redesign or input behavior changes.
- Publishing a stable library interface beyond what current users require; any
  breaking public-crate decision should be documented explicitly.

## Acceptance Criteria / Definition of Done

- [ ] An ADR records the `RuleEngine` seam decision with explicit owner sign-off.
- [ ] `RuleEngine`, boxing, and one-adapter forwarding implementation are removed
      (or retained per the ADR decision).
- [ ] App's implementation state is private and mutated through meaningful operations.
- [ ] Browse-model invariants have one owner and are tested through its interface.
- [ ] Render consumes immutable view/layout data, not the entire mutable App state.
- [ ] Parallel representation consistency is construction-atomic or eliminated.
- [ ] README/crate docs describe the actual discovery and rule-set interfaces.
- [ ] No behavior regression in all tasks 001–014.
- [ ] Full validation and relevant fuzz smoke targets pass.

## Required Tests

- Existing App behavior suite migrated without direct public-field dependency where
  practical.
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

## Completion Record

- **Implemented:**
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
