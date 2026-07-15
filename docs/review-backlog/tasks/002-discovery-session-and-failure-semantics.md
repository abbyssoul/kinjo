# 002: Own Discovery Lifecycle and Failure Semantics

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P0` |
| Workstream | Discovery |
| Depends on | 001, 016 |
| Likely conflicts | 003, 008, 015 |
| Owner | Unclaimed |

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

- `src/discovery/mdns.rs:131-138`: browse startup failure calls `fake::spawn`.
- `src/discovery/zeroconf.rs:92-97`: total adapter failure calls `fake::spawn`.
- `src/discovery/worker.rs:58-65`: runtime creation failure calls `fake::spawn`.
- `src/discovery/fake.rs:49-78`: samples use plausible `192.168.1.x` endpoints.
- `src/discovery/mod.rs:58-62`: `Discovery::events()` is callable once.
- `src/discovery/fake.rs:19-24` and `worker.rs:79-84`: the one-call invariant
  is implemented with `Option::take().expect(...)`.
- `src/ui/app.rs:61,80-84`: receiver, backend, and factory are separate fields.
- `src/lib.rs:40-45`: composition extracts and then reattaches discovery state.
- `src/ui/app.rs:204-229`: `try_recv()` does not distinguish Empty from Disconnected.

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

- [ ] Injected real-backend startup/runtime failure emits no sample entry.
- [ ] Explicit fake discovery still streams the documented samples.
- [ ] Dropping/replacing any session stops its producer and releases worker state.
- [ ] A disconnected channel produces a persistent stopped/failed UI state.
- [ ] Real terminal failure clears records/closes pickers, while finite fake
      completion preserves its samples and reports normal completion.
- [ ] Refresh can recover from failure and does not mix old/new session events.
- [ ] The one-call `events().expect(...)` panic interface is removed.
- [ ] README/privacy/discovery documentation matches failure behavior.
- [ ] Full validation passes for default and all-feature builds.

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
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
