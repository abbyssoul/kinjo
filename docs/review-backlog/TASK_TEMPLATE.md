# NNN: Task title

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P0` / `P1` / `P2` |
| Workstream | Discovery / Command rules / UI / Architecture |
| Depends on | Task IDs or — |
| Likely conflicts | Task IDs or — |
| Owner | Unclaimed |

## Why This Matters

Describe the observable bug, safety risk, or architectural friction and its user
impact. Use the domain and architecture language from [`CONTEXT.md`](CONTEXT.md).
When copying this template under `tasks/`, change that link to `../CONTEXT.md`.

## Evidence

- `path/to/file.rs:line`: current behavior and why it is relevant.
- Include a reproduction when one exists.

Line numbers are starting points. Revalidate them against the current branch.

## Required Outcome

State the externally observable behavior and important invariants. This section
must be specific enough that an implementer does not need to make a product
decision.

## Implementation Constraints

- Required compatibility or module dependency direction.
- Trust/safety constraints.
- Architectural locality/depth requirements.

## Suggested Implementation Sequence

1. Add a failing regression test through the affected module's interface.
2. Make the smallest coherent behavior change.
3. Refactor only where required to concentrate the invariant.
4. Update user-facing documentation.

## Non-Goals

- List tempting adjacent changes that must not be folded into the task.

## Acceptance Criteria / Definition of Done

- [ ] Observable required behavior is implemented.
- [ ] Regression tests cover success and failure/edge behavior.
- [ ] Public documentation matches the behavior.
- [ ] No unrelated changes are included.
- [ ] Targeted and full validation pass.

## Required Tests

- Name concrete scenarios and the most appropriate existing test module.

## Validation

```sh
# Add targeted commands first.
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

Fill this in before marking the task `done`:

- **Implemented:**
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
