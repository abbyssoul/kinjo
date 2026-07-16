# 021: Make the Discovery Activity Indicator Session-Aware

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P2` |
| Workstream | UI / Discovery state |
| Depends on | 002, 010, 014 (all done) |
| Likely conflicts | 014, 015 |
| Owner | Completed on `main` |

## Why This Matters

Task 002 gave discovery an explicit persistent lifecycle and corrected the list
body after failure or finite fake completion. The top bar still renders the same
animated spinner in every state. Once discovery is complete or failed, animation
continues to imply background activity that is no longer happening.

The midpoint review found this follow-up had been assigned informally to task
010, but task 010 completed without owning it. Record it explicitly rather than
reopening either completed task.

## Evidence

- `src/ui/render.rs:82-89` selects `SPINNER[(app.ticks / 2) % len]` on every
  frame without consulting `SessionState`.
- Task 002 introduced `SessionState::{Listening, Complete, Failed}` and made the
  body distinguish those states, but its completion follow-up left the top-bar
  animation for later work.
- Task 010 completed mode-aware aggregate projections; the spinner remains
  unconditional on the current branch.
- Task 014 will replace render's access to the whole mutable `App` with an
  immutable view/layout interface. Implementing this after 014 avoids creating
  another temporary direct-App dependency.

## Required Outcome

- Animate the discovery indicator only while `SessionState::Listening`.
- Render a stable, distinguishable complete state after the explicit fake
  stream ends and a stable failed state after real discovery fails/stops.
- The top bar and list-body lifecycle message cannot contradict one another.
- Refresh immediately returns the indicator to listening for the replacement
  session and then follows that session's terminal state.
- Task 014's immutable render view carries the session/activity fact explicitly;
  render does not reach into discovery internals or the full mutable `App`.
- Indicator text/symbols remain safe at tiny widths and do not cause adjacent tab
  layout to jump as animation frames change.

## Implementation Constraints

- Consume `SessionState`; do not infer activity from whether records exist, from
  fake mode, or from a transient status message.
- Keep the state mapping pure and independently testable.
- Reuse the established color/style vocabulary unless a small distinction is
  required for complete versus failed.
- Preserve task 002's semantics: fake completion is normal, real failure is not,
  and neither state starts an automatic retry.
- Do not implement this before task 014 unless that task's view interface has
  already landed; otherwise the two changes will rework the same render seam.

## Suggested Implementation Sequence

1. Extend task 014's immutable view fixture with discovery session state.
2. Add pure mapping tests for listening, complete, and failed indicators.
3. Render animation only for listening; use stable complete/failed variants.
4. Add refresh-transition and tiny-terminal TestBackend regressions.
5. Drive the finite fake session and an injected failure through the real App.

## Non-Goals

- Automatic discovery retry or reconnect policy.
- Changing lifecycle semantics or failure provenance from task 002.
- Redesigning the top bar or moving discovery status into a new panel.
- Adding progress percentages; discovery has no bounded progress measure.

## Acceptance Criteria / Definition of Done

- [x] Listening animates and complete/failed states do not.
- [x] Complete and failed indicators are visually distinguishable and truthful.
- [x] Refresh transitions the indicator through the replacement session's state.
- [x] The top bar and empty/body lifecycle messages agree in every state.
- [x] Render consumes session state through task 014's immutable view interface.
- [x] Tiny-width rendering is stable and panic-free.
- [x] Full validation passes.

## Required Tests

- Pure indicator mapping for all `SessionState` variants.
- TestBackend frames at different tick counts: listening changes; complete and
  failed remain byte-for-byte stable.
- Finite fake session reaches complete without continuing to animate.
- Injected real failure displays the failed top-bar state and matching body text.
- Refresh from complete/failed returns to listening and follows the new outcome.
- One-column and narrow top-bar layouts.

## Validation

```sh
cargo test --locked ui::render
cargo test --locked ui::app::tests::refresh
cargo test --locked discovery::session
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
  - **`Activity` in `src/ui/render.rs`:** a pure, total mapping from
    `&SessionState` to what the UI claims discovery is doing — `Listening`,
    `Complete`, `Failed` — with `symbol(ticks)` and `color()`. Only `Listening`
    varies with the tick; `Complete` draws `✓` in `GOOD` and `Failed` draws `✗`
    in `WARN`, reusing the palette's existing "fine"/"needs attention"
    vocabulary rather than inventing one. Kept in `render.rs` beside the palette
    and `SPINNER` because it is render's vocabulary: a separate module would
    either have to import private palette constants or force the palette out of
    render, which is task 015's call, not this task's.
  - **A second, unrecorded spinner was in scope.** The task's evidence names
    `render_top_bar` only, but `render_services` had its own unconditional
    `SPINNER[...]`, and its `else` branch treated `Complete` as *listening* — so
    a finished sample stream with an empty list said "listening for mDNS
    services on local…". That is the contradiction the Required Outcome forbids,
    so both call sites now render from one `Activity`. The new `session_lines`
    helper makes the top bar and the body two renderings of one fact rather than
    two opinions kept in step by hand, and `Complete` gained its own honest
    empty-state ("sample discovery complete / no sample services to show").
  - **`DiscoverySession::ended(state)`** — a `#[cfg(test)]` constructor beside
    the existing `detached`/`inert` pair. `SessionState::Complete` is otherwise
    only reachable through a real finite fake stream, and then only under the
    `fake` feature, which would have left the completed indicator untested in the
    default build. Not a seam: a constructor, following this module's own
    established pattern for driving endings.
  - **`App::drain_discovery` is now `pub(crate)`**, matching `App::update_layout`
    from task 014, so a render test can drive the loop's real steps instead of
    simulating them.
