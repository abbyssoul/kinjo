# 001: Preserve Discovery Occurrence Identity

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P0` |
| Workstream | Discovery |
| Depends on | — |
| Likely conflicts | 002, 006, 008, 010, 014, 015 |
| Owner | Unclaimed |

## Why This Matters

Kinjo currently identifies resolved entries by registration plus host/port. The
mdns-sd adapter's Found and Removed events expose an interface index, but Kinjo
discards it. Two occurrences of
the same registration on different interfaces can therefore overwrite each
other, and a removal on one interface can delete the remaining live occurrence.
Users see lost addresses or an entire logical service disappearing incorrectly.

Occurrence identity is a discovery concern; logical-service merging is a UI
grouping concern. Concentrating that distinction in the entry module gives every
adapter and caller the same invariant.

## Evidence

- `src/discovery/mdns.rs:25-26`: `ServiceKey` contains only name, type, and domain.
- `src/discovery/mdns.rs:190-203`: tracking ignores `interface_index`.
- `src/discovery/mdns.rs:273-282`: adapter conversion does not carry the interface.
- `src/discovery/entry.rs:7-18`: `EntryId` claims concurrent instances stay
  distinct, but its resolved discriminator is only host/port.
- `src/discovery/entry.rs:146-153`: resolved identity is derived without an
  occurrence discriminator.
- `src/ui/app.rs:215-220`: removal retains/drops by registration and therefore
  removes every matching entry.

The current zeroconf event types expose no equivalent occurrence discriminator;
that difference is part of the required adapter behavior below.

Severity nuance: Avahi-published multi-homed services usually differ per
interface by *instance name* (see the MAC-suffix handling in
`src/discovery/entry.rs:226-238`), so those occurrences already have distinct
registrations. The identical-name cross-interface collision this task fixes
arises mainly with non-Avahi responders that announce one instance name on
several interfaces. The fix and priority stand; do not expect avahi-daemon
alone to reproduce the collision.

## Required Outcome

- Represent a backend occurrence with a structured identity that includes the
  registration and an optional adapter-provided stable discriminator.
- The mdns-sd adapter uses its Found/Removed interface index as that discriminator for Found,
  tracking/probing, upsert, and remove paths.
- An exact Remove event deletes only the named occurrence. The current zeroconf
  adapter exposes no discriminator; its removal remains an explicit
  registration-wide removal. An mdns-sd event whose interface index is absent
  uses the same documented registration-wide fallback.
- Multiple occurrences with the same name/type/domain/host/port coexist in the
  record store.
- Logical-service grouping still merges compatible occurrences for display.
- Address-set and TXT updates for one occurrence replace that occurrence rather
  than creating duplicates.

Do not encode interface identity in display labels or command fields.

## Implementation Constraints

- Keep identity-defining state private or invariant-preserving; avoid adding
  another public field callers must remember to synchronize with `EntryId`.
- Keep discovery independent of UI and command rules.
- Prefer structured keys over delimiter-joined strings.
- Preserve registration-wide removal as an explicit operation where an adapter
  genuinely cannot identify a narrower occurrence.
- Update the `discovery_entry` fuzz oracle if the public identity invariant changes.

## Suggested Implementation Sequence

1. Add entry/app regression tests with two otherwise identical occurrences that
   differ only by discriminator and address.
2. Introduce the structured occurrence identity through `Entry` construction.
3. Carry the discriminator through mdns-sd tracking, probes, and event conversion.
4. Split exact-occurrence and registration-wide removal semantics in events/app.
5. Confirm logical grouping continues to merge the two occurrences.

## Non-Goals

- Replacing the `Discovery` lifecycle interface; task 002 owns that work.
- Changing logical-service grouping policy beyond preserving occurrence data.
- Displaying network interfaces in the TUI.

## Acceptance Criteria / Definition of Done

- [ ] Two same-registration, same-host/port occurrences from different interfaces
      coexist and retain their own addresses.
- [ ] Removing one occurrence preserves the other in records and grouped views.
- [ ] Liveness failure/recovery affects only the tracked occurrence.
- [ ] Registration-wide removal remains testable for adapters lacking a discriminator.
- [ ] Entry construction cannot produce stale duplicated identity state through
      normal public mutation.
- [ ] Identity/grouping documentation and fuzz properties are updated as needed.
- [ ] Full validation passes.

## Required Tests

- `discovery::entry`: occurrence equality, address update stability, grouping.
- `discovery::mdns`: interface-specific Found/Remove and liveness transitions.
- `ui::app`: exact removal preserves sibling occurrence; registration removal
  clears all siblings.
- Upserts that change endpoint/TXT/address data replace the same occurrence.
- Unknown-interface mdns-sd removal and zeroconf removal exercise the explicit
  registration-wide fallback.
- Fuzz: identity equality matches its defining field tuple.

## Validation

```sh
cargo test --locked discovery::entry
cargo test --locked discovery::mdns
cargo test --locked ui::app::tests::remove
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
