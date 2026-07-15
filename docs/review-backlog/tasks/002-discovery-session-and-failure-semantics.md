# 002: Own Discovery Lifecycle and Failure Semantics

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P0` |
| Workstream | Discovery |
| Depends on | 001, 016 |
| Likely conflicts | 003, 008, 015 |
| Owner | agent-a54fd9b (worktree `worktree-agent-a54fd9b7985060aab`) |

## Why This Matters

Real discovery failures currently start the fake adapter and publish plausible
LAN endpoints as ordinary actionable entries. A user can launch a configured
SSH/browser action against sample addresses without realizing that real discovery
failed. Separately, the discovery lifetime is split between an `events()` receiver,
backend object, and factory in `App`, and event-channel disconnection looks like
"no new events" forever.

Deepen the discovery-session module so lifecycle, event ownership, cancellation,
and failure provenance have one interface and one test surface.

Task 016 extracts the minimal, immediately shippable slice of this work: deleting
the three implicit `fake::spawn` fallback call sites and correcting the README
fallback promise. This task owns everything else — session ownership, disconnect
detection, typed persistent failure states, and refresh coherence. Once 016 is
done, revalidate the fallback-related evidence below against the current branch.

## Evidence

Re-validated against `dd872b7` (post-016, post-001) before implementing. Line
numbers below are updated to the current branch; stale claims are marked.

**Already landed by 016 — no longer true:**

- ~~`src/discovery/mdns.rs:131-138`: browse startup failure calls `fake::spawn`.~~
  STALE. `mdns.rs:166-174` now sends a `Status` and returns; no `fake::spawn`.
- ~~`src/discovery/zeroconf.rs:92-97`: total adapter failure calls `fake::spawn`.~~
  STALE. `zeroconf.rs:92-98` now sends a `Status` and returns; no `fake::spawn`.
- ~~`src/discovery/worker.rs:58-65`: runtime creation failure calls `fake::spawn`.~~
  STALE. `worker.rs:58-67` now sends a `Status` and returns; no `fake::spawn`.

`fake::spawn` retains exactly one caller, `FakeDiscovery::start` (`fake.rs:14`),
i.e. explicit fake mode only. The README fallback promise was corrected by 016.

**Still true — owned by this task:**

- `src/discovery/fake.rs:48-73`: samples use plausible `192.168.1.x` endpoints.
- `src/discovery/mod.rs:70-74`: `Discovery::events()` is callable once.
- `src/discovery/fake.rs:19-24` and `worker.rs:79-84`: the one-call invariant
  is implemented with `Option::take().expect(...)`.
- `src/ui/app.rs:61,82-84`: receiver, backend, and factory are separate fields.
- `src/lib.rs:40-45`: composition extracts and then reattaches discovery state.
- `src/ui/app.rs:204-238`: `try_recv()` does not distinguish Empty from Disconnected.

**New evidence found while re-validating:**

- `src/discovery/fake.rs:33-45`: the fake producer is a bare `thread::spawn` with
  no cancellation token and no join handle. Dropping `FakeDiscovery` does **not**
  stop it, so "dropping a session stops its producer" is currently false for the
  fake adapter. This is a real defect in scope for this task.
- `src/ui/render.rs:217-228`: the empty-list state unconditionally renders
  "listening for mDNS services on {domain}…" with a spinner. After a failure that
  clears records this actively lies about the session still listening.

## Required Outcome

- Sample entries are emitted only when `DiscoveryConfig.fake` is true.
- Real adapter startup/runtime failure emits a typed, persistent failure state and
  no sample Upserts.
- Unexpected real-adapter channel disconnection is visible to the UI as failed
  discovery; it clears now-untrustworthy real records, closes derived pickers,
  stops implying that it is listening, and offers refresh where available.
- Completion of the explicit fake adapter's finite sample stream is a normal
  `complete` state, not a discovery failure.
- One discovery session owns its receiver and shutdown behavior. Replacing or
  dropping it stops all producer work, including fake streaming.
- Refresh replaces the current session coherently and clears stale records only
  when the new session has been created according to the chosen session interface.
- Startup errors retain actionable cause text without relying on a transient status
  immediately overwritten by a second event.

## Implementation Constraints

- Keep the real adapter seam: mdns-sd, zeroconf, and explicit fake adapters vary.
- Do not add a public trait that merely wraps a receiver. Prefer a concrete deep
  session interface and internal adapter seams where behavior varies.
- Session Drop must be bounded and responsive to cancellation.
- Do not retain records from a failed refresh while labeling them as current.
- Update README language that currently promises implicit sample fallback.

## Suggested Implementation Sequence

1. Add tests proving real failure emits no Upsert and disconnect is observable.
2. Introduce a concrete discovery session/result type around events and shutdown.
3. Move worker/fake cancellation ownership behind that interface.
4. Update composition and refresh to own/replace the session as one value.
5. Render listening, failed, stopped, and explicit-fake states distinctly.
6. Remove implicit fallback calls and update product documentation.

## Non-Goals

- Retrying discovery automatically; refresh remains the recovery action.
- Changing service-type/domain validation; task 003 owns it.
- Redesigning entry occurrence identity; task 001 must land first.

## Acceptance Criteria / Definition of Done

- [x] Injected real-backend startup/runtime failure emits no sample entry.
- [x] Explicit fake discovery still streams the documented samples.
- [x] Dropping/replacing any session stops its producer and releases worker state.
- [x] A disconnected channel produces a persistent stopped/failed UI state.
- [x] Real terminal failure clears records/closes pickers, while finite fake
      completion preserves its samples and reports normal completion.
- [x] Refresh can recover from failure and does not mix old/new session events.
- [x] The one-call `events().expect(...)` panic interface is removed.
- [x] README/privacy/discovery documentation matches failure behavior.
- [x] Full validation passes for default and all-feature builds.

## Required Tests

- Worker/session: creation failure, cancellation, drop, receiver disconnect.
- Fake session: cancellation during its delayed stream.
- App: failed startup, real disconnect with empty/non-empty records, finite fake
  completion, picker closure, and refresh recovery.
- All-feature zeroconf failure path emits no fake Upsert.

## Validation

```sh
cargo test --locked discovery
cargo test --locked ui::app::tests::refresh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
  - New `src/discovery/session.rs`: `DiscoverySession`, the concrete deep
    interface owning the receiver, the producer, and the shutdown as one value.
    Interface is `poll` / `state` / `shutdown`. No public trait wrapping a
    receiver was added; the adapter seam stays where behaviour genuinely varies,
    at the browse loop inside the module.
  - Typed failure provenance: `SessionState::{Listening, Complete, Failed}` and
    `DiscoveryFailure { kind: FailureKind::{Startup, Stopped}, cause }`. The
    cause travels with the value, so it survives later events instead of being
    a status line the next event overwrites.
  - `SessionPoll::{Event, Idle, Ended}` makes Empty and Disconnected distinct.
    `Ended` fires exactly once, on the transition, so the app reacts to the
    ending rather than re-applying it every tick; the verdict persists in
    `state()` and never decays back to "listening".
  - `worker.rs`: browse loops now return a `BrowseOutcome` (`Startup(cause)` /
    `Stopped` / `Complete` / `Cancelled`) — a dropped channel alone cannot tell
    those endings apart. The outcome is published *before* the channel
    disconnects (via a keepalive sender clone), so observing `Disconnected`
    guarantees the ending that explains it is already readable — no race.
  - `DiscoveryWorker::spawn` now returns `(worker, receiver)`, which is what
    removes the take-once `events().expect(...)` panic interface; the `Discovery`
    trait is deleted entirely.
  - `fake.rs`: the sample stream moved onto the same worker scaffold. It is now
    cancellable mid-stream (`tokio::select!` on the token during its pauses) and
    reports `Complete` when its finite stream ends.
  - `app.rs` (kept minimal, confined to lifecycle): `discovery_rx` + `discovery`
    collapse into one `session` field; `with_discovery` becomes
    `with_discovery_factory`; new `apply_session_end` clears records and closes
    derived pickers on real failure, and preserves samples on fake completion.
  - `refresh_services`: stops the old producer, creates the replacement, and only
    then drops the old session and clears records. Old/new event mixing is now
    structurally impossible — the old session takes its receiver with it.
  - `lib.rs`: composition no longer extracts and reattaches discovery state; it
    passes one session value to `App::new`.
  - `render.rs` (surgical, discovery-state only): the empty list no longer renders
    "listening for mDNS services…" with a spinner after a failure; it renders the
    failure headline and cause.

