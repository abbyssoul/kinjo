# Task 113 — Retire the dead `prepare` failure path

- **Priority**: P2 (maintainability)
- **Status**: done
- **Depends on**: 101 (ADR 0003 is what made this dead)
- **Likely conflicts**: none

## Problem

ADR 0003 made `argv[0]` entirely literal. Task 101's completion record answered
the "is `prepare` now infallible?" question for `prepare`'s *signature* — it
stays fallible because `render` can fail for a caller-supplied `Entry` missing a
referenced field — but two artefacts of the old, dynamic-program world survived
underneath it, and both now describe a case that cannot occur.

**1. `CommandAction::prepare`'s empty-`argv[0]` check is unreachable, and its
comment contradicts ADR 0003.**

```rust
// Compilation rejects a literally-empty program name; a placeholder one
// can only be judged now that it has a value.
if argv[0].is_empty() {
    return Err(eyre!("action command has an empty program name"));
}
```

There is no longer a "placeholder one" to judge — ADR 0003 prohibits a field
fragment in the program token. And the value cannot be empty for any caller:
`flush_literal` never pushes an empty `Fragment::Literal`, and
`CommandTemplate::compile` rejects a program token with no fragments, so a
literal-only `argv[0]` is a concatenation of at least one non-empty literal.
Task 109 separately made `plumber::exec` validate an empty program name at the
process boundary, which is where a hand-built `PreparedCommand` is actually
caught, so this check is redundant as well as dead.

**2. `distinct_targets`' doc overclaims a guarantee nothing can exercise.**

> A candidate that fails to prepare is kept as a target rather than dropped, so
> a rule that cannot run for the chosen service can never quietly run against a
> different one instead.

That was round-1 task 006's rule and it was real then. It cannot arise now:
`distinct_targets` is private, reached only from `Matcher::matches_group`, and
fed only by `CommandConfig::candidates`, which already gates on
`template_fields_resolve` for every non-address field and yields one concrete
address per candidate when the rule uses one. Every field therefore resolves
before `prepare` is called. The test that pinned the rule
(`a_candidate_that_cannot_prepare_is_offered_rather_than_skipped`) was removed
by task 101 — correctly, since its `{name} --version` vehicle is now rejected at
load — with no replacement, because no replacement is constructible through the
module's real interface.

Neither is a bug. Both are the residue round 1 was careful about: a check that
cannot fire, and a comment asserting a property no test can hold it to.

## Decision

Delete the dead check. **Keep** the `Option<PreparedCommand>` dedup key in
`distinct_targets` and correct its doc.

The asymmetry is deliberate. The check is dead for *every* caller and duplicates
a real guard at the process boundary, so it removes no way of being wrong —
round 1's own deletion test says it goes. The `Option` is different: it is
unreachable only because of an invariant enforced *elsewhere*
(`candidates`/`template_fields_resolve`). Removing it would make `distinct_targets`
depend on that invariant silently, and a future change loosening candidate gating
would then either panic or drop a candidate — reintroducing exactly the defect
task 006 fixed. It stays, described honestly as defence rather than as a live path.

## Scope

- `src/plumber/mod.rs`: remove the empty-`argv[0]` check and its false comment
  from `CommandAction::prepare`. `prepare` keeps `-> Result`: `render` can still
  fail for an `Entry` a library caller supplies that is missing a referenced
  field, which is the fallibility task 101's record identified.
- `src/plumber/mod.rs`: rewrite the `distinct_targets` paragraph so it states
  the failure path is unreachable from the matcher and why it is retained.

## Out of scope

- `exec`'s empty-program and empty-argv validation (task 109) — that is the live
  guard and stays.
- `CommandTemplate::compile`'s empty-program rejection — still reachable, still
  tested.

## Tests

No behaviour changes, so no new test. The existing suite must stay green
unchanged; `template.rs`'s `empty_and_whitespace_only_commands_are_rejected` and
`exec.rs`'s empty-program tests already cover the two live guards that remain.

## Definition of Done

- The dead check and its contradicting comment are gone.
- `distinct_targets`' doc no longer claims a path the matcher cannot reach.
- Completion gate green, test count unchanged.

## Completion Record (2026-07-17)

- Removed `CommandAction::prepare`'s unreachable empty-`argv[0]` check and the
  comment that still described ADR 0003's prohibited dynamic program name.
  `prepare` remains `-> Result` for the public-caller case `render` genuinely
  has.
- Rewrote the `distinct_targets` paragraph: the failure-preserving branch is
  now described as defence against a loosening of candidate gating, not as a
  case the matcher produces, and names the invariant (`template_fields_resolve`)
  that makes it unreachable today.
- No behaviour change and no test change. Gate green: fmt, clippy
  (`--all-targets --all-features -D warnings`), 436 + 6 default,
  462 + 7 all-features, 1 ignored (task 102's manual benchmark), fuzz targets
  check clean — identical to the counts before this task.
