# 019: Make Fake Discovery a Selectable, Feature-Gated Backend

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P2` |
| Workstream | Discovery / CLI |
| Depends on | 017 |
| Likely conflicts | — |
| Owner | Unclaimed |

## Why This Matters

Fake discovery *is* a discovery backend. It implements the same seam, produces
the same `Entry` values, and answers the same question as mdns-sd and zeroconf —
it simply answers from a fixed sample set rather than the network. The CLI does
not say so. Backends are chosen with `--backend`, but this one is chosen with a
separate boolean `--fake-discovery`, so the interface has two ways to pick a
backend and neither mentions the other.

The mismatch is not cosmetic. It costs twice:

- **The interface lies about the model.** `--backend` reads as an exhaustive
  choice and is not one. `--backend mdns-sd --fake-discovery` is accepted today,
  and one of the two silently wins. A user cannot tell from `--help` that fake is
  a backend at all, and this task was proposed precisely because a reader
  reached for `--backend fake` and found it did not exist.
- **Sample-record generation ships to production.** It is compiled into every
  release binary. The zeroconf backend is already feature-gated for a comparable
  reason, so the pattern exists; fake does not use it. CONTEXT's discovery
  invariants exist to keep sample records away from real failures (task 016 fixed
  a real instance of exactly that). A backend that cannot be compiled into a
  production build cannot violate them there at all — defense in depth under a
  rule the codebase already treats as load-bearing.

This is a maintainability and interface-honesty task, not a correctness fix. Task
016 closed the actual hole; this removes the shape that allowed it.

## Evidence

- `src/ui/cli.rs:12`: `pub fake_discovery: bool` — a flag, parallel to `backend`.
- `src/ui/cli.rs:100`: `fake_discovery: matches.get_flag("fake-discovery")`.
- `src/ui/cli.rs:29`: `fake: self.fake_discovery` projects it into the discovery
  request beside the chosen backend, so both reach validation independently.
- `src/discovery/options.rs:31-41`: `DiscoveryBackend { MdnsSd, Zeroconf }`, where
  `Zeroconf` is `#[cfg(feature = "zeroconf")]` — the exact pattern this task
  applies to fake. `MdnsSd` is `#[default]`.
- `src/discovery/options.rs:43-45`: `DiscoveryBackend::name` — backends already
  own the name they are selected by and reported as.
- `Cargo.toml:25-30`: `[features] default = []`, with `zeroconf` opt-in and
  documented as a backend selector.
- `src/discovery/fake.rs`: `fake_records` and `sample_loop`, the sample backend.
- `src/ui/cli.rs:423`: `fake_discovery_accepts_a_domain_no_backend_supports` —
  task 003's rule that fake bypasses real-backend capability checks, which must
  survive as a property of the fake *backend*.
- CONTEXT, *Discovery*: "Explicit fake mode continues to stream sample records and
  remains suitable for development and smoke tests." and "Real discovery failure
  must never create actionable sample devices."

## Required Outcome

- Fake is selected as a backend: `--backend fake`. `--backend` becomes the single,
  exhaustive way to choose one.
- The fake backend is behind a Cargo feature, following the `zeroconf` pattern.
  Decide and record whether it is in `default` — the honest trade is developer
  convenience against not shipping sample generation to users. If it is not
  default, `CONTRIBUTING.md` and the README must make the build obvious, and the
  error for selecting an uncompiled backend must name the feature to enable.
- Selecting a backend that was not compiled in fails with actionable text, exactly
  as an unsupported backend does today.
- Every existing fake behavior is preserved as a property of the backend:
  arbitrary configured domains are accepted (no real capability is exercised), the
  sample stream stays finite and completes normally rather than failing, and the
  records still exercise the UI's real behavior (task 017).
- Real discovery failure still never yields sample records. Removing the flag must
  not reintroduce any implicit path to fake (task 016).
- `--fake-discovery` is retired. This is a breaking CLI change: decide whether to
  drop it outright or accept it as a deprecated alias for one release, and record
  the decision. Do not leave both as live, independent ways to choose a backend —
  that is the defect.
- `scripts/drive-tui.sh` and its documentation move to the new invocation. Its
  defaults are the project's documented way to exercise the UI.

## Implementation Constraints

- Preserve the dependency direction: discovery must not depend on the UI. The
  backend enum stays in `src/discovery/`; the CLI only selects a value.
- Validation stays at the discovery session/start seam that task 003 established.
  Do not reintroduce per-adapter capability checks.
- Keep both default and `zeroconf` feature builds working, and now also the
  with/without-fake combinations. Check the feature matrix actually compiles;
  `cargo test --all-features` alone will not catch a broken default build.
- Tests currently reaching for fake discovery must not silently vanish from a
  build that excludes it. Gate them with the feature deliberately rather than
  letting them disappear.
- Update README, `CONTRIBUTING.md`, and any `--fake-discovery` example.
- If dropping the flag outright, say so in the release notes; a user's muscle
  memory and scripts are an interface too.

## Suggested Implementation Sequence

1. Add `Fake` to `DiscoveryBackend` behind a feature, mirroring `Zeroconf`.
2. Route selection through `--backend`, and make an uncompiled backend a clear
   startup error naming the feature.
3. Move fake's domain/capability properties onto the backend, keeping task 003's
   tests meaningful.
4. Remove `--fake-discovery` (or alias it, per the recorded decision) and update
   every caller: tests, `scripts/drive-tui.sh`, README, CONTRIBUTING, CI.
5. Verify the feature matrix builds, and drive the app both ways.

## Non-Goals

- Changing what the sample records contain; task 017 owns the sample set.
- Adding new backends, or a plugin mechanism for them.
- Revisiting task 016's fallback removal, or task 002's failure semantics.
- Making the sample set configurable at runtime.

## Acceptance Criteria / Definition of Done

- [ ] `--backend fake` selects sample discovery; `--backend` is the only way to
      choose a backend.
- [ ] The fake backend is feature-gated, and its default-ness is a recorded
      decision rather than an accident.
- [ ] Selecting an uncompiled backend fails with text naming the feature.
- [ ] Fake still accepts any domain, still completes normally, and still cannot be
      reached by a real backend's failure.
- [ ] The feature matrix builds: default, `zeroconf`, fake on and off.
- [ ] `scripts/drive-tui.sh`, README, and CONTRIBUTING use the new invocation.
- [ ] The `--fake-discovery` removal or deprecation is documented.
- [ ] Full validation passes.

## Required Tests

- `ui::cli`: `--backend fake` parses; an uncompiled backend errors naming the
  feature; `--fake-discovery` behaves per the recorded decision.
- Discovery options: fake accepts a domain no real backend supports (retarget the
  existing test at `src/ui/cli.rs:423`).
- `discovery::fake` and `discovery::session`: the sample stream still completes
  normally rather than failing.
- `ui::app`: the sample set still reaches the UI (task 017's coverage) under the
  new selection.

## Validation

```sh
cargo test --locked ui::cli
cargo test --locked discovery
cargo run --locked -- --backend fake --config-dir actions   # or the built feature
scripts/drive-tui.sh run 'Tab Tab Down Down Down Enter'
cargo build --locked                       # default feature set
cargo build --locked --all-features
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
