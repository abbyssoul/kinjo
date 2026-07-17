# Task 108 — Probe cycles must not starve browse events

- **Priority**: P2 (latency bug, mdns-sd backend) — but schedule with 107,
  which shares its ADR
- **Status**: done
- **Depends on**: none
- **Likely conflicts**: 107 (same ADR, same module; design and land together)

> **Reconciliation (2026-07-17).**
> [ADR 0005](../../adr/0005-discovery-overload-fails-closed.md) settles this
> task's design question: mDNS "runs at most 32 probe resolvers concurrently,
> applies results incrementally, and continues polling browse events while
> probes are active. Probe cycles do not overlap." That is the multiplexing
> this task asked for, plus the concurrency bound the follow-up note asked for.
> The problem statement below stands; the ADR decides the shape. Treat 107 and
> 108 as one piece of work in `src/discovery/` — the ADR's probe-concurrency
> limit and its ingress bounds are the same defence.

## Problem

The mdns-sd browse loop (`src/discovery/mdns.rs:205-241`) is a `tokio::select!`
over three arms: shutdown, the probe timer, and `browser.recv()`. When the
probe timer fires, the handler enters a **nested** `select!`
(`mdns.rs:215-222`):

```rust
_ = probe_timer.tick() => {
    let keys = tracker.probe_keys();
    if keys.is_empty() { continue; }
    tokio::select! {
        _ = shutdown.cancelled() => break BrowseOutcome::Cancelled,
        results = probe_services(keys) => { ... }
    }
}
```

While `probe_services(keys)` is awaited, the outer `select!` is not running, so
`browser.recv()` is **not polled**. Browse events — a new service appearing, a
service being removed — therefore queue in the mDNS layer until the probe cycle
finishes. A probe cycle is bounded by `PROBE_TIMEOUT` (5s, `mdns.rs:25`) and
runs every `PROBE_INTERVAL` (30s, `mdns.rs:23`) once any service is being
tracked. So once discovery is populated, up to 5 seconds out of every 30 the
loop is deaf to the network.

The channel does not lose events (the underlying browser buffers), so this is a
latency bug, not a correctness one: a service that appears or vanishes during a
probe cycle shows up in the UI a few seconds late. On a busy network that is a
visibly laggy list.

## Goal

Keep processing browse events while a probe cycle is in flight, so browse
latency is unaffected by probing.

## Suggested approach (agent to validate)

- Fold the probe into the outer `select!` rather than blocking inside a timer
  arm. Options:
  - Spawn the probe cycle as a task/future whose completion is one arm of the
    single outer `select!`, alongside `browser.recv()` and shutdown, so browse
    events are serviced concurrently with the probe. The probe futures are not
    `Send` on Windows (`mdns.rs:264-286` documents this — hence the
    current-thread runtime), so use a `LocalSet`/`join`-style local future
    rather than `tokio::spawn`, keeping the non-`Send` constraint satisfied.
  - Or drive the probe as a `FuturesUnordered`/pinned future polled by the same
    `select!` loop, so one loop multiplexes browse + probe + shutdown.
- Preserve every current guarantee: probe cadence (`PROBE_INTERVAL`), per-probe
  timeout (`PROBE_TIMEOUT`), the missed-tick `Delay` behaviour
  (`mdns.rs:200-201`), interface-confined resolves (`mdns.rs:275-281`), and
  bounded shutdown (the loop must still stop promptly on cancellation, not wait
  out a probe). Do not let two probe cycles overlap — if a cycle is still
  running when the timer fires again, skip or coalesce, matching today's
  effectively-serial cadence.

## Constraints

- The resolve futures must remain non-`Send`-safe (current-thread runtime); do
  not require `Send`.
- Shutdown must stay bounded (round-1 invariant: every browse loop selects on
  the token).
- No change to what events are emitted or the liveness state machine
  (`LivenessTracker`) — only *when* browse events are serviced relative to a
  probe.

## Tests

- The existing mdns tests are unit tests over the tracker and the event
  translation; they do not exercise the select loop timing. Add a test (or a
  documented manual check) that a browse event delivered while a probe future
  is pending is still emitted promptly — e.g. drive `browse_loop` with a stub
  browser that yields an event during a long-running probe and assert the event
  comes out without waiting for the probe. If a deterministic test is not
  feasible against the real crate types, document the manual verification in the
  completion record and keep the change minimal and reviewed.

## Definition of Done

- Browse events are serviced concurrently with probe cycles; no 5s deafness.
- Probe cadence, timeout, interface confinement, and bounded shutdown
  unchanged.
- Completion gate green (note: the mdns path is only exercised with a live
  network; state the verification method used).

## Follow-up validation note (2026-07-17)

**Finding confirmed, with a resource-use consequence.** Browse events normally
are not lost during the nested probe wait because the dependency's channel is
unbounded; they accumulate. The result is both latency and a possible memory
burst when a busy network produces events during probing.

Coordinate this task with the bounded ingress, tracker, and probe-concurrency
work added to task 107. Multiplexing browse and probe completion fixes the deaf
period but does not by itself bound the number of simultaneous resolvers or the
queued event volume. Conversely, a bounded inbox must preserve removals and the
latest useful state when it coalesces events.

Prefer an internal deterministic seam for the timing test. Do not add a new
public discovery adapter interface solely to make this select loop testable
unless the interface has independent production leverage.

## Completion Record (2026-07-17)

- Removed the nested probe wait. One outer `tokio::select!` now continuously
  polls shutdown, browse events, the probe timer, and incremental probe
  completions.
- A `VecDeque` plus local `FuturesUnordered` starts at most 32 non-`Send`
  resolvers, applies each result immediately, and refills the window. Timer
  guards prevent overlapping cycles; timeout, interface confinement, and
  delayed missed-tick behavior remain unchanged.
- The internal probe-window seam has deterministic tests. The real mDNS
  Adapter was also smoke-run successfully; structural verification confirms
  `browser.recv()` remains an active select arm throughout a probe cycle.
- Completion gate passed.
