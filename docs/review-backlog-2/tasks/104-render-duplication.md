# Task 104 — Render duplication: tree rows and description precedence

- **Priority**: P2 (maintainability)
- **Status**: ready
- **Depends on**: none
- **Likely conflicts**: 103 (same file)

## Problem

`src/ui/render.rs` repeats two patterns enough that a future edit will change
one copy and miss the others.

### 1. The `├─`/`└─` tree-branch row loop, three times

The "last child gets `└─`, the rest get `├─`, build a two-cell `Row`" loop
appears three times with only the payload differing:

- occurrence rows in `push_logical_service_rows` (`render.rs:555-578`),
- child-service rows in `push_child_service_rows` (`render.rs:646-690`),
- matching-service rows in `command_detail_rows` (`render.rs:947-966`).

Each recomputes `let last = len.saturating_sub(1)` and the
`if i == last { "└─" } else { "├─" }` branch by hand.

### 2. The description fallback chain, four times, two orders

The "action description, else metadata description, else empty" fallback is
hand-rolled four times:

- `action_row` (`render.rs:779-785`) — **action-first**,
- `render_action_picker` (`render.rs:1144-1152`) — **action-first**,
- `command_row` (`render.rs:858-863`) — **metadata-first**,
- `command_detail_rows` (`render.rs:914-918`) — **metadata-first**.

The two precedence orders are **intentional**: `docs/actions.md` says
`action.description` "is shown in the action picker", so the action-level
description wins where an *action* is being shown, and the metadata description
wins in the *command* view. The problem is that this deliberate difference is
expressed as four inline `.or()` chains — nothing names the two policies, so a
well-meaning future edit can "consolidate" them into one order and silently
break the documented behaviour in two places.

## Goal

Replace the three tree loops with one helper, and the four description chains
with two named helpers whose names state which precedence they encode. No
behaviour change: the same rows, spans, colours, and description text as today.

## Suggested approach

- A `tree_rows`/`push_tree_rows` helper that takes the children and a closure
  producing each row's cells (or the branch prefix plus a payload closure),
  and owns the `last`/`├─`/`└─` logic. The three call sites differ in cell
  content, gutter colour, and endpoint formatting — the helper should carry the
  branch, the callers the payload.
- Two functions, e.g. `action_description(command)` (action-first) and
  `command_description(command)` (metadata-first), each returning `&str` or
  `Option<&str>`, with a doc comment citing `docs/actions.md` for why the
  orders differ. Replace all four inline chains with the matching call.

## Constraints

- Byte-for-byte identical rendered output for a fixture in every grouping mode
  and both pickers. This is a pure refactor.
- Keep render pure over `App` (ADR 0002).
- Coordinate with task 103 if it lands first — it also edits the details
  builders. Whichever is second rebases onto the first.

## Tests

- Existing render tests stay green unchanged.
- Add a small unit test for the two description helpers pinning both
  precedence orders (action-first returns the action desc when both present;
  metadata-first returns the metadata desc when both present; both fall through
  to the other, then to `None`/empty).

## Definition of Done

- Three tree loops → one helper; four description chains → two named helpers.
- Description-precedence helpers documented against `docs/actions.md`.
- Rendered output unchanged (drive `scripts/drive-tui.sh` and compare).
- Completion gate green.
