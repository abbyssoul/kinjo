# Task 107 — Bound hostile record growth

- **Priority**: P2 (safety / resource use)
- **Status**: ready
- **Depends on**: 102 (bound the reshaped pipeline, not the current one)
- **Likely conflicts**: 102

## Problem

`App::records` (`src/ui/app.rs:177`) is a `BTreeMap<EntryId, Entry>` that grows
with every distinct occurrence discovery reports and only shrinks on explicit
`Remove`/`RemoveRegistration` or session failure (`drain_discovery`,
`app.rs:434-455`). There is no upper bound.

An `EntryId`'s identity includes the registration `(name, service_type,
domain)` and, for the mdns-sd backend, an occurrence discriminator
(`entry.rs:65-100`, `299-312`). A device on the link fully controls all of
these fields. A hostile or misconfigured host can therefore mint unbounded
distinct registrations (`evil-0`, `evil-1`, …) or unbounded endpoints, each a
new map entry. Every insertion also feeds the recompute pipeline (task 102),
which is superlinear in a couple of places, so growth is amplified into CPU as
well as memory.

The liveness tracker (`mdns.rs`) probes and eventually removes silently-dead
services, which caps *steady-state* count for well-behaved networks — but an
attacker who keeps re-announcing defeats that, and the probe cycle itself grows
with the tracked set.

This is not a live incident (a normal LAN has tens to low-hundreds of
services), and it is deliberately P2. But an mDNS tool's entire input is
untrusted by definition, and "the network can OOM the client" is worth a
bound.

## Goal

Put a defensible ceiling on how much discovery can make the app hold and
recompute, chosen so a real network is never affected and a flood degrades
gracefully (drops/ignores the excess with a visible status) rather than growing
without limit.

## Suggested approach (agent to choose and justify)

- A configurable-but-defaulted cap on `records.len()` (e.g. a few thousand),
  enforced in `drain_discovery` when applying `Upsert`. On overflow: reject new
  *registrations* while still accepting updates to already-known ones (so a
  legitimate service already listed keeps resolving), and set a status line
  that says discovery is capped. Prefer dropping the newest unknown over
  evicting a known one — eviction policy is a real decision; document it.
- Consider whether the cap belongs in the UI layer (`App`) or the discovery
  layer. The trust boundary is discovery's, but the resource being protected
  (records + recompute) is the UI's. Either is defensible; record the choice.
- Keep it observable: a silently capped list looks like a quiet network, which
  the round-1 trust model forbids for *failures* and the same reasoning applies
  here. The user must be able to tell "capped" from "that's all there is".

## Constraints

- A normal network must never hit the cap. Choose the default with headroom.
- Do not break occurrence identity or removal semantics: an evicted/rejected
  record must not corrupt the identity of the ones kept.
- Land after 102 so the bound guards the cheaper pipeline.

## Tests

- Feeding more than the cap distinct registrations through `drain_discovery`
  leaves `records.len()` at the cap and sets an observable "capped" status.
- Updates to an already-listed occurrence still apply when at the cap.
- Removing records below the cap re-opens capacity.

## Definition of Done

- `records` growth is bounded with a documented eviction/rejection policy and
  an observable capped state.
- The chosen layer (UI vs discovery) and default are justified in the code and,
  if it is an enduring decision, in an ADR.
- Completion gate green.
