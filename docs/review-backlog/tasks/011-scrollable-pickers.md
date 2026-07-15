# 011: Keep Picker Selection Visible

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P1` |
| Workstream | UI |
| Depends on | 008, 010 |
| Likely conflicts | 012, 013, 014, 015 |
| Owner | Unclaimed |

## Why This Matters

Type, action, occurrence, and service pickers build a full stateless list and
manually style the selected item. Selection indexes can move beyond the modal's
visible height, leaving the active choice off-screen. The user can then run an
invisible target, and terminal resize can make a previously visible target vanish.

One picker viewport module should own the selected-index-to-visible-window
invariant for every modal.

## Evidence

- `src/ui/app.rs:409-425,457-500,648-673`: picker selection moves across the
  complete item count with no viewport/offset state.
- `src/ui/render.rs:700-815`: modal renderers build complete item vectors.
- `src/ui/render.rs:859-885`: popup content is rendered without list selection or
  scroll state.
- `src/ui/render.rs:1132-1147`: `build_list_items` only applies selection style.

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
- Modal operation remains keyboard-driven unless a separate task adds mouse input.

## Implementation Constraints

- Use structured selection identity/index supplied by the mode-aware model from
  task 010; do not infer selection from styled text.
- Prefer a pure viewport calculation over four copies of offset mutation logic.
- Rendering should consume the calculated window; it must not secretly mutate
  unrelated application state through `Cell`.
- Preserve current deterministic item ordering.

## Suggested Implementation Sequence

1. Add pure viewport tests for start/middle/end, empty, and resize cases.
2. Implement one picker viewport calculation returning offset/range.
3. Apply it to all four picker render paths.
4. Add a scrollbar or compact range indicator only when clipping occurs.
5. Add Ratatui TestBackend assertions that the highlighted row is present.

## Non-Goals

- Mouse support inside modals.
- Changing picker target-disambiguation policy.
- Visual-theme redesign.
- General browse-list scrolling, which already follows the selected row.

## Acceptance Criteria / Definition of Done

- [ ] Selected item remains visible at every index in every picker type.
- [ ] Shrinking/growing the terminal retains a visible selected item.
- [ ] Empty/zero-height modals do not panic or underflow.
- [ ] Clipped pickers visibly communicate their range/position.
- [ ] All picker implementations use the shared viewport calculation.
- [ ] Full validation passes.

## Required Tests

- Pure viewport: list shorter/equal/longer than viewport, first/middle/last selection.
- TestBackend: oversized TypeFilter, ActionPicker, InstancePicker, ServicePicker.
- Resize from selection-visible to selection-clipped dimensions.
- Unicode labels and one-row viewport.

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
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
