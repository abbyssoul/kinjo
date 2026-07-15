# 003: Validate Discovery Options and Adapter Capabilities

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P0` |
| Workstream | Discovery / CLI |
| Depends on | 002 |
| Likely conflicts | 009, 015 |
| Owner | Unclaimed |

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

- [ ] A malformed explicit service type fails without starting broad discovery.
- [ ] Valid `_name._tcp` and `_name._udp` inputs browse exactly one type.
- [ ] Zeroconf with a custom domain fails before spawning with actionable text.
- [ ] mdns-sd custom-domain behavior remains covered.
- [ ] CLI help/README explain backend capability accurately.
- [ ] Default and all-feature validation pass.

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
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