- **Fixed beyond the original evidence:** the fake adapter's producer was a bare
  `thread::spawn` with no cancellation and no join handle, so dropping a fake
  backend did not stop it. It is now cancelled and joined like any other.

- **Tests added/updated:** 47 net new tests (163 baseline → 210 default).
  - `worker.rs` (7): startup failure emits no upsert, outcome published before
    disconnect, config handed to the loop, bounded cancel+join, idempotent
    shutdown, drop stops the producer, loop that never runs still publishes.
  - `session.rs` (8): Empty vs Disconnected, ending reported once and persisting,
    buffered events delivered before the ending, fake completes normally after
    its finite stream, dropping a fake session cancels its delayed stream,
    idempotent shutdown, cancelled ≠ complete, failure carries its cause.
  - `fake.rs` (3): status-then-filtered-records completes, cancellation mid-stream,
    receiver-gone stops the loop.
  - `zeroconf.rs` (2, all-features): no-started-workers is a `Startup` failure
    with the cause and **no events at all**; the whole session reports a typed
    failure with **no sample Upsert**.
  - `app.rs` (8): real disconnect clears records + persistent failure, disconnect
    with no records still fails, startup cause survives later drains, disconnect
    closes a derived picker, finite fake completion keeps samples and reports
    completion, refresh recovers from failure, replaced-session events never
    reach the new list, a failed refresh does not retain old records.
  - `render.rs` (2): a live session says "listening"; a failed one reports the
    failure instead (the paired positive test keeps the negative non-vacuous).

