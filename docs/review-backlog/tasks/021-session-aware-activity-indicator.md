# 021: Make the Discovery Activity Indicator Session-Aware

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P2` |
| Workstream | UI / Discovery state |
| Depends on | 002, 010, 014 (all done) |
| Likely conflicts | 014, 015 |
| Owner | Unclaimed |

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

- [ ] Listening animates and complete/failed states do not.
- [ ] Complete and failed indicators are visually distinguishable and truthful.
- [ ] Refresh transitions the indicator through the replacement session's state.
- [ ] The top bar and empty/body lifecycle messages agree in every state.
- [ ] Render consumes session state through task 014's immutable view interface.
- [ ] Tiny-width rendering is stable and panic-free.
- [ ] Full validation passes.

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
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
