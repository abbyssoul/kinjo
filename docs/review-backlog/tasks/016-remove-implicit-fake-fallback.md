# 016: Remove the Implicit Sample-Record Fallback on Real Discovery Failure

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P0` |
| Workstream | Discovery / Safety |
| Depends on | — |
| Likely conflicts | 002 |
| Owner | Ivan Ryabov |

## Why This Matters

This is the minimal, immediately shippable slice of task 002. When a real
discovery backend fails to start, all three failure paths silently call
`fake::spawn` and publish plausible `192.168.1.x` sample endpoints as ordinary
actionable entries. A user whose real discovery failed can select a configured
SSH/browser action against a fabricated address — one that may belong to a real,
unrelated host on their LAN.

Task 002 owns the full discovery-session redesign (lifecycle ownership,
disconnect detection, refresh coherence) and is blocked on task 001. Removing
the fallback itself requires none of that: it is three call sites and a status
message. Per the trust model in `CONTEXT.md`, real discovery failure must never
create actionable sample devices; that invariant should not wait behind an
identity refactor.

## Evidence

- `src/discovery/mdns.rs:131-139`: browse startup failure sends a Status and
  calls `fake::spawn`.
- `src/discovery/zeroconf.rs:92-98`: when no browser worker starts, sends a
  Status and calls `fake::spawn`.
- `src/discovery/worker.rs:58-66`: runtime creation failure sends a Status and
  calls `fake::spawn`.
- `src/discovery/fake.rs:27-28`: `spawn`'s doc comment describes the sharing
  with the mDNS backend fallback.
- `src/discovery/fake.rs:49-78`: samples use plausible `192.168.1.x` endpoints
  with real service types and ports.
- `README.md:61-62`: "If mDNS discovery is unavailable, it falls back to sample
  records so the UI remains usable."

Line numbers are starting points. Revalidate them against the current branch.

## Required Outcome

- Sample entries are emitted only when `DiscoveryConfig.fake` is true (the user
  passed `--fake-discovery`).
- Real adapter startup/runtime failure emits a Status event naming the failed
  backend and cause, and emits no Upsert. The UI shows an empty list with that
  status; it does not fabricate devices.
- The failure Status text no longer claims "using sample records"; it names the
  error and points at `--fake-discovery` as the explicit demo/testing option and
  refresh as the retry action.
- Explicit fake mode (`--fake-discovery`) continues to stream the documented
  samples unchanged.
- README no longer promises implicit sample fallback; it documents that
  discovery failure leaves the list empty with an error status and that
  `--fake-discovery` is the explicit sample mode.

## Implementation Constraints

- Keep the change minimal: do not restructure session/receiver ownership,
  disconnect detection, or refresh semantics — task 002 owns those.
- Keep `fake::spawn` available for the explicit fake backend; only remove the
  implicit fallback call sites (and update `spawn`'s doc comment).
- Keep both default and `zeroconf` feature builds working.
- A typed persistent failure state is task 002's scope; a Status string is
  sufficient here as long as no sample Upserts occur.

## Suggested Implementation Sequence

1. Add regression tests asserting that injected browse/runtime/worker startup
   failure produces Status events only and never an Upsert.
2. Delete the three `fake::spawn` fallback calls and update the Status text.
3. Update `fake::spawn`'s doc comment and README's fallback sentence.
4. Confirm explicit `--fake-discovery` behavior is unchanged.

## Non-Goals

- Discovery session ownership, cancellation, or the one-call `events()` panic
  interface (task 002).
- Detecting event-channel disconnection or clearing records on terminal failure
  (task 002).
- Typed discovery failure states and their rendering (task 002).
- Retrying discovery automatically.

## Acceptance Criteria / Definition of Done

- [x] Injected mdns-sd browse failure emits no sample Upsert.
- [x] Injected worker runtime-creation failure emits no sample Upsert.
- [x] Zeroconf total startup failure emits no sample Upsert (all-features build).
- [x] Failure Status names the cause and the explicit fake/refresh remedies.
- [x] Explicit `--fake-discovery` still streams the documented samples.
- [x] README fallback language matches the new behavior.
- [x] Full validation passes for default and all-feature builds.

## Required Tests

- `discovery::worker`: runtime/browse startup failure → Status only, no Upsert.
- `discovery::zeroconf` (all-features): zero started workers → Status only,
  no Upsert.
- `discovery::fake`: explicit fake mode still emits the documented sample set.

## Validation

```sh
cargo test --locked discovery
cargo test --locked --all-features discovery
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:** Removed the three implicit `fake::spawn` fallback calls
  (`src/discovery/mdns.rs` browse-startup failure, `src/discovery/zeroconf.rs`
  zero-started-workers, `src/discovery/worker.rs` runtime-creation failure).
  Each now sends a `DiscoveryEvent::Status` naming the cause and pointing at
  `--fake-discovery` and refresh, then returns without emitting any `Upsert`.
  `fake::spawn` itself is untouched and still backs `FakeDiscovery::start` for
  explicit `--fake-discovery`. Updated `fake::spawn`'s and the two backend
  `start()` doc comments to drop the "falls back to fake" language. Zeroconf's
  `browse_loop` no longer uses its `domain`/`service_type_filter` params after
  the fallback call was removed; they're kept (underscore-prefixed) since the
  closure signature is shared with `DiscoveryWorker::spawn`.
- **Tests added/updated:** `discovery::worker::tests::browse_startup_failure_relays_status_only_no_upsert`
  (new — exercises `DiscoveryWorker::spawn` with a closure that simulates a
  browse/runtime startup failure and asserts only a `Status` event is relayed).
  `discovery::zeroconf::tests::no_started_workers_emits_status_only_no_upsert`
  (new, all-features only — calls `browse_loop` with an empty service-type list
  to deterministically hit the zero-workers branch). Existing
  `discovery::fake::tests::spawn_streams_status_then_filtered_records` already
  covers explicit fake mode and was left unchanged, confirming that behavior is
  unaffected.
- **Documentation updated:** `README.md` — replaced the "falls back to sample
  records" sentence with the empty-list/explicit-`--fake-discovery`/refresh
  behavior.
- **Validation evidence:** All green locally:
  `cargo fmt -- --check`; `cargo clippy --locked --all-targets --all-features -- -D warnings`;
  `cargo test --locked discovery` (36 passed); `cargo test --locked --all-features discovery`
  (40 passed); `cargo test --locked --all-targets` (164 passed, up from the
  163-test baseline); `cargo test --locked --all-targets --all-features` (168
  passed, up from the 166-test baseline).
- **Follow-ups:** None beyond task 002's already-scoped session/lifecycle
  redesign (typed failure state, disconnect detection, refresh coherence).
