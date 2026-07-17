# Task 102 — Recompute pipeline: remove the clone cascade

- **Priority**: P1 (performance)
- **Status**: ready
- **Depends on**: none
- **Likely conflicts**: 103, 105 (same file), 107 (builds on this)

## Problem

`App::recompute_visible` (`src/ui/app.rs:502-539`) runs on every tick that saw
a discovery event, and on every search keystroke. One call currently:

1. `self.records.values().cloned().collect()` — clones **every** `Entry` out
   of the `BTreeMap` (`app.rs:503`).
2. `filter.observe_types(&records)` — allocates a `BTreeSet<String>` of types
   (`filter.rs:55-62`).
3. `filter.apply(&records)` — for a text query, calls `searchable_text()` on
   every record (a fresh `String` built from every field, `entry.rs:350-377`),
   then `.cloned()` on the survivors (`filter.rs:120-134`).
4. `count_tabs(&filtered)` — calls `browse_row_count` **three** times
   (`app.rs:700-705`). Each call buckets every record into a
   `BTreeSet<EntryGroupId>` (`entry.rs:709-715`), and each `EntryGroupId`
   clones several `String`s (`entry.rs:723-742`). All of it is discarded after
   counting.
5. `browse_groups(&filtered, browse)` — buckets **again**, cloning every
   record into its bucket (`entry.rs:683`), building the same `EntryGroupId`s a
   fourth time.
6. Per row, `matcher.matches_group` clones the whole `CommandConfig` for every
   match (`plumber/mod.rs:154`), and `distinct_targets` prepares every
   candidate into a `PreparedCommand` and scans an O(n²) `seen` vector
   (`plumber/mod.rs:215-227`).

`recompute_command_groups` (`app.rs:709-760`) is worse in the command view:
it clones every `CommandConfig` out of the engine, then clones the whole
`EntryGroup` once per matching command (`app.rs:740`).

At a few hundred services this is invisible. It is nonetheless allocation-bound
work, superlinear in a couple of places, on the interactive path — and it is
the substrate task 107 needs to be bounded.

## Goal

Cut the redundant cloning and re-bucketing without changing any observable
behaviour or any round-1 invariant. The projection the user sees, the tab
counts, selection survival, and picker reconciliation must be identical.

## Suggested approach (agent to validate against current code)

- **Count from the projection, not a third bucketing.** In the browse modes,
  `browse_groups` already produces the rows for the active tab; that tab's
  count is `rows.len()`. Only the *inactive* tabs need `browse_row_count`.
  Consider computing the active tab's count from the rows just built and
  calling `browse_row_count` only for the other three — or, better, a single
  helper that returns all four counts and the active projection from one pass
  over `filtered`.
- **Borrow instead of clone where the value outlives the call.** `filter.apply`
  and `browse_groups` both take `&[Entry]`; several of the intermediate
  `Vec<Entry>` clones exist only to satisfy ownership that a borrow would
  serve. `browse_groups` cloning records into buckets is the load-bearing one
  (rows own their instances); the two `values().cloned()` collections feeding
  it may not need to be owned.
- **`distinct_targets` O(n²)** (`plumber/mod.rs:215-227`): the `seen` list is a
  `Vec<Option<PreparedCommand>>` scanned with `.contains`. For rows with many
  candidates this is quadratic. A hash set keyed on the prepared argv+mode
  would make it linear; `PreparedCommand` is already `PartialEq`/`Eq`, so it
  needs `Hash`. Keep the *ordering* guarantee (targets in discovery order) and
  the "unpreparable candidate is kept" guarantee (`mod.rs:211-214`).
- **Command view**: avoid cloning `EntryGroup` per matching command where a
  shared reference or an index would do. This is inside the `RuleEngine`
  contract, which says `commands()` returns owned rules by design
  (`mod.rs:417-425`) — do not change that contract; reduce the *group* cloning
  in `recompute_command_groups`, not the engine's `commands()` return type.

Measure, do not guess: add a temporary bench or a `--backend fake` run under
`perf`/`valgrind --tool=dhat` if you want numbers, but the correctness bar is
the tests, not a speedup target.

## Constraints

- No change to `RuleEngine`'s public contract (ADR 0001).
- No change to any round-1 invariant: occurrence identity, grouping honesty,
  target dedup semantics, selection-survival-by-identity, tab-count-matches-list.
- `render.rs` reads `App` directly (ADR 0002); do not introduce a projected
  view type as a side effect. If a shared intermediate helps, keep it private
  to `app.rs`.

## Tests

- Existing `app.rs`, `entry.rs`, and `plumber/mod.rs` tests must stay green
  unchanged — they already pin the behaviour this task must preserve
  (`every_tab_count_matches_the_rows_that_tab_lists`,
  `selection_survives_recomputation_by_structured_row_identity`,
  `distinct_targets`-family tests).
- Add a targeted test that `distinct_targets` preserves order and keeps an
  unpreparable candidate, if the data structure changes.
- If `PreparedCommand` gains `Hash`, add a test that equal prepared commands
  hash equal.

## Definition of Done

- Redundant cloning/re-bucketing removed as above, behaviour identical.
- `distinct_targets` is no longer O(n²) in candidate count (or a note explains
  why the change was not worth it).
- Drive `scripts/drive-tui.sh` on `--backend fake` and confirm the list,
  counts, details, and pickers are unchanged.
- Completion gate green.
