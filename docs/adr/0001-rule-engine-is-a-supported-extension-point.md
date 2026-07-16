# ADR 0001: `RuleEngine` is a supported extension point

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-07-17 |
| Decider | Ivan Ryabov (project owner) |
| Context | [review-backlog task 015](../review-backlog/tasks/015-app-encapsulation.md), [task 022](../review-backlog/tasks/022-rule-engine-seam.md) |

## Context

The July 2026 review asked whether `plumber::RuleEngine` earns its place.

The case for deleting it was the deletion test from
[`review-backlog/CONTEXT.md`](../review-backlog/CONTEXT.md): the trait declares
three methods, each forwards straight to the inherent method of the same name on
`Matcher`, and `Matcher` is the only implementation in the tree. By that rule —
"a seam with one adapter is hypothetical; a seam with two adapters is real" —
the trait is complexity that would vanish if removed, and its cost is real:
`Box<dyn RuleEngine>` in `ReloadOutcome`, in `App`, and a trait-object
conversion at the composition root.

The counter-case is that the deletion test as written assumes every caller lives
in this repository. Kinjo is not a binary with an incidental `lib.rs`. It is
published to crates.io, and `src/lib.rs` deliberately exposes `discovery`,
`plumber`, and `ui` as public modules. `App::new` is public and accepts
`impl RuleEngine + 'static`; `ui::app`, `ReloadOutcome`, and `App::run` are
public too. A third-party crate can therefore depend on `kinjo`, implement
`RuleEngine` over its own matching strategy, and drive it from its own
composition root without forking this repository. That path exists today and
compiles today.

So the adapter count in this tree is not evidence that the seam is unused. It is
evidence that we ship one engine. Those are different claims, and only the first
one supports deletion.

### What discovery does, and why it is not a precedent

An earlier draft of this ADR argued that `RuleEngine` mirrors the discovery
seam, which the review already accepts as real. **That argument is wrong and is
recorded here so it is not made again.**

There is no `Discovery` trait. `discovery::start` is a function that dispatches
on a closed `DiscoveryBackend` enum, and `src/discovery/mod.rs` states the
reasoning outright: the adapter seam lives *inside* the module at the browse
loop, because `mdns-sd`, `zeroconf`, and the fake genuinely differ in how they
browse but not in how a caller runs and stops them — "that is why there is one
concrete session type rather than a trait over receivers". A dependent crate
cannot add a discovery backend at all.

Discovery therefore has three adapters and *no* public trait; `RuleEngine` has
one adapter and *is* a public trait. They are not two instances of one pattern.
They are opposite decisions, and discovery's is the better-argued of the two:
its seam is where the variation actually is.

That weakens the case for `RuleEngine` rather than strengthening it, and this
decision is taken with that understood. `RuleEngine` is kept on its own merits —
it is already published API, and matching strategy is a place where an outside
crate could plausibly want to differ — not because discovery set a precedent.
`README.md` claimed two trait seams and was simply wrong about one of them; it
has been corrected.

## Decision

Keep `RuleEngine` as a **supported** extension point, not a speculative one.

"Supported" is a commitment with consequences, and this ADR is the place they
are written down:

- The trait is public API. Breaking it is a semver event for the crate.
- Its interface is designed for an implementor who is not `Matcher`. Where the
  current interface only makes sense for `Matcher`, that is a defect to fix
  rather than a shape to preserve (see task 022).
- It carries a test that implements it from outside `Matcher`, so the claim
  "someone else can extend this" is checked by CI rather than asserted in a
  README.
- The documented extension path is honest about its limit: substitution works by
  writing your own composition root against `App::new`. `run()` hardcodes
  `ui::config::load_matcher`, so the seam is not reachable through `run()`, and
  the documentation says so instead of implying otherwise.

Deleting the trait is explicitly rejected. The `Box<dyn RuleEngine>` indirection
and its dynamic dispatch are accepted as the price of the extension point; the
matcher is consulted once per recompute, not per frame, so the cost is not on a
hot path.

## Consequences

Positive:

- The extension intent documented in `README.md` stays true rather than being
  reversed.
- Third parties keep a supported way to substitute a matching strategy.
- Task 022 turns a forwarding trait into an interface that an outsider can
  actually implement, which is work that deletion would have simply discarded.

Negative, accepted knowingly:

- One in-tree adapter means the seam is validated by a test rather than by a
  second real engine. A test adapter is weaker evidence than a shipped one, and
  we accept that gap rather than pretend it does not exist.
- `Box<dyn RuleEngine>` and dynamic dispatch remain in `ReloadOutcome` and
  `App`.
- Public API is now something we owe compatibility to, which constrains future
  refactors of `plumber`.
- `RuleEngine` is now the *only* public extension trait in the crate, and it
  sits next to a discovery module that reached the opposite conclusion for
  defensible reasons. The crate is therefore not of one mind about seams, and a
  future reader will notice. This ADR is the answer to "why is this one
  different": because it is already public, not because it is better justified.
- The extension path is real but narrow: a substituting crate must reimplement
  the composition root, since `run()` is not generic. If that turns out to be
  the actual barrier to someone extending kinjo, the trait was not the thing
  standing in their way, and this decision should be revisited.

## Notes

This ADR overrides the blanket rule in `review-backlog/CONTEXT.md` that a
one-adapter seam is hypothetical, for this seam only. The rule remains a good
default for internal abstractions; it does not decide the fate of a published
crate's public API, where the adapter that justifies a seam may live in a
repository we cannot see. `CONTEXT.md` has been updated to point here so the
question is not silently reopened.
