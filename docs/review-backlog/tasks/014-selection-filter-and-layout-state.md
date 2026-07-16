# 014: Concentrate Selection, Filter Counts, and Layout State

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P2` |
| Workstream | UI / Architecture |
| Depends on | 010 |
| Likely conflicts | 008, 011, 012, 013, 015, 021 |
| Owner | Completed on `main` |

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

Midpoint validation on 2026-07-16 confirmed that render still writes temporal
state back into `App`: `src/ui/app.rs` retains `Cell`-backed
`details_max_scroll`, `list_area`, and `details_area` fields, and
`src/ui/render.rs` still sets the detail limit during drawing. The passing suite
therefore does not invalidate this task; it lacks a regression that breaks the
render-before-input ordering dependency.

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
- The immutable render view/layout interface exposes the current discovery
  `SessionState` needed by task 021; that task must not reach back into the full
  mutable `App` merely to render the activity indicator.

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

- [x] Removing/filtering out the selected row focuses a deterministic replacement
      at detail scroll zero.
- [x] Updating the same selected identity preserves/clamps scroll appropriately.
- [x] Type chips count only currently discovered and enabled types.
- [x] Disabled types disappear/reappear according to the documented preference.
- [x] Render no longer mutates App through layout `Cell` fields.
- [x] Render and mouse/scroll input consume one layout snapshot.
- [x] Resize, tiny, and zero-size layouts remain safe and tested.
- [x] Full validation passes.

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
  - **`src/ui/layout.rs` (new): `LayoutSnapshot` + `Content`.** One pure function
    of terminal area and content dimensions owns the whole browse-screen split.
    `App::update_layout(area)` computes it in the event loop *before* both
    `terminal.draw` and `event::poll`, so render and input consume the same
    value. `terminal.autoresize()` runs first, so the snapshot describes the
    screen the frame is actually drawn on rather than the previous size.
    The snapshot answers list window, details window, details viewport, details
    max scroll, click hit testing (`list_row_at`), and wheel routing
    (`is_over_details`), so those can no longer disagree.
  - **Removed all four render-written `Cell` fields** from `App`
    (`details_max_scroll`, `details_viewport`, `list_area`, `details_area`).
    `render` is now read-only over `&App`: it takes every rectangle and bound
    from `app.layout`. `App::service_types` also went — it duplicated state the
    filter now owns. `LayoutSnapshot::default()` is the zero-size layout, so an
    event handled before the first frame hits nothing instead of a stale rect.
  - **Details content height is measured by the builders that draw it.**
    `render_details`/`render_command_details` were split into pure
    `detail_rows`/`command_detail_rows` plus a draw step, and
    `render::details_content_height` feeds `Content::details_total`. The scroll
    bound and the rows on screen therefore come from the same rows rather than
    from two counts that could drift — avoiding the parallel calculation the
    task's constraints forbid.
  - **`FilterState` re-modelled around the observation/preference split.**
    `enabled_service_types` is gone; the state is now `discovered_types`
    (an observation, replaced wholesale by `observe_types`) and `disabled_types`
    (a preference, deliberately never pruned). "Enabled" is derived as
    `discovered − disabled`, which is what makes the stale count impossible by
    construction rather than by a `.min()` patch. Rendering asks
    `type_counts()`/`is_enabled()` and never sees a set length. `is_active()` is
    now `enabled_count() < discovered.len()`, so a preference against a type
    nobody advertises no longer claims the list is narrowed.
  - **Detail scroll follows focused identity.** Both recompute paths capture the
    focused identity before the rebuild and call the new `App::refocus_details`
    after, resetting the scroll to zero only when the identity under the cursor
    changed — covering removal, filtering, and the clamp-onto-a-neighbour case
    uniformly. The same identity keeps its scroll, which `update_layout` then
    clamps to the new content bounds.
- **Tests added/updated:** 397 → from 320 at the midpoint baseline (422 with all
  features).
  - `ui::layout` (new, 10 tests): panel tiling, details bounds, click hit
    testing against the drawn window, wheel routing, plus the safety fixtures —
    zero-size, panels shorter than their own borders, the default snapshot, and
    a resize producing new bounds.
  - `ui::filter`: `a_type_that_disappeared_is_counted_on_neither_side` is the
    task's `0/1`-not-`1/1` case; `a_disabled_type_stays_disabled_across_
    disappearing_and_reappearing` pins the documented preference.
  - `ui::render::the_type_chip_counts_only_types_that_are_still_discovered`
    asserts the same bug through the rendered buffer (verified against the old
    code: it printed `types 1/1`).
  - `ui::app`: selected row removed / filtered away / same identity updated /
    kept scroll clamped to shortened content / list emptied. Plus
    `details_scroll_is_bounded_before_anything_has_been_drawn` and
    `a_resize_reclamps_the_details_scroll_before_the_next_input` — the two the
    old `Cell` ordering could not fail on.
  - Mouse and detail-scroll tests now share one `MOUSE_SCREEN` fixture and
    compute their bounds from the real layout rather than asserting fabricated
    rectangles and maxima into `App`.
  - `render_buffer` takes `&mut App` and computes the layout first, so no test
    can draw a frame without the app having been told what size it is.
- **Documentation updated:** `README.md` — the `types n/m` chip counts only
  currently advertised types; switching a type off is remembered across a device
  disappearing and reappearing; details scroll is kept per row and clamped on
  resize.
- **Validation evidence:**

  ```text
  cargo fmt -- --check                                          pass
  cargo clippy --locked --all-targets --all-features -- -D warnings
                                                                pass
  cargo test --locked --all-targets                             397 passed
  cargo test --locked --all-targets --all-features              422 passed
  cargo +nightly fuzz build                                     5 targets built
  ```

  Driven per `CONTEXT.md` (sample backend, so reproducible):
  `drive-tui.sh run 'Down Down'` at 100×30 renders correctly through the new
  layout; `'t Down Space Escape'` takes the chip `4/4 → 3/4` with the list
  following; `Tab Tab Down Enter` at 60×18 still opens the right picker;
  `'Down Down d d'` at 100×14 scrolls the details with a scrollbar, and
  `'Down Down d d Up'` shows the moved-to row from the top of its details — the
  scroll-reset rule, seen in the real app.
- **Follow-ups:**
  - Task 021 is unblocked: `render` is `&App` and reads discovery state through
    the immutable `DiscoverySession::state()` (already used at
    `src/ui/render.rs:205` for the failed-session case), so the activity
    indicator needs no access to a mutable `App`. No extra wrapper was added —
    one would be a shallow module that fails the deletion test.
  - `LayoutSnapshot` deliberately excludes modal geometry: pickers and help are
    centred on the whole terminal area and their windows belong to
    `ui::viewport` (task 011). `handle_key` now takes its area from
    `self.layout.area()`, so modal geometry at least shares the snapshot's
    notion of the screen.
  - Noted, not fixed: `details_max_scroll()` over a zero-height pane is `total`
    rather than `0` (inherited from `Window::max_scroll`, shared with the help
    overlay). Harmless — the window stays empty and the next snapshot with room
    re-clamps — and pinned by a layout test. Changing it would touch task 011's
    module, so it belongs to task 015 if anyone wants it tidier.
