# 022: Make the Retained `RuleEngine` Seam Implementable

Shared context: [`CONTEXT.md`](../CONTEXT.md).
Decision: [ADR 0001](../../adr/0001-rule-engine-is-a-supported-extension-point.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P2` |
| Workstream | Architecture |
| Depends on | — (independent of 015; coordinate if both are in flight) |
| Likely conflicts | 015 (both touch `src/ui/app.rs` and `src/lib.rs`) |
| Owner | Claude (branch `main`) |

## Why This Matters

Task 015 originally proposed deleting `RuleEngine`. ADR 0001 rejected that: the
seam stays, as a *supported* extension point rather than a speculative one.

That decision creates work rather than avoiding it. The trait was written as a
mirror of `Matcher`'s inherent methods, and it shows. Today it is shaped for the
one implementor it has, which is exactly the property that makes a seam
hypothetical — not the adapter count. If an outsider cannot implement the trait
without contorting their design to look like `Matcher`, the extension point is
decorative, and the ADR's promise is not kept.

This task makes the claim in `README.md` true and checkable.

## Evidence

Re-checked at the head of `main` on 2026-07-17; line numbers are anchors, not
authority.

- `src/plumber/mod.rs:378-399`: the trait and its sole `impl for Matcher`, each
  method forwarding to the inherent method of the same name.
- `src/plumber/mod.rs:382`: `fn commands(&self) -> &[CommandConfig]` returns a
  borrowed contiguous slice. This is `Matcher`'s private storage
  (`src/plumber/mod.rs:135-137`, `commands: Vec<CommandConfig>`) promoted into
  the interface. An implementor that stores rules in a map, generates them
  lazily, or wraps them cannot return `&[CommandConfig]` without keeping a
  redundant `Vec` purely to satisfy the signature.
- `src/lib.rs:114`: `run_invocation` calls `ui::config::load_matcher`, which
  returns a concrete `Matcher`. The seam is unreachable through `run()`; the
  supported path is an external composition root using `App::new`.
- `src/lib.rs:6`: the crate doc says plumber sits "behind the
  [`plumber::RuleEngine`] trait". The default path does not go through the trait
  at all, so this overstates what is true.
- `src/lib.rs:145`: `Box::new(matcher) as Box<dyn RuleEngine>` — the one
  conversion, in the one place a caller would need to mirror.
- `README.md:323,330` and `CONTRIBUTING.md:165`: document the seam as an intended
  extension point without saying how to reach it.
- No test implements `RuleEngine` for any type other than `Matcher`.

## Required Outcome

- `commands()` no longer requires the implementor to own a contiguous
  `Vec<CommandConfig>`. Choose the smallest change that lifts the constraint and
  record why; do not redesign the trait beyond what an implementor needs.
- The trait's doc comment states its contract as obligations on an implementor —
  what each method must return, ordering guarantees, and what callers may assume
  — rather than describing what `Matcher` happens to do.
- A test implements `RuleEngine` over a type that is *not* `Matcher` and is not
  backed by a `Vec<CommandConfig>`, and drives it through `App` to prove the
  substitution path in ADR 0001 works. This is not a new seam added for testing
  (`CONTEXT.md` forbids that); it is coverage of an existing public interface.
- `README.md`, `CONTRIBUTING.md`, and the `src/lib.rs` crate doc describe the
  extension path accurately, including its limit: substitution happens at
  `App::new` in your own composition root, not through `run()`.
- Behavior is unchanged. `Matcher` remains the only shipped engine and the
  default path behaves identically.

## Implementation Constraints

- Behavior-preserving. No change to matching semantics, rule loading, or
  execution.
- Preserve the dependency direction from `CONTEXT.md`: `discovery ← plumber ← ui`.
  The trait lives in `plumber` and must not learn about the UI.
- Do not weaken the invariants of tasks 004–006: predicate conjunction, candidate
  distinctness, and grouped-action targeting are `CommandConfig`'s and
  `MatchResult`'s contracts, not the engine's, and must not migrate into the
  trait.
- If a change here would be easier after 015 privatises App, say so and sequence
  behind it rather than fighting the merge.
- Resist widening the trait. Each method must be one an outside engine genuinely
  has to provide; if `Matcher` is the only plausible source of a method, it does
  not belong.

## Open Question for the Implementer

`commands()` has at least three defensible answers, and the right one depends on
what an implementor actually needs:

1. Return an owned `Vec<CommandConfig>` or a boxed iterator — frees storage,
   costs an allocation per call. Check the call sites first:
   `src/ui/app.rs:603` and `src/lib.rs` (`write_commands`) are not hot, but
   confirm rather than assume.
2. Keep `&[CommandConfig]` and accept that implementors must materialise a slice.
   Cheapest, but is the constraint ADR 0001 calls a defect.
3. Narrow it to what callers actually use. `list-commands` needs to enumerate
   rules for display; `App` needs a count and enumeration. If no caller needs
   `CommandConfig` *by reference for mutation or identity*, the interface may be
   narrower than it looks.

Inventory the call sites, then decide and record it in the task. Do not pick
based on what is least work in `Matcher`.

## Non-Goals

- A plugin system, dynamic loading, or third-party registration.
- Making `run()` engine-generic. ADR 0001 accepts that `run()` is the concrete
  default path; the seam is reached by writing a composition root.
- New matching strategies. This task ships no second engine.
- App encapsulation (task 015).

## Acceptance Criteria / Definition of Done

- [ ] `RuleEngine`'s interface is implementable without `Vec<CommandConfig>`
      storage, and the chosen approach is recorded with its call-site evidence.
- [ ] The trait's documentation is written as an implementor's contract.
- [ ] A non-`Matcher` engine is implemented in tests and driven through `App`.
- [ ] `README.md`, `CONTRIBUTING.md`, and the crate doc state the extension path
      and its `run()` limitation accurately.
- [ ] `Matcher` behavior and all rule-related regressions from tasks 003–007 are
      unchanged.
- [ ] Full validation gate passes.

## Required Tests

- A test engine implementing `RuleEngine` over non-`Vec` storage, exercised
  through `App` (open an action picker against it, and reach a prepared command).
- ~~A test that the `list-commands` path works against the test engine, since it
  is the other `commands()` consumer.~~ **Withdrawn during implementation.** The
  premise was wrong: `write_commands(writer, matcher: &Matcher)` in `src/lib.rs`
  takes the concrete `Matcher` and calls its *inherent* `commands()`, so
  `list-commands` never goes through the trait. `App` is the trait's only
  consumer. Testing a `list-commands` path that does not exist would have
  asserted a fiction.
- Existing `plumber` and `ui::app` suites unchanged and green.

## Validation

```sh
cargo test --locked plumber
cargo test --locked ui::app
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
  - `RuleEngine::commands` now returns `Vec<CommandConfig>` instead of
    `&[CommandConfig]`, lifting the requirement that an engine store its rules
    contiguously. Chosen from the open question's option 3 on call-site evidence:
    `App` was the trait's *only* consumer of `commands()`, and it already cloned
    every rule into a `CommandGroup`, so owning costs the real caller nothing.
    `src/ui/app.rs` now moves them instead — one clone per rule per recompute
    removed.
  - `command_count` became a provided method defaulting to `self.commands().len()`,
    so a minimal engine implements two methods rather than three. `Matcher`
    overrides it, since its rules are already a `Vec` and need not be cloned to
    be counted.
  - The trait's docs are now an implementor's contract: the name-agreement
    invariant between `matches_group` and `commands` (the UI joins them by name
    and silently drops a match naming an unlisted rule), name uniqueness,
    purity, order stability, and the non-empty-targets rule.
  - `Matcher`'s inherent `commands() -> &[CommandConfig]` is untouched, so every
    concrete caller — tests, fuzz targets, `ui/config.rs`, `write_commands` —
    keeps its borrow and Rust's inherent-first resolution keeps them on it.
- **Tests added/updated:** `tests/rule_engine_extension.rs` (new, 7 tests).
  Placed in `tests/` deliberately: the ADR's claim is about *external* crates, so
  a unit test in `src/` could reach private items a real extender cannot and
  would beg the question. `KeyedEngine` stores rules in a `BTreeMap` and matches
  by keyed lookup, so it cannot satisfy a `&[CommandConfig]` return — the test
  stops compiling if the interface regresses. Covers the trait object, the
  default `command_count`, foreign matching, preparing a command from a foreign
  rule, the reload path (`ReloadOutcome::Loaded(Box::new(foreign))`), and
  `App::new` composition (feature-gated on `fake`, which is the only session an
  external caller can start without touching the network).
- **Documentation updated:** ADR 0001 (corrected, see below), `README.md`
  (architecture + new "Extending it" section), `CONTRIBUTING.md` (project layout
  + `docs/adr/` pointer), `src/lib.rs` crate docs.
- **Validation evidence:**

  ```text
  cargo fmt -- --check                                        pass
  cargo clippy --locked --all-targets --all-features -- -D warnings
                                                              pass
  cargo test --locked --all-targets            410 lib + 6 integration
  cargo test --locked --all-targets --all-features
                                               436 lib + 7 integration
  cargo +nightly fuzz build                    5 targets built
  ```

  Library counts are unchanged from baseline (verified by stashing the source
  change and re-running: 410 default, 436 all-features), so the refactor is
  behavior-preserving; the additions are the new integration tests. Note the
  backlog's recorded 409/435 were both off by one *before* this task.

  Drove the real TUI at 100×30 on `--backend fake` with a two-rule config dir,
  since the fake backend ships no commands and the command view — the code this
  task changed — is dead without them. The group-by-command tab listed both
  rules in load order with correct match counts (`ssh ★2 svc`), and `Enter`
  opened the "run ssh on" service picker offering both matching services.
- **Follow-ups:**
  - **Corrected an error in ADR 0001 while implementing it.** The ADR argued
    `RuleEngine` mirrored a `discovery::Discovery` trait seam. There is no such
    trait: `discovery::start` dispatches on a closed `DiscoveryBackend` enum, and
    `src/discovery/mod.rs:12-15` explains why — the adapters differ in how they
    browse but not in how a caller runs them, so the seam sits inside the module.
    Discovery has three adapters and no public trait; `RuleEngine` has one
    adapter and is a public trait. They are opposite decisions, not one pattern.
    The ADR now records the mistaken argument and the honest reason the seam is
    kept. `README.md:330` had asserted "two trait seams (`Discovery` and
    `RuleEngine`)", which was the likely source of the error; it is corrected.
  - The dead `list-commands` requirement was withdrawn; see Required Tests.
  - No new task raised. Task 015 inherits `tests/rule_engine_extension.rs` as a
    guard on the public composition path.
