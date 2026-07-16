# 017: Give Fake Discovery a Second SSH Host

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P2` |
| Workstream | Discovery |
| Depends on | 006 |
| Likely conflicts | — |
| Owner | Unclaimed |

## Why This Matters

Explicit fake mode is the project's smoke-test surface: CONTEXT states it
"remains suitable for development and smoke tests", and it is the only way to
exercise the UI without a live network.

Its sample set cannot currently produce a heterogeneous aggregate row. It
advertises exactly one `_ssh._tcp` service, so the `_ssh._tcp` service-type row
has a single child, and the `ssh {hostname}` rule prepares one command. The
behavior task 006 introduced — several children preparing *different* commands,
so the user is asked which host to act on — is therefore unreachable by hand.

Task 006 was completed on regression tests alone for this reason, and its
completion record names this gap. That is a real hole in the project's ability to
verify its own P0 fix: the instance picker for a non-address rule has never been
seen by a person, only asserted.

This is a test-affordance task, not a correctness fix. It is worth doing because
the affordance is what lets the next reviewer confirm 006 without reading code.

## Evidence

- `src/discovery/fake.rs:67-92`: `fake_records` returns one `_ssh._tcp` entry
  (`workstation`, on `workstation.local`, two addresses), one `_http._tcp`, one
  `_https._tcp`, and one unresolved `_ipp._tcp`.
- `src/discovery/fake.rs:107-113`: a test asserts the SSH service is exactly one
  entry carrying both addresses. That invariant is deliberate — it covers address
  expansion — and must survive, so a second host is an addition, not an edit.
- `actions/ssh.toml`: the shipped rule is `ssh {hostname}`, the exact rule shape
  task 006 fixed.
- Task 006 completion record: "the fake backend advertises a single `_ssh._tcp`
  service, so it cannot present a service-type row with two hosts."

Reproduction of the gap, once this task lands the picker should appear:

```sh
kinjo --backend fake            # group by service type, select _ssh._tcp, press Enter
```

## Required Outcome

- Fake discovery advertises a second `_ssh._tcp` occurrence on a *different*
  resolved host, so the `_ssh._tcp` service-type row has two children whose
  `ssh {hostname}` commands differ.
- Selecting that row and invoking `ssh` opens the instance picker, and choosing a
  child runs that child's host.
- The existing single-logical-service-with-two-addresses entry is preserved
  unchanged, so address-expansion behavior stays demonstrable.
- The unresolved entry and the requested-domain propagation stay as they are.
- Sample records remain obviously fake: reserved/documentation-style values, no
  address or hostname that could collide with a real network.

## Implementation Constraints

- Fake records are sample data, not a fixture API. Do not add a flag, an
  environment variable, or a scripting hook to vary them.
- Discovery must not depend on command rules or UI: the fake backend cannot know
  what `ssh {hostname}` is. It just advertises a plausible second host.
- Any counting test that a second SSH occurrence disturbs (tab counts, host rows,
  service-type rows) must be updated to the new correct expectation rather than
  worked around.
- Explicit fake mode still streams sample records and must not become the
  fallback for real discovery failure — see task 016.

## Suggested Implementation Sequence

1. Add the second `_ssh._tcp` entry on its own host in `fake_records`.
2. Update `fake_records_carry_the_requested_domain_and_an_unresolved_entry` and
   any grouping/count tests the addition disturbs.
3. Drive `kinjo --backend fake` by hand: group by service type, invoke `ssh` on
   the `_ssh._tcp` row, confirm the picker lists both hosts and that choosing one
   runs that host.
4. Record the manual verification against task 006's behavior.

## Non-Goals

- Changing what fake mode is for, or making its record set configurable.
- Adding sample records for their own sake beyond the second SSH host.
- Revisiting task 006's selection semantics; this task only makes them visible.
- Changing the bundled `actions/` rules.

## Acceptance Criteria / Definition of Done

- [ ] `fake_records` yields two `_ssh._tcp` occurrences on different hosts.
- [ ] The existing SSH entry still carries both of its addresses on one entry.
- [ ] The `_ssh._tcp` service-type row presents two children, and invoking the
      shipped `ssh` rule opens the instance picker.
- [ ] Choosing a child runs that child's hostname.
- [ ] Tests disturbed by the extra record state the new correct expectation.
- [ ] Task 006's behavior is confirmed by hand and the evidence recorded.
- [ ] Full validation passes.

## Required Tests

- `discovery::fake`: two `_ssh._tcp` occurrences, on distinct resolved hosts,
  carrying the requested domain.
- `discovery::fake`: the multi-address SSH entry invariant still holds.
- `ui::app`: fake-derived records grouped by service type offer selection for a
  `{hostname}` rule. Prefer asserting through the existing app test interface
  rather than duplicating 006's plumber-level coverage.

## Validation

```sh
cargo test --locked discovery::fake
cargo test --locked ui::app
cargo run --locked -- --backend fake   # confirm the picker by hand
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
