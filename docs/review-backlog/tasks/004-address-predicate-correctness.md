# 004: Make Address Predicates Conjunctive per Candidate

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P0` |
| Workstream | Command rules |
| Depends on | — |
| Likely conflicts | 005, 006 |
| Owner | Unclaimed |

## Why This Matters

Each address predicate currently checks whether any address satisfies it. Two
predicates can therefore be satisfied by different addresses even though no
single executable candidate satisfies the complete rule. Candidate expansion
then falls back to every address, allowing execution against an address that
violates the configured predicates.

The command-rule module must answer one coherent question: which concrete
candidates satisfy the whole rule?

## Evidence

- `src/plumber/mod.rs:164-173`: each address predicate independently calls
  `any(...)` over the entry's addresses.
- `src/plumber/mod.rs:131-140`: candidate narrowing requires one address to
  satisfy all address predicates, but an empty result falls back to all addresses.
- `src/plumber/mod.rs:64-84`: matching and candidate expansion are separate steps,
  allowing the two interpretations to disagree.

Example: an entry with `10.0.0.1` and `2001:db8::1`, plus predicates containing
`10.` and `:`. Each predicate matches some address; no address matches both.
The command must not be offered.

## Required Outcome

- Evaluate all address predicates as a conjunction over one concrete address.
- Produce only addresses satisfying every address predicate.
- If address predicates exist and no address satisfies all of them, the entry does
  not match and no action is offered.
- When a template references `{address}` but no address predicate exists, expand
  all available addresses as candidates for user disambiguation.
- Entries without an address cannot satisfy an address predicate or produce a
  concrete candidate for a `{address}` template. Such an action is not offered;
  it is not deferred to a preparation error.

## Implementation Constraints

- Keep matching/candidate behavior in one module so they cannot drift again.
- Preserve predicate semantics for non-address fields.
- Do not introduce address ordering changes unless required for deterministic
  candidate presentation; preserve the entry's existing stable order.
- Update the `prepare_command` fuzz properties only if its inputs/interface change.

## Suggested Implementation Sequence

1. Add a regression where separate addresses satisfy separate predicates.
2. Derive candidate addresses once using all address predicates.
3. Make record matching depend on the non-empty concrete candidate set.
4. Retain all-address expansion only for address templates without predicates.

## Non-Goals

- General grouped-action disambiguation; task 006 owns it.
- Compiling templates/configs at load time; task 005 follows this task.
- Adding OR/NOT predicate syntax.

## Acceptance Criteria / Definition of Done

- [ ] Mutually incompatible address predicates yield no match.
- [ ] Multiple compatible predicates yield only satisfying addresses.
- [ ] `{address}` without address predicates expands every available address.
- [ ] No-address entries fail address-dependent rules cleanly.
- [ ] Existing single-address and multi-address behavior remains covered.
- [ ] Full validation passes.

## Required Tests

- Dual-stack entry with predicates that match different addresses: no result.
- Several addresses with two predicates matching one address: one candidate.
- Several addresses, `{address}`, no address predicates: all candidates.
- Missing address with predicate/template: no match/executable candidate is offered.

## Validation

```sh
cargo test --locked plumber::tests
cargo test --locked plumber::exec::tests
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
