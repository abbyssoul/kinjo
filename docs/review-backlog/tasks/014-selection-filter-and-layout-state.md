# 014: Concentrate Selection, Filter Counts, and Layout State

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P2` |
| Workstream | UI / Architecture |
| Depends on | 010 |
| Likely conflicts | 008, 011, 012, 013, 015 |
| Owner | Unclaimed |

## Why This Matters

Several small state invariants are split across App, FilterState, and render:
recomputation can move selection without resetting detail scroll; type-filter
counts include historical types rather than current enabled types; and render
mutates `Cell` fields that later input handling assumes were populated. These are
temporal interfaces—correctness depends on call order not expressed by types.

Concentrate each invariant behind the module that owns it and use one explicit,
pure layout snapshot for render and hit testing.

## Evidence

- `src/ui/app.rs:243-257`: recomputation preserves/clamps selection but does not
  reset `details_scroll` when identity changes.
- `src/ui/filter.rs:27-34`: enabled/disabled sets retain historical service types.
- `src/ui/render.rs:163-172`: enabled count uses `len().min(total)`, not an
  intersection with currently discovered types.
- `src/ui/render.rs:52-55,1116-1121`: render mutates pane rectangles and detail
  limits through `Cell` on `&App`.
- `src/ui/app.rs:515-574`: scrolling/mouse handling later depends on those values.
- `src/ui/app.rs:97-107`: layout feedback is represented as several independent
  fields rather than one coherent snapshot.

## Required Outcome

- Selection is tracked by structured row identity. Whenever recomputation changes
  focused identity, details scroll resets to zero; preserving identity preserves
  scroll subject to new content bounds.
- FilterState exposes current discovered types, enabled intersection/count, and
  disabled preferences through its interface. Rendering never derives counts from
  internal set lengths.
- Historical disabled preferences may survive disappearance/reappearance, but do
  not inflate the current enabled count.
- Introduce a pure layout module/snapshot computed explicitly from terminal area,
  active projection, and content dimensions before render/input use.
- Renderer consumes layout without mutating App through interior mutability.
- Mouse hit testing, list viewport, details viewport/max scroll, and rendering use
  the same snapshot/calculations.
- Resize and zero-size panes produce a new safe snapshot before subsequent input.

## Implementation Constraints

- Build on task 010's structured row identities/projections.
- Do not maintain parallel layout calculations in App and render.
- Keep pure layout functions easy to test without a live terminal.
- Preserve current keyboard/mouse behavior except where fixing stale geometry.
- Prefer one coherent `LayoutSnapshot` over several independent optional fields.

## Suggested Implementation Sequence

1. Add selection-removal/filter and stale-count regression tests.
2. Move current-type intersection/count behavior behind FilterState.
3. Reset scroll based on before/after selected identity.
4. Extract pure layout calculation and explicit snapshot update in the event loop.
5. Route render, hit testing, and scroll clamping through the snapshot.
6. Remove renderer-written `Cell` fields and temporal ordering comments.

## Non-Goals

- Full App encapsulation; task 015 follows.
- Picker viewport behavior; task 011 owns modal scrolling.
- Visual layout percentage changes.
- Persisting filters between application runs.

## Acceptance Criteria / Definition of Done

- [ ] Removing/filtering out the selected row focuses a deterministic replacement
      at detail scroll zero.
- [ ] Updating the same selected identity preserves/clamps scroll appropriately.
- [ ] Type chips count only currently discovered and enabled types.
- [ ] Disabled types disappear/reappear according to the documented preference.
- [ ] Render no longer mutates App through layout `Cell` fields.
- [ ] Render and mouse/scroll input consume one layout snapshot.
- [ ] Resize, tiny, and zero-size layouts remain safe and tested.
- [ ] Full validation passes.

## Required Tests

- Selected entry removed; selected entry filtered; same identity updated.
- Stale enabled SSH plus current disabled HTTP reports `0/1`, not `1/1`.
- Disabled type disappearance/reappearance.
- Pure layout fixtures for normal, narrow, zero-height, and resized terminals.
- Mouse click/wheel and detail scroll use the same fixture coordinates.

## Validation

```sh
cargo test --locked ui::filter
cargo test --locked ui::app::tests::scroll
cargo test --locked ui::app::tests::mouse
cargo test --locked ui::render
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