- **Tests added/updated:** 409 (from 397), 435 with all features.
  - Pure mapping over every `SessionState` variant, including both
    `FailureKind`s.
  - `only_a_listening_session_animates`: the spinner produces all 10 frames over
    40 ticks; the two endings produce exactly one glyph each.
  - `an_ended_session_renders_an_identical_frame_at_every_tick`: whole-buffer
    equality at ticks 0/1/2/7/21/99 for `Complete` and `Failed` — byte-for-byte,
    as the task asks. `a_listening_session_still_animates` is its control, so
    the stability assertion cannot pass vacuously.
  - Agreement tests: `Complete` reports completion and never "listening", and is
    not dressed as a failure; `Failed` shows `✗` beside its cause; an active
    filter keeps its own message while the indicator still tells the truth.
  - `every_indicator_is_one_column_wide` pins the layout-jump requirement at the
    source (all 12 glyphs), rather than eyeballing the tab strip.
  - `the_indicator_follows_the_session_a_refresh_installs`, plus a new
    `refresh_restarts_a_completed_session` in `ui::app` — the existing refresh
    tests only covered recovery from *failed*.
  - `a_real_finite_fake_stream_ends_on_a_still_complete_frame` (`fake` feature):
    the real backend drained to its own ending through the real `App`, asserting
    a still frame showing `✓`.
  - `the_indicator_is_safe_in_a_terminal_with_no_room_for_it`: every state at
    100×24 down to 1×1.
- **Documentation updated:** `README.md` — a table of the three indicators and
  what each means, that a still indicator means the list is final until `r`, that
  the empty-list message agrees with it, and that neither ending retries by
  itself (task 002's semantics, now visible in the UI).
- **Validation evidence:**

  ```text
  cargo fmt -- --check                                          pass
  cargo clippy --locked --all-targets --all-features -- -D warnings
                                                                pass
  cargo test --locked --all-targets                             409 passed
  cargo test --locked --all-targets --all-features              435 passed
  ```

  Driven through the real app per `CONTEXT.md`, covering two of the three states
  end to end:
  - **Complete** (`--backend fake`): settles to ` ✓ kinjo` and is byte-identical
    two seconds later — previously an endless ` ⠸`.
  - **Listening** (`--backend mdns-sd --service-type _nokinjo._tcp`): ` ⠙ kinjo`
    animating, with the body reading `⠙ listening for mDNS services on local…` —
    the *same* frame in both places, which is the agreement made structural.
  - **Refresh**: `✓` → `r` → `⠼` (live again) → `✓`, i.e. through the
    replacement session's own state.
  - **Narrow**: 20×8 renders the complete indicator safely.
- **Follow-ups:**
  - **`Failed` was not driven through the real app**, only unit-tested (render
    frame + body text + `ui::app`'s injected-failure tests). A real failure needs
    a browse that starts and then dies, which the CLI cannot ask for; the sample
    backend cannot fail by design. Recorded rather than left unstated, per
    `CONTEXT.md`. A `--backend` that fails on demand would close this, but that
    is a new fixture, not this task's scope.
  - No ADR: this adds no enduring architectural choice beyond task 002's
    lifecycle, which it renders rather than changes.