- **Documentation updated:**
  - `README.md`: discovery paragraph now describes the real behaviour — no
    fallback, failure clears the list and explains why (with the edge-triggered
    mDNS reasoning), no automatic retry, refresh is the recovery action, and fake
    samples are a finite stream that completes normally. Architecture and Privacy
    sections corrected: the `Discovery` trait no longer exists.
  - `CONTRIBUTING.md`: project layout no longer references the removed trait.
  - 016 had already corrected the implicit-fallback promise; verified rather than
    re-fixed.

- **Validation evidence:**
  - `cargo fmt -- --check` — pass.
  - `cargo clippy --locked --all-targets -- -D warnings` — pass.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings` — pass.
  - `cargo test --locked --all-targets` — 210 passed, 0 failed.
  - `cargo test --locked --all-targets --all-features` — 216 passed, 0 failed.
  - `cargo test --locked --all-features zeroconf` — 7 passed, including both
    failure-path tests (the all-features build exercises the zeroconf failure
    path and asserts no fake Upsert).

- **Follow-ups:**
  - `DiscoverySession::{detached, inert}` are `#[cfg(test)]` crate-internal
    constructors. They exist because the required App-level tests must drive
    exact event sequences and endings; CONTEXT permits private internal seams.
    If a real second production producer ever appears, revisit whether they
    should merge with it.
  - The top-bar spinner (`render.rs`) still animates unconditionally. Harmless
    (it is not a "listening" claim, and the list body now tells the truth), but a
    mode-aware top bar belongs with task 010 rather than here.
  - `FailureKind::Startup` is only reachable from a worker-backed session, since
    it is the producer's own account of its ending. A detached session ends as
    `Stopped`, which is the honest answer for a producer that cannot explain
    itself. Noted in case a future adapter wants finer provenance.
  - Not in scope, unchanged: automatic retry (refresh remains the recovery
    action), service-type/domain validation (task 003), occurrence identity
    (task 001, already landed).
