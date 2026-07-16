# 001: Preserve Discovery Occurrence Identity

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
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

- [x] Two same-registration, same-host/port occurrences from different interfaces
      coexist and retain their own addresses.
- [x] Removing one occurrence preserves the other in records and grouped views.
- [x] Liveness failure/recovery affects only the tracked occurrence.
- [x] Registration-wide removal remains testable for adapters lacking a discriminator.
- [x] Entry construction cannot produce stale duplicated identity state through
      normal public mutation.
- [x] Identity/grouping documentation and fuzz properties are updated as needed.
- [x] Full validation passes.

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
  - `discovery::entry` now models identity as `EntryId { registration, occurrence }`.
    `Registration` is the structured `(name, service type, domain)` triple;
    `OccurrenceId` is an adapter's opaque, stable name for one occurrence.
    The private `Occurrence` enum makes the three cases explicit rather than
    implied: `Named` (adapter named it), `Endpoint` (no name — the resolved
    host/port discriminates, as before), and `Pending` (a registration
    placeholder, not an occurrence). `EntryId`'s fields are private; callers use
    `registration()`, `is_pending()`, and the `named`/`pending` constructors.
  - `Entry::id` changed from a stored field to a method derived from the entry's
    current fields, so the stale-identity class of bug is gone by construction
    rather than by remembering to call a sync method. `with_instance_id` and
    `pending_id` are deleted along with it. The only identity-defining state that
    is not already a public field — the occurrence name — is private and settable
    only via `with_occurrence` at construction.
  - A `Named` occurrence's id is stable across endpoint/address/TXT changes, so
    re-resolving one occurrence updates it in place. Addresses remain out of
    identity for both cases.
  - `DiscoveryEvent::Remove(EntryId)` now means *exactly this occurrence*; the new
    `DiscoveryEvent::RemoveRegistration(Registration)` is the explicit
    registration-wide operation for adapters that genuinely cannot name what they
    lost. `App::drain_discovery` removes by exact id and retains siblings.
  - The mdns-sd adapter carries `interface_index` as the discriminator through
    Found, tracking, probing, upsert, and remove. `ServiceKey` became a struct of
    `(Registration, Option<NonZeroU32>)`. Removals with an interface index name one
    occurrence; without one they use the documented registration-wide fallback,
    which also forgets every tracked occurrence of that registration.
  - Liveness probes are now confined to the tracked interface via
    `ServiceResolverBuilder::interface_index`. This was a latent correctness bug:
    an unconfined resolve answers from *any* interface still carrying the
    registration, so a dead occurrence would have been reported alive for as long
    as one sibling survived. Probe recovery upserts with the *tracked* key's
    occurrence, so it lands on the record being probed.
  - The zeroconf adapter's removal is now an explicit `RemoveRegistration`, which
    is what it always meant; `id_from_removal` became `registration_from_removal`.
- **Tests added/updated:** +16 default / +18 all-features (163→179, 166→184).
  - `discovery::entry`: occurrences differing only by adapter name are unequal; a
    named occurrence keeps its id across endpoint/address/TXT changes; endpoint
    identity still separates unnamed occurrences while addresses do not; two
    occurrences still merge into one logical service. Rewrote the id tests that
    asserted the old `instance`/`registration_key` shape.
  - `discovery::mdns`: two-interface Found yields two occurrences;
    interface-specific removal names exactly that occurrence; unknown-interface
    removal falls back registration-wide; per-interface liveness transitions;
    probe failure removes only the failing occurrence; recovery upserts the probed
    occurrence; registration-wide removal forgets all tracked occurrences while an
    interface-specific one keeps probing the sibling.
  - `discovery::zeroconf`: removal is an explicit registration-wide removal.
  - `ui::app`: occurrences coexist with their own addresses and group as one
    service; removing one preserves its sibling's address; registration removal
    clears all siblings but not other registrations; removing an unknown
    occurrence does not widen into a registration removal; upsert replaces the
    same occurrence across endpoint/TXT changes.
  - Fuzz: `identity()` mirrors the new invariant (occurrence name when present,
    otherwise endpoint), and `RawEntry` gained an arbitrary occurrence.
- **Documentation updated:** none required. The identity/grouping contract is
  documented on the types themselves and was rewritten there. `README.md`,
  `docs/actions.md`, and `docs/keybindings.md` describe no user-visible behavior
  this changes: no display label, command field, or configuration surface moved —
  removals simply stop deleting live siblings.
- **Validation evidence:**
  - `cargo fmt -- --check`: pass.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`: pass.
  - `cargo test --locked --all-targets`: 179 passed, 0 failed.
  - `cargo test --locked --all-targets --all-features`: 184 passed, 0 failed.
  - `cargo +nightly fuzz run discovery_entry -- -max_total_time=60`: 101,513 runs,
    no crashes — the identity property holds over the new occurrence dimension.
  - Ran the sample backend on a sized pty (now `kinjo --backend fake`): all four
    sample records render,
    including the unresolved placeholder, with no panic.
- **Follow-ups:**
  - The zeroconf backend still cannot separate same-name occurrences across
    interfaces, because its dependency exposes no discriminator at all. Its
    endpoint fallback is unchanged and its removals stay registration-wide. This
    is a dependency limitation, not a gap in this change; the seam is ready if
    `zeroconf` ever reports an interface.
  - `OccurrenceId` wraps `NonZeroU32` because that is what mdns-sd reports and
    because index 0 ("unspecified") correctly maps to "no name". If a future
    adapter needs a non-numeric discriminator, the newtype is the place to widen.
