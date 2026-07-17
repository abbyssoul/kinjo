# ADR 0005: Discovery overload fails closed

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-07-17 |
| Decider | Ivan Ryabov (project owner) |
| Context | [task 107](../review-backlog-2/tasks/107-bound-record-growth.md), [task 108](../review-backlog-2/tasks/108-probe-starvation.md) |

## Context

Discovery previously crossed an unbounded `std::sync::mpsc` channel, the UI
drained until empty, the mDNS liveness tracker had no ceiling, and a probe cycle
started one native resolver for every tracked occurrence. A hostile or merely
very busy link could therefore consume unbounded memory, keep the UI from
returning to input/drawing, and fan out native work without limit.

Lossy event coalescing is not initially safe. Occurrence upserts/removals and
registration-wide removals have ordering semantics; an incorrect merge can
leave a stale actionable entry on screen. Blocking an Adapter on a bounded
channel is also unsafe because a full send can stop its browse loop observing
cancellation, making session shutdown wait indefinitely.

## Decision

Discovery is bounded and fails closed:

- A private discovery inbox Module has a fixed-capacity, non-blocking producer.
  It keeps the existing `DiscoverySession::poll` Interface.
- A full inbox cancels the Adapter and records an overload cause outside the
  queue. Once the accepted events drain, the session becomes visibly failed and
  the UI clears records it can no longer verify.
- The UI applies at most 256 discovery events per frame.
- The UI and mDNS liveness tracker each retain at most 4,096 occurrences.
  Existing occurrence updates and capacity-releasing removals remain accepted.
- mDNS runs at most 32 probe resolvers concurrently, applies results
  incrementally, and continues polling browse events while probes are active.
  Probe cycles do not overlap.

## Consequences

Positive:

- Kinjo-owned event memory, retained occurrences, per-frame discovery work, and
  native resolver concurrency are all bounded.
- Overload never silently presents an incomplete list as the complete network.
- Shutdown stays non-blocking and bounded.
- Probe traffic no longer makes the browse loop deaf for up to a resolver
  timeout.

Negative:

- A sufficiently large burst ends the discovery session and requires refresh;
  this is deliberate fail-closed behavior rather than graceful coalescing.
- `mdns-sd-discovery` currently has its own upstream unbounded Tokio channel.
  Kinjo consumes it concurrently and stops promptly on local overload, but an
  absolute end-to-end bound requires an upstream change or patched dependency.
- The chosen limits are policy constants. Operational evidence may justify
  changing them later, but they must remain coordinated and regression-tested.

## Rejected alternatives

- **Blocking bounded sends:** rejected because they can deadlock cancellation
  and join.
- **Drop newest:** rejected because dropping a removal leaves stale actionable
  state.
- **Ad-hoc coalescing:** rejected until ordering between occurrence and
  registration events has a proven state model.
