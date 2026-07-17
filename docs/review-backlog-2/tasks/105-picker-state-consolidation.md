# Task 105 — Consolidate App's modal/picker state

- **Priority**: P2 (maintainability)
- **Status**: done
- **Depends on**: 102 (same file; let the recompute reshape settle first)
- **Likely conflicts**: 102, 103, 110

## Problem

The app's modal state is spread across seven fields that must be kept mutually
consistent by hand (`src/ui/app.rs:184-232`):

- `mode: AppMode`
- `picker_anchor: Option<PickerAnchor>`
- `action_matches: Vec<MatchResult>`
- `action_index: usize`
- `pending_action: Option<MatchResult>`
- `instance_index: usize`
- `service_picker_index: usize`

The invariants between them are real but unenforced: `pending_action` is
`Some` exactly when `mode == InstancePicker`; `picker_anchor` is `Some` for the
three picker modes and `None` otherwise; `action_index` only means anything in
`ActionPicker`; `service_picker_index` only in `ServicePicker`. The cost of
this shape is concentrated in:

- `reconcile_action_pickers` (`app.rs:617-689`) — 70 lines that read a chosen
  identity out of one cache, rebuild from the anchor, then re-seat the cursor,
  with an early return for each mode;
- `return_to_browse` (`app.rs:802-807`) clearing a subset of the fields;
- three near-identical picker key handlers
  (`handle_action_picker_key` 995-1016, `handle_instance_picker_key`
  1018-1049, `handle_service_picker_key` 1181-1213) that each recompute a
  `count`, `move_index`, and dispatch close/up/down/select.

This is exactly the shape round 1 kept warning about (parallel fields a comment
asserts and the compiler cannot) — it is why `BrowseRow` merged
`visible_groups`/`group_matches`. The picker state is the next instance.

## Goal

Make the illegal combinations unrepresentable, so the reconciliation and the
three handlers get shorter and safer, without changing any observable picker
behaviour or any round-1 picker invariant (stale-picker revalidation, cursor
follows chosen identity across recompute, abandon-with-reason on disappearance).

## Suggested approach (agent to validate and size)

Model the open modal as data hung off the mode rather than as loose fields.
Sketch:

```rust
enum Modal {
    None,
    Search,
    TypeFilter { index: usize },
    Help { scroll: usize },
    ActionPicker { anchor: PickerAnchor, matches: Vec<MatchResult>, index: usize },
    InstancePicker { anchor: PickerAnchor, action: MatchResult, index: usize },
    ServicePicker { index: usize }, // anchored on `selected` command row
}
```

This is a *direction*, not a mandate — the exact split (e.g. whether the three
pickers share a struct, whether `Search`/`Help`/`TypeFilter` move in too) is
the implementer's call. Round 1's own guidance applies: a consolidation earns
its place only if it deletes a real way to be wrong. The picker fields do —
`reconcile_action_pickers` exists to paper over their desync risk. The
`Search`/`Help`/`TypeFilter` scalar fields may not be worth moving; keep them
if folding them in just relists the same data behind more machinery
(ADR 0002's test).

Whatever the shape:

- `AppMode` may stay as the key-mode selector (`AppMode::key_mode`) or be
  derived from the modal; keep whichever keeps `handle_key`'s dispatch simple.
- Collapse the three picker key handlers toward one where they genuinely share
  logic (move/close/select over a count), keeping select's per-picker action.
- `reconcile_action_pickers`, `reconcile_service_picker`, and `close_pickers`
  should get materially shorter because the "which fields are live" question is
  answered by the variant.

## Constraints

- No observable behaviour change. Every picker test in `app.rs` must pass
  unchanged: `removing_the_selected_service_closes_an_open_action_picker`,
  the reconcile/abandon family, `command_view_runs_single_service_and_picks_among_many`,
  the instance-picker address tests, etc.
- Render reads these fields today (`render.rs` picker functions read
  `action_matches`, `pending_action`, `service_picker_index`, indices). Keep
  render pure over `App` (ADR 0002); expose whatever accessors it needs.
- Do this **after** 102: 102 may change how matches are stored/borrowed, and
  rebasing a large field reshape onto that is cheaper than the reverse.

## Tests

- All existing picker and modal tests green, unchanged.
- If new accessors or a `Modal` type are introduced, add a couple of unit tests
  that the illegal states are now unrepresentable (e.g. you cannot be in
  `InstancePicker` without an action).

## Definition of Done

- Modal/picker state is consolidated so the cross-field invariants are
  structural, not hand-maintained.
- `reconcile_action_pickers` and the three picker handlers are shorter with no
  behaviour change.
- Drive `scripts/drive-tui.sh` through each picker and confirm no regression.
- Completion gate green.

## Follow-up validation note (2026-07-17)

**The enum/construction-atomic state direction is valid, but two claims need
correction.** `picker_anchor` is not `Some` for all three picker modes:
`ServicePicker` opens without one and becomes anchored only after a service is
selected. Model that valid transition rather than forcing the stated invariant.

Most of `reconcile_action_pickers` protects against live discovery records
disappearing or changing while a picker is open, not merely against parallel
fields becoming desynchronised. A state enum will remove illegal combinations,
but stale-data revalidation and abandon-with-reason behaviour must remain. Set
the shortening expectation accordingly.

Finally, “illegal state cannot compile” is not an ordinary unit-test assertion.
Use construction tests around the available constructors/accessors, or a
compile-fail test only if the type is public enough for that to add value. The
three picker handlers should be consolidated only where the shared handler is
shallower than the explicit branches.

## Completion Record (2026-07-17)

- Replaced seven parallel fields with one construction-atomic `ModalState`.
  Action and instance variants require their anchor and live data; the service
  picker models its valid pre-anchor transition separately.
- Reconciliation now consumes and rebuilds one variant while preserving all
  stale-record, chosen-identity, and abandon-with-reason behavior. Render reads
  narrow picker accessors and remains pure over `App`.
- Kept the three selection handlers explicit where their select actions differ;
  their shared cursor primitive remains `move_index`, avoiding a closure-heavy
  Interface with no additional Depth.
- Picker smoke tests and the completion gate passed.
