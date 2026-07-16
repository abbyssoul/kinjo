# 011: Keep Modal Content and Picker Selection Visible

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P1` |
| Workstream | UI |
| Depends on | 008, 010 |
| Likely conflicts | 012, 013, 014, 015 |
| Owner | agent task-011 (branch `worktree-agent-ac1eff752e1dade49`) |

## Why This Matters

Type, action, occurrence, and service pickers build a full stateless list and
manually style the selected item. Selection indexes can move beyond the modal's
visible height, leaving the active choice off-screen. The user can then run an
invisible target, and terminal resize can make a previously visible target vanish.

The generated help overlay has the same visibility defect without a selection:
its fixed-percentage popup renders all rows into a shorter area and clips the
tail. On a short terminal, documented actions and badges exist but cannot be
reached. Modal visibility should be one coherent UI responsibility.

One viewport module should own visible windows for every modal: a
selected-index-to-visible-window invariant for pickers and a navigable content
window for help.

## Evidence

- `src/ui/app.rs:409-425,457-500,648-673`: picker selection moves across the
  complete item count with no viewport/offset state.
- `src/ui/render.rs:700-815`: modal renderers build complete item vectors.
- `src/ui/render.rs:859-885`: popup content is rendered without list selection or
  scroll state.
- `src/ui/render.rs:1132-1147`: `build_list_items` only applies selection style.
- Task 013's completion record notes that the fixed-percentage help popup clips
  below roughly 24 terminal rows.

Midpoint validation on 2026-07-16 reproduced the help defect in the real TUI:

```sh
KINJO_COLS=60 KINJO_ROWS=18 scripts/drive-tui.sh run '?'
```

The lower help rows were clipped with no way to reveal them. The same 60×18 run
could display the current two-target SSH picker, but that small sample does not
exercise an oversized picker; the structural picker defect remains open.

## Required Outcome

- Every list picker derives a visible slice/window that always contains its
  selected identity.
- Moving above/below the viewport scrolls predictably; moving within it does not
  jump unnecessarily.
- Resizing to a shorter/taller modal recomputes the window and keeps selection
  visible when at least one content row exists.
- Empty and zero-height lists render safely.
- A scrollbar/range indicator communicates position when content is clipped.
- Type, action, occurrence, and service pickers share the same viewport behavior.
- Help content is scrollable/windowed when it exceeds the popup. Every generated
  help row and badges line can be reached at supported short terminal sizes.
- Help navigation and its on-screen hints are derived from typed configurable
  keybindings, preserving task 013's single source of truth.
- Modal operation remains keyboard-driven unless a separate task adds mouse input.

## Implementation Constraints

- Use structured selection identity/index supplied by the mode-aware model from
  task 010; do not infer selection from styled text.
- Prefer a pure viewport calculation over four copies of offset mutation logic.
- Rendering should consume the calculated window; it must not secretly mutate
  unrelated application state through `Cell`.
- Preserve current deterministic item ordering.
- Add explicit typed help-scroll actions/default bindings if help navigation
  needs new input. Do not add hard-coded key checks beside task 013's resolver.
- Update `docs/keybindings.md` for any new action and ensure custom bindings alter
  the help-scroll hint.

## Suggested Implementation Sequence

1. Add pure viewport tests for start/middle/end, empty, and resize cases.
2. Implement one picker viewport calculation returning offset/range.
3. Apply it to all four picker render paths.
4. Add a scrollbar or compact range indicator only when clipping occurs.
5. Add Ratatui TestBackend assertions that the highlighted row is present.
6. Give Help mode a viewport and typed navigation actions, reusing the same pure
   range calculation where its semantics fit.
7. Add short-terminal TestBackend and real-driver checks proving every help row
   can be reached and the hint follows effective bindings.

## Non-Goals

- Mouse support inside modals.
- Changing picker target-disambiguation policy.
- Visual-theme redesign.
- General browse-list scrolling, which already follows the selected row.
- Rewording or redesigning the help content beyond navigation/range hints.

## Acceptance Criteria / Definition of Done

- [x] Selected item remains visible at every index in every picker type.
- [x] Shrinking/growing the terminal retains a visible selected item.
- [x] Empty/zero-height modals do not panic or underflow.
- [x] Clipped pickers visibly communicate their range/position.
- [x] All picker implementations use the shared viewport calculation.
- [x] Every generated help row is reachable on a 60×18 terminal.
- [x] Help range/navigation hints reflect the effective configured bindings.
- [x] Full validation passes.

## Required Tests

- Pure viewport: list shorter/equal/longer than viewport, first/middle/last selection.
- TestBackend: oversized TypeFilter, ActionPicker, InstancePicker, ServicePicker.
- Resize from selection-visible to selection-clipped dimensions.
- Unicode labels and one-row viewport.
- TestBackend: oversized Help content at 60×18, scroll to the final row/badges,
  resize while scrolled, and empty/one-row safety.
- Rebound help-navigation keys change both dispatch and the displayed hint; old
  defaults stop scrolling when unbound.

## Validation

```sh
cargo test --locked ui::render
cargo test --locked ui::app
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
  - New `src/ui/viewport.rs` owns the one pure calculation. `Window` is derived
    from content plus geometry and nothing else, with two anchors:
    `Window::containing(total, height, selected)` for surfaces with a cursor and
    `Window::at(total, height, scroll)` for content that is read. Both clamp into
    `0..total`, so `range()` is always a valid slice index, and a resize simply
    produces a different window from the same state — there is no offset to keep
    in sync and nothing for a renderer to write back.
  - All four pickers (type filter, action, instance, service) now go through one
    `render_picker`, which computes the popup rect, derives the window from the
    height the content will really get, and builds items for the visible indices
    only. The four copies of "build every item and style the selected one" are
    gone, along with `build_list_items`.
  - The browse list's `scroll_offset` now delegates to `Window::containing`, so a
    list row and a picker row stay visible for the same reason; `list_title` uses
    `Window::range_label`.
  - Help is windowed: `help_lines` generates the content, `render_help` shows a
    `Window::at` slice of it, and typed `help.up`/`help.down` actions scroll it.
  - Range chip and scrollbar render only when `Window::is_clipped()`; the help
    scroll hint likewise appears only while there is somewhere to scroll to.
  - No new `Cell`. The help scroll bound needs the popup height, so `render`
    exposes it as the pure `help_viewport(frame_area)`, and `handle_key` takes
    the `area` the next frame will use. State a key changes stays state only key
    handling changes — task 014 inherits the existing `Cell` fields untouched.
  - `modal_hint` now takes action groups, so the help scroll hint reuses the one
    hint builder rather than re-implementing its formatting.

