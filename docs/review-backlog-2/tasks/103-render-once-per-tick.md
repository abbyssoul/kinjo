# Task 103 — Render pipeline: build details once, skip dead frames

- **Priority**: P1 (performance)
- **Status**: ready
- **Depends on**: 102 (same files; land after the recompute reshape)
- **Likely conflicts**: 102, 104, 110

## Problem

Two independent costs on the draw path.

### 1. Details rows are built twice every tick

`App::update_layout` (`src/ui/app.rs:405-414`) calls
`render::details_content_height(self)` to learn how tall the details pane is.
That function (`render.rs:456-463`) builds the **entire** `Vec<Row<'static>>`
of the pane — every field row, TXT row, occurrence row, child-service row, and
action row — purely to return `rows.len()`. Then `render::render` runs and
`render_details`/`render_command_details` (`render.rs:465-484`, `877-896`)
build the *same* rows a second time to actually draw them. Every occurrence
and child row is a fresh `Vec<Span>` with `display::text` escaping applied, so
this is real allocation, done twice, every tick.

### 2. The loop redraws frames that cannot have changed

`App::run` (`app.rs:362-393`) polls for input on a 120ms timeout and redraws
unconditionally each iteration — ~8 times a second, forever. When the session
has ended (`Complete`/`Failed`), no search cursor is blinking, and no discovery
event arrived, the frame is logically identical to the last one. ratatui diffs
the buffer before writing, so the *terminal I/O* is already skipped — but the
layout computation and the full row/span allocation for the whole screen run
regardless.

One wrinkle blocks a naive "only redraw on change": the occurrence rows render
`record.last_seen.elapsed().as_secs()` (`render.rs:573`), so a "still" frame's
text actually changes once a second. That is a live element and must keep
updating — but at 1Hz, not 8Hz, and only while something is on screen to age.

## Goal

- Build the details rows **once** per tick and share them between the height
  measurement and the draw.
- Stop doing full-screen layout+render work on ticks where nothing the user
  could see has changed, while preserving the two things that legitimately
  animate: the discovery spinner (only while `Listening`) and the
  `last_seen` age counter (1Hz, only while records are shown), plus the search
  caret blink.

## Suggested approach (agent to validate)

### Details once

- Have the event loop build the details rows once and pass both the count (for
  `LayoutSnapshot`) and the rows (for the draw) from one source. Options:
  compute the `Vec<Row>` in the loop and hand it to `render`, or memoise it on
  `App` for the current tick keyed on `(selected, grouping, records-version)`.
  The `render.rs` functions are pure over `App` (ADR 0002); keep them pure —
  prefer computing once in the loop and threading the value, over caching state
  the renderer writes.
- Whatever the shape, `details_content_height` and the draw must agree by
  construction, not by both re-deriving. This is the same "one calculation, not
  two habits" principle round 1 applied to `Window` and tab counts.

### Fewer dead frames

- Track a cheap "does anything need redrawing this tick?" signal: a discovery
  event was applied, a key/mouse event was handled, the session state changed,
  the spinner frame advanced (only when `Listening`), the search caret toggled
  (only in `Search`), or the 1Hz age tick elapsed (only when records are on
  screen). If none is true, skip the layout+draw for that iteration.
- Be conservative: when unsure whether something changed, redraw. A missed
  redraw is a visible bug; a redundant one is only the cost this task is
  reducing. The status line, reload/refresh transitions, and modal
  open/close all count as changes.
- Do **not** lengthen the input poll timeout as a substitute — input latency
  must stay at 120ms or better. This is about skipping *work*, not slowing the
  loop.

## Constraints

- Render stays a pure read of `App` (ADR 0002); no new renderer write-back.
- The spinner must still animate while and only while `Listening`
  (`render.rs:78-84` and the tests in `render.rs` around
  `only_a_listening_session_animates` /
  `an_ended_session_renders_an_identical_frame_at_every_tick`).
- `last_seen` ages must still advance for visible occurrences.

## Tests

- The existing render tests that pin spinner stillness/animation must stay
  green (`an_ended_session_renders_an_identical_frame_at_every_tick`,
  `a_listening_session_still_animates`).
- Add a test that details rows are built once per render cycle if the
  single-build is done via an observable seam (e.g. the loop computes the rows
  and both consumers receive the same value). If the change is internal, assert
  instead that `details_content_height` equals the drawn row count for a fixture
  selection in every grouping mode (pins the "agree by construction" property).
- If a redraw-skip signal is added, add a test that an ended, idle session does
  not rebuild frame content on an inert tick, and that a live session still does.

## Definition of Done

- Details rows built once per tick; measurement and draw share them.
- Idle, ended sessions no longer do full-screen render work every 120ms, while
  spinner/age/caret animation is preserved.
- Drive `scripts/drive-tui.sh` on both `--backend fake` (settles to a still
  `✓`) and `--backend mdns-sd` (live browse animates) and confirm no visual
  regression.
- Completion gate green.
