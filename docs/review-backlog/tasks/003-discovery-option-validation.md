# 003: Validate Discovery Options and Adapter Capabilities

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P0` |
| Workstream | Discovery / CLI |
| Depends on | 002 |
| Likely conflicts | 009, 015 |
| Owner | agent (branch `worktree-agent-ab199244b0258f42d`) |

## Why This Matters

With the zeroconf adapter, a malformed explicit service type silently becomes
"browse every default type," broadening network observation instead of limiting
it. The same adapter accepts a custom domain from the common discovery interface
but cannot pass it to its dependency, so it silently browses the default domain.
CLI options must either be honored exactly or rejected before work starts.

## Evidence

- `src/discovery/mod.rs:52-55`: `DiscoveryConfig` promises a browse domain and
  optional single-type limit.
- `src/ui/cli.rs:26-32`: CLI values are projected without semantic validation.
- `src/discovery/zeroconf.rs:180-190`: invalid `Some(filter)` falls through to
  `DEFAULT_SERVICE_TYPES`.
- `src/discovery/zeroconf.rs:210-214`: a test enshrines the broad fallback.
- `src/discovery/zeroconf.rs:61-77`: `domain` is accepted but not used when
  constructing successful browsers.
- Upstream `zeroconf`'s browser interface has no domain setter, so this adapter
  cannot currently honor non-default domains.

## Required Outcome

- Validate and canonicalize an explicit DNS-SD service type once before starting
  an adapter. Accepted syntax is `_<name>._tcp` or `_<name>._udp`; `<name>` is
  1–15 ASCII letters/digits/internal hyphens, begins and ends alphanumeric, has
  no consecutive hyphens, and contains at least one ASCII letter. Malformed values
  produce an actionable CLI/startup error.
- Valid TCP and UDP service types remain accepted and reach adapters as a typed or
  otherwise validated value.
- `None` alone means "browse all supported/default types."
- Canonicalize empty, case-insensitive `local`, and `local.` to the default
  `local` domain. Reject zeroconf with any other domain before spawning.
- The mdns-sd adapter continues to receive supported custom domains.
- Explicit fake mode accepts arbitrary configured domains because no real adapter
  capability is exercised.
- `list-commands` does not validate unused discovery options or start discovery.

## Implementation Constraints

- Keep feature-gated CLI behavior clear when zeroconf is not compiled.
- Do not silently normalize an invalid value into a broader browse.
- Validation belongs at the discovery session/start seam shared by CLI startup,
  refresh, and library callers, not duplicated across adapter loops.
- Error text must name the invalid option and the supported remedy.

## Suggested Implementation Sequence

1. Add start/config tests for malformed/valid service types and domain/backend pairs.
2. Introduce validated discovery inputs without leaking dependency-specific types
   through unrelated modules unless that materially simplifies the interface.
3. Delete zeroconf's invalid-filter fallback and replace its test.
4. Add the zeroconf capability check before worker creation.
5. Document the zeroconf domain limitation.

## Non-Goals

- Replacing the zeroconf dependency to add domain support.
- Changing the curated default service-type list.
- Redesigning discovery session ownership; task 002 owns lifecycle.

## Acceptance Criteria / Definition of Done

- [x] A malformed explicit service type fails without starting broad discovery.
- [x] Valid `_name._tcp` and `_name._udp` inputs browse exactly one type.
- [x] Zeroconf with a custom domain fails before spawning with actionable text.
- [x] mdns-sd custom-domain behavior remains covered.
- [x] CLI help/README explain backend capability accurately.
- [x] Default and all-feature validation pass.

## Required Tests

- `ui::cli`: valid/invalid type syntax; backend/domain combinations.
- Discovery configuration: canonical values and error variants.
- Refresh and direct library start use the same validation; explicit fake and
  `list-commands` bypass unused real-backend capability checks.
- `discovery::zeroconf`: explicit type yields one browser; `None` yields defaults;
  invalid explicit values cannot reach adapter resolution.

## Validation

```sh
cargo test --locked ui::cli
cargo test --locked --all-features discovery::zeroconf
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
  - New `src/discovery/options.rs`, which owns the discovery inputs and their
    validation. `DiscoveryConfig` (moved here, unchanged in shape) is now
    explicitly a *request*; `DiscoveryConfig::validate` turns it into
    `DiscoveryOptions`, the validated/canonical value that `discovery::start`
    now requires. `DiscoveryBackend` moved here too, since it is an option; both
    are re-exported from `discovery`, so no caller outside sees the move.
  - The guarantee is structural rather than remembered: there is no way to reach
    an adapter except through `validate`, so no browse loop can be handed a
    value it would have to reinterpret. This is what let the adapters *delete*
    their fallbacks rather than have each of them re-check. Validation is at the
    start seam, shared by CLI startup, refresh, and library callers, and it is
    not duplicated in any adapter loop.
  - `ServiceTypeFilter`: a validated `_<name>._tcp` / `_<name>._udp`, per the
    task grammar (1–15 ASCII letters/digits/internal hyphens, begins and ends
    alphanumeric, no consecutive hyphens, at least one letter). DNS-SD types are
    case-insensitive (RFC 6763 §7), so it canonicalizes to lower case; it
    carries `name()`/`protocol()` so adapters need no dependency-specific
    parsing. `None` still means "browse all supported/default types".
  - Domain canonicalization: empty, `local`, and `local.` (case-insensitive) all
    become the default `local`; any other value passes through untouched, so
    canonicalization can never silently redirect a browse.
  - Zeroconf capability check in `validate`, i.e. before any worker is spawned:
    `DiscoveryBackend::supports_custom_domain` states the real limitation
    (upstream `zeroconf`'s browser exposes no domain setter). mdns-sd continues
    to accept custom domains; explicit fake mode skips the *capability* check
    only — its service type is still validated, since that is not a backend
    limitation.
  - **Deleted zeroconf's invalid-filter fallback to `DEFAULT_SERVICE_TYPES`.**
    `resolve_service_types` now takes an already-validated type and builds the
    browser with `ServiceType::new(name, protocol)`. Upstream `ServiceType::from_str`
    was the root of the bug: it only rejects empty text, `.` and `,`, and
    requires exactly two dot-separated parts, so `--service-type "not a service
    type"` failed to parse and fell through to sweeping all 20 default types.
  - `DiscoveryOptionError` (`ServiceType`/`UnsupportedDomain`) names the value
    and the remedy; `Cli::discovery_options` dresses it as a clap usage error
    naming the flag, so it reads like every other usage error (exit 2) rather
    than an eyre report. It is deliberately *not* called from `parse_from`, so
    `list-commands` never validates options it will not use.
  - `DiscoveryFactory` became `Box<dyn Fn() -> DiscoverySession>`: the
    composition root builds it around the same validated options the startup
    session used, so refresh repeats that browse exactly and cannot re-derive a
    different or unchecked one. This also removed `DiscoveryConfig` from
    `src/ui/app.rs` entirely.
- **Tests added/updated:**
  - `discovery::options` (13): valid TCP/UDP types; lower-case canonicalization;
    18 malformed values each asserting the rule they broke; the 15-character
    boundary; `None` means all types; every default-domain spelling; custom
    domains passed through intact; mdns-sd accepts custom domains; zeroconf
    rejects them with actionable text; zeroconf accepts every spelling of the
    default; fake accepts a domain no real backend could; fake still validates
    the service type.
  - `ui::cli` (13, +10): valid/canonicalized types reach discovery validated;
    malformed `--service-type` is a usage error naming the flag, the value and
    the remedy; no type means browse-all; default-domain spellings; mdns-sd with
    a custom domain; zeroconf+custom domain refused naming `--domain` and the
    mdns-sd remedy; zeroconf accepts the default domain; fake bypasses the
    capability check; **`list-commands` parses despite a malformed
    `--service-type` and despite an unsupported backend/domain pair**.
  - `discovery::zeroconf`: **replaced `unparseable_filter_falls_back_to_default_sweep`**
    (which enshrined the defect) with `an_invalid_filter_is_rejected_before_it_can_widen_the_browse`;
    added `a_custom_domain_is_rejected_before_any_browser_starts` and
    `an_explicit_udp_filter_browses_that_one_udp_type`; kept explicit-type-yields-one-browser
    and `None`-yields-defaults.
  - `discovery::worker`: `the_loop_is_handed_the_validated_domain_and_filter`
    now proves the loop receives canonical values (`_SSH._tcp` → `_ssh._tcp`).
  - `discovery::{session,fake}` updated to the validated-options interface.
- **Documentation updated:**
  - `README.md`: new "Discovery backends" section stating that zeroconf browses
    only `local` and why, with the actual error text; new "Limiting discovery to
    one service type" section giving the grammar, case-insensitivity, and the
    rejection example; the architecture section now records the
    request-vs-validated-options seam.
  - CLI help: `--service-type` shows the shape and the browse-all default;
    `--domain` and `--backend` state the zeroconf domain limitation.
- **Validation evidence:**
  - `cargo fmt -- --check`: clean.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`: clean.
    Also clean with default features.
  - `cargo test --locked --all-targets`: 253 passed, 0 failed.
  - `cargo test --locked --all-targets --all-features`: 268 passed, 0 failed
    (baseline before this task: 253/258 — this task adds 10 CLI + 13 options
    tests, replacing 1).
  - Targeted: `ui::cli` 13 passed; `--all-features discovery::zeroconf` 8 passed;
    `--all-features discovery::options` 13 passed.
  - The `zeroconf` feature builds in this environment (Avahi client headers
    present), so the all-features gate really ran; no environment limitation.
  - Driven end-to-end against the built binary:
    `kinjo --service-type bogus` → exit 2 with "invalid value for
    `--service-type`: `bogus` is not a DNS-SD service type … or omit it to
    browse every service type" (no discovery started);
    `kinjo --service-type bogus list-commands` → exit 0, lists commands;
    `kinjo --backend zeroconf --domain corp` → exit 2 with "invalid value for
    `--domain`: the `zeroconf` backend cannot browse the `corp` domain …";
    `kinjo --backend zeroconf --domain local. list-commands` → exit 0.
- **Follow-ups:**
  - **The `discovery_entry` fuzz target does not compile, on `main`, independent
    of this task.** It imports `kinjo::discovery::group_entries`, which commit
    `2ffb5b8` (task 010) renamed to `browse_groups` without updating the target;
    `git show main:src/discovery/mod.rs | grep -c group_entries` → 0. Left alone
    here rather than broadening this task's scope. The other four targets
    (`decode_dns_sd`, `parse_command`, `prepare_command`, `command_roundtrip`)
    build against this change. Worth a small new task, or folding into 015.
  - The new service-type grammar is a parsing surface but is outside every
    existing fuzz target's oracle, so none was extended. `ServiceTypeFilter::parse`
    is total by construction (the only byte indexing is guarded by an is-empty
    check), and 18 malformed inputs are covered by unit tests; a dedicated fuzz
    target would be cheap if the grammar grows.
  - `Cli::discovery_config` stays public alongside `discovery_options`. It cannot
    be used to bypass validation (`start` demands `DiscoveryOptions`), but task
    015 may want to collapse the two projections.