- **Tests added/updated:**
  - `ui::viewport` (16 pure tests): shorter/equal/longer than the viewport,
    first/middle/last selection, no-jump within the window, one-row scroll past
    an edge, empty list, zero-height viewport, one-row viewport, out-of-bounds
    selection, scroll clamping, and `max_scroll`. `every_selection_is_visible_at_every_size`
    exhaustively asserts the invariant for totals 0–24 × heights 1–11.
  - `ui::render`: oversized TypeFilter/ActionPicker/InstancePicker/ServicePicker
    each assert the selected row is on screen **at every index**; scroll-only-on-
    leaving-the-window; resize 6→40 rows keeps selection visible; range chip only
    when clipped; unicode labels; every help row reachable at 60×18; help range
    and scroll-to-end; help that fits advertises no scrolling; scrolled help
    reflows when the terminal grows; help scroll hint follows and disappears with
    the bindings; empty/tiny modals and a long picker in a 1×1 terminal.
  - `ui::app`: help opens at the top; scrolls a row at a time; stops at both
    ends; does not scroll when it all fits; rebound `help.up`/`help.down` move
    dispatch and the old defaults stop scrolling; unbound scroll keys do nothing;
    growing the terminal pulls a scrolled window back onto its content.

- **Documentation updated:** `docs/keybindings.md` — new `[help] up`/`down`
  commands with defaults and when the overlay scrolls, an Emacs-navigation
  example extended to `[help]`, and a note that pickers keep the selected entry
  on screen at every size and show a `first-last/total` chip plus a scrollbar
  when clipped.

- **Validation evidence:**

  ```text
  cargo fmt -- --check                                        pass
  cargo clippy --locked --all-targets --all-features -- -D warnings
                                                              pass
  cargo test --locked --all-targets                           357 passed
  cargo test --locked --all-targets --all-features            382 passed
  ```

  The help defect was reproduced and fixed in the real TUI, per CONTEXT.md.
  Before, `KINJO_COLS=60 KINJO_ROWS=18 scripts/drive-tui.sh run '?'` clipped the
  tail with no way to reveal it — `r / F5`, `esc`, `?`, `q / ^c` and the badges
  line existed but were unreachable:

  ```text
  ╭ servic╭ help ────────────────────────────────────╮───────╮
  │       │   ⇥ / → / ⇧⇥ / ←  switch view tab (servic│SH into│
  │       │   s               filter to selected host│ to run│
  │       ╰ esc closes ──────────────────────────────╯       │
  ```

  After, the same command reports its range, draws a scrollbar, and names the
  scroll keys:

  ```text
  ╭ servic╭ help 1-12/18 ────────────────────────────╮───────╮
  │       │   ⇥ / → / ⇧⇥ / ←  switch view tab (servic║SH into│
  │       │   s               filter to selected host║ to run│
  │       ╰ ↓/↑ scrolls · esc closes ────────────────╯       │
  ```

  `KINJO_COLS=60 KINJO_ROWS=18 scripts/drive-tui.sh run '? Down Down Down Down Down Down'`
  reaches the final row and the badges line, and further `Down` presses clamp at
  `7-18/18` rather than banking scroll:

  ```text
  ╭ servic╭ help 7-18/18 ────────────────────────────╮───────╮
  │       │   r / F5          refresh: restart servic█nces (1│
  │       │   esc             close a modal          ║tion.lo│
  │       │   ?               toggle this help       ║       │
  │       │   q / ^c          quit                   ║ (1)   │
  │       │                                          ║SH into│
  │       │   badges:  ×N occurrences   ★N matching c║ to run│
  │       ╰ ↓/↑ scrolls · esc closes ────────────────╯       │
  ```

  The two-target SSH picker (`Tab Tab Down Down Down Enter`) and the type-filter
  checklist (`t`) were re-driven at 60×18 and are unchanged: both fit, so neither
  shows a range chip. As the midpoint review noted, the sample set cannot produce
  an oversized picker, so the picker defect is covered by TestBackend instead.

- **Follow-ups:**
  - Task 014 still owns removing the existing `Cell` layout fields
    (`details_max_scroll`, `details_viewport`, `list_area`, `details_area`). This
    task added none and left them alone; `handle_key` now receives the frame
    `area`, which is the seam 014 can route the details pane through as well.
  - The details pane still keeps its own scroll/clamp logic rather than using
    `Window`. Converting it is a natural extension but sits inside 014's
    selection/layout-state scope, so it was left out rather than broadened here.
  - Pickers deliberately have no page-scroll action; `[help]` got `pageup`/
    `pagedown` aliases only because help is read in pages. Worth revisiting only
    if a picker ever grows long enough for a row-at-a-time to feel slow.
