# Task 107 — Bound hostile record growth

- **Priority**: **P1** (safety / resource use) — upgraded from P2; see the
  follow-up validation note and [ADR 0005](../../adr/0005-discovery-overload-fails-closed.md)
- **Status**: done
- **Depends on**: **none** — no longer depends on 102. Ingress bounds are safety
  work and must not wait on projection optimisation.
- **Likely conflicts**: 102 (final record cap coordinates with it), 108 (same
  ADR; design together)

> **Reconciliation (2026-07-17).** The scope question this task and its
> follow-up note raised is decided in
> [ADR 0005](../../adr/0005-discovery-overload-fails-closed.md), which bounds
> the whole ingress path rather than just the final map. **The ADR supersedes
> the "Goal", "Suggested approach", and "Definition of Done" below**; they are
> retained as the record of how the problem was found. See "Implementing ADR
> 0005" at the foot of this file.

## Problem

`App::records` (`src/ui/app.rs:177`) is a `BTreeMap<EntryId, Entry>` that grows
with every distinct occurrence discovery reports and only shrinks on explicit
`Remove`/`RemoveRegistration` or session failure (`drain_discovery`,
`app.rs:434-455`). There is no upper bound.

An `EntryId`'s identity includes the registration `(name, service_type,
domain)` and, for the mdns-sd backend, an occurrence discriminator
(`entry.rs:65-100`, `299-312`). A device on the link fully controls all of
these fields. A hostile or misconfigured host can therefore mint unbounded
distinct registrations (`evil-0`, `evil-1`, …) or unbounded endpoints, each a
new map entry. Every insertion also feeds the recompute pipeline (task 102),
which is superlinear in a couple of places, so growth is amplified into CPU as
well as memory.

The liveness tracker (`mdns.rs`) probes and eventually removes silently-dead
services, which caps *steady-state* count for well-behaved networks — but an
attacker who keeps re-announcing defeats that, and the probe cycle itself grows
with the tracked set.

This is not a live incident (a normal LAN has tens to low-hundreds of
services), and it is deliberately P2. But an mDNS tool's entire input is
untrusted by definition, and "the network can OOM the client" is worth a
bound.

## Goal

Put a defensible ceiling on how much discovery can make the app hold and
recompute, chosen so a real network is never affected and a flood degrades
gracefully (drops/ignores the excess with a visible status) rather than growing
without limit.

## Suggested approach (agent to choose and justify)

- A configurable-but-defaulted cap on `records.len()` (e.g. a few thousand),
  enforced in `drain_discovery` when applying `Upsert`. On overflow: reject new
  *registrations* while still accepting updates to already-known ones (so a
  legitimate service already listed keeps resolving), and set a status line
  that says discovery is capped. Prefer dropping the newest unknown over
  evicting a known one — eviction policy is a real decision; document it.
- Consider whether the cap belongs in the UI layer (`App`) or the discovery
  layer. The trust boundary is discovery's, but the resource being protected
  (records + recompute) is the UI's. Either is defensible; record the choice.
- Keep it observable: a silently capped list looks like a quiet network, which
  the round-1 trust model forbids for *failures* and the same reasoning applies
  here. The user must be able to tell "capped" from "that's all there is".

## Constraints

- A normal network must never hit the cap. Choose the default with headroom.
- Do not break occurrence identity or removal semantics: an evicted/rejected
  record must not corrupt the identity of the ones kept.
- Land after 102 so the bound guards the cheaper pipeline.

## Tests

- Feeding more than the cap distinct registrations through `drain_discovery`
  leaves `records.len()` at the cap and sets an observable "capped" status.
- Updates to an already-listed occurrence still apply when at the cap.
- Removing records below the cap re-opens capacity.

## Definition of Done

- `records` growth is bounded with a documented eviction/rejection policy and
  an observable capped state.
- The chosen layer (UI vs discovery) and default are justified in the code and,
  if it is an enduring decision, in an ADR.
- Completion gate green.

## Follow-up validation note (2026-07-17)

**Severity and scope must be upgraded before implementation.** A cap on
`App::records` protects only the final collection. It does not bound:

- the unbounded worker-to-UI `std::sync::mpsc` channel;
- `drain_discovery`, which runs until the channel is empty and can therefore
  starve input/rendering under a sustained producer;
- `LivenessTracker::services`;
- the full `probe_keys` clone; or
- the `join_all` resolver fan-out for every tracked service.

The upstream `mdns-sd-discovery` browser also uses an unbounded Tokio channel,
so delayed consumption accumulates memory rather than applying backpressure.
Treat ingress protection as P1 and do not block it on task 102. Use a bounded or
coalescing discovery inbox, a bounded amount of discovery work per frame,
limits for tracker state and probe concurrency, and observable counters/status
for coalesced or rejected events. A later final-map cap may still coordinate
with task 102.

Clarify the cap unit and rejection policy. The map key represents an occurrence
(`EntryId`), so a new occurrence of an already-known registration can consume
capacity. “Reject new registrations” is insufficient unless registration and
occurrence limits are separately defined.

Tests must include a producer that remains non-empty while the consumer runs,
prove that input/rendering receives time, and cover tracker/probe limits—not
only `records.len()`.

## Implementing ADR 0005 (2026-07-17)

The decision is recorded; this section is what to build, and it supersedes the
"Goal", "Suggested approach", and "Definition of Done" above. Task 108 is the
probe half of the same ADR — read both before starting either.

### What the ADR decided

Discovery is bounded and **fails closed**:

- A private discovery inbox module with a fixed-capacity, **non-blocking**
  producer, keeping the existing `DiscoverySession::poll` interface.
- A full inbox **cancels the adapter** and records an overload cause outside
  the queue. Once accepted events drain, the session becomes visibly `Failed`
  and the UI clears records it can no longer verify.
- ≤256 discovery events applied per frame.
- ≤4,096 occurrences retained in both the UI and the mDNS liveness tracker;
  existing-occurrence updates and capacity-releasing removals stay accepted.
- ≤32 concurrent probe resolvers, applied incrementally, browse polling
  continues during probes, cycles do not overlap.

Two rejected alternatives are load-bearing and must not be reintroduced:
blocking bounded sends (a full send can stop a browse loop observing
cancellation, breaking round 1's bounded-shutdown invariant), and drop-newest
(dropping a *removal* leaves a stale actionable entry on screen).

### Open questions the ADR leaves for implementation

1. **The inbox capacity is not stated in the ADR**, and it is the number that
   decides whether this is safety or a self-inflicted outage. Choose it, and
   justify it in the code against a realistic worst-case **legitimate** link
   (a conference or large-enterprise network with thousands of AirPlay/Cast
   devices), not merely against a hostile one. Record the reasoning where the
   constant lives. The other limits are given; this one you must defend.
2. **Overload must be distinguishable from a real browse failure** in what the
   user sees. The ADR requires the cause be recorded outside the queue — make
   the resulting `SessionState::Failed` cause say *overloaded*, with a distinct
   remedy, so it does not read as "the browse ended unexpectedly". Reuse the
   round-1 `DiscoveryFailure { kind, cause }` shape rather than inventing a
   parallel channel.
3. **Refresh re-trips on an honestly large network.** Fail-closed is correct
   for a flood; for a genuinely oversized link it makes kinjo unusable rather
   than degraded, because refresh restarts the same browse. The ADR defers
   coalescing "until ordering between occurrence and registration events has a
   proven state model", which is a fair deferral — but record in the completion
   note that graceful coalescing is the intended path for *legitimate* scale,
   so a later reader does not take fail-closed as the permanent answer.

### Cap unit

The follow-up note above still applies and the ADR does not overrule it: the
map key is an **occurrence** (`EntryId`), so a new occurrence of an
already-known registration consumes capacity. "Reject new registrations" is
insufficient unless registration and occurrence limits are defined separately.
Say which of the two the 4,096 bounds, and what happens to a new occurrence of
a known registration at the ceiling.

### Definition of Done (supersedes the one above)

- Bounded inbox, per-frame work budget, tracker cap, and occurrence cap, all
  per ADR 0005, with the inbox capacity justified in code.
- Overload surfaces as a distinct, actionable `Failed` cause; records are
  cleared, matching round 1's "unverifiable records must not stay" rule.
- Shutdown stays bounded and non-blocking — the round-1 cancellation invariant
  must be re-proved, not assumed, since this is exactly what a bounded channel
  threatens.
- Tests: a producer that stays non-empty while the consumer runs, proving
  input/rendering still gets time; tracker and probe limits; the occurrence-vs-
  registration ceiling behaviour.
- Completion gate green.

## Completion Record (2026-07-17)

- Added a 4,096-event non-blocking discovery inbox. Full delivery cancels the
  producer, retains the first overload cause outside the queue, and ends the
  session as a distinct failed/stopped state; the UI clears unverifiable rows.
- The UI drains at most 256 discovery events per tick and retains at most 4,096
  occurrences. Existing occurrence updates remain accepted at the ceiling;
  removals reopen capacity. The cap unit is explicitly `EntryId` occurrence,
  including a new occurrence of a known registration.
- `LivenessTracker` is capped at 4,096 and resolver concurrency at 32. Tests
  cover non-blocking overflow/cancellation, worker outcome, per-frame yield,
  tracker capacity, probe window, occurrence updates, and capacity reopening.
- The dependency's internal browser channel remains unbounded before Kinjo's
  Adapter receives it. ADR 0005 records that residual. Graceful coalescing is
  the intended future path for legitimately larger links once event ordering
  has a proven state model; fail-closed is not assumed to be the final UX.
- Completion gate passed.
