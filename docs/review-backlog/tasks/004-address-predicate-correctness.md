# 004: Make Address Predicates Conjunctive per Candidate

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P0` |
| Workstream | Command rules |
| Depends on | — |
| Likely conflicts | 005, 006 |
| Owner | agent-a7769bd1fe322fb1e |

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

- [x] Mutually incompatible address predicates yield no match.
- [x] Multiple compatible predicates yield only satisfying addresses.
- [x] `{address}` without address predicates expands every available address.
- [x] No-address entries fail address-dependent rules cleanly.
- [x] Existing single-address and multi-address behavior remains covered.
- [x] Full validation passes.

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

- **Implemented:** Evidence re-verified against `dd872b7`; all three anchors were
  still accurate at the quoted lines. Merged matching and candidate expansion
  into one operation in `src/plumber/mod.rs` so they cannot drift:
  - `Matcher::matches_group` no longer filters by `matches_record` and then
    separately expands; it only flat-maps `CommandConfig::candidates`.
  - New `CommandConfig::candidates` is the single answer to "which concrete
    candidates satisfy the whole rule?". It checks non-address predicates
    against the record, returns the record unchanged when the command cannot
    distinguish addresses, and otherwise returns one single-address candidate
    per address satisfying *every* address predicate, in the entry's existing
    order. `matches_record`/`candidate_instances` are gone.
  - Removed the all-address fallback: an empty candidate set now means "no
    match", so an address violating the predicates can no longer be executed.
  - Removed the `record.addresses.len() <= 1` short-circuit, which let a
    `{address}` rule offer an address-less entry and defer to a preparation
    error. Such rules are now simply not offered.
  - `FieldPredicate::matches` no longer special-cases `address` with `any(...)`
    (the root of the disagreement); it is documented as non-address-only and
    holds a `debug_assert!`. Address field spelling is centralised in a new
    `is_address_field`, which `is_instance_field` now reuses.
  - Non-address predicate semantics and the public `MatchResult`/`RuleEngine`
    interface are unchanged, so no UI change was required.
- **Tests added/updated:** 7 tests added in `src/plumber/mod.rs` `tests`, all
  driving the real `MatcherBuilder` → `Matcher::matches_group` interface (no
  test-only seam), with `matcher_with`/`workstation_group`/`candidate_addresses`
  helpers: `address_predicates_satisfied_by_different_addresses_do_not_match`
  (the dual-stack `10.` + `:` case from the task),
  `address_predicates_are_conjunctive_over_one_address`,
  `address_template_without_predicates_expands_every_address_in_order`,
  `entry_without_address_does_not_satisfy_an_address_predicate`,
  `entry_without_address_offers_no_candidate_for_an_address_template`,
  `address_predicate_still_matches_a_single_satisfying_address`,
  `commands_without_address_use_keep_all_addresses_on_one_candidate`. The
  regressions were confirmed to fail (2 failures) against the pre-fix fallback
  before the fix was restored. `prepare_command` fuzz properties were reviewed
  and left unchanged: the target only exercises `exec::prepare`/`CommandAction`,
  whose inputs and interface this task does not touch.
- **Documentation updated:** `docs/actions.md` — documented that all
  `[match.address]` predicates apply to the same single address (with the
  unsatisfiable dual-stack example), and that an `{address}` command offers only
  satisfying addresses and is not offered at all for an unresolved entry.
- **Validation evidence:** Full gate passed on this worktree.
  - `cargo fmt -- --check`: clean.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`: clean.
  - `cargo test --locked --all-targets`: 191 passed, 0 failed (184 before this
    change: +7).
  - `cargo test --locked --all-targets --all-features`: 196 passed, 0 failed
    (189 before: +7).
  - Note: the baseline in `CONTEXT.md` (163/166) predates tasks 001/002/012/016.
- **Follow-ups:**
  - Out of scope here (task 006 owns it): a rule with an `address` predicate but
    no `{address}`/`{port}` in its template still reports `needs_instance` and
    can expand to several candidates that prepare *identical* argv, producing a
    redundant instance picker. This is the "identical prepared commands may
    collapse" decision in `CONTEXT.md`; behaviour was preserved as-is.
  - Task 005 can now compile the address predicate conjunction at load time; the
    per-address evaluation is already isolated in `address_predicates_match`.
