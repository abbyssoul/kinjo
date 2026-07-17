# Task 112 — Support TXT keys longer than nine bytes

- **Priority**: P1 (network data correctness)
- **Status**: done
- **Depends on**: 111 (same module; 111 creates the code this changes)
- **Likely conflicts**: 111 (`src/discovery/txt.rs`), 102 (field lookup)

## Owner ruling (2026-07-17)

> "If devices in the wild use more than 9-byte TXT keys, there's no reason not
> to support them in the TUI."

Task 111 was already in flight when this was found, so the ceiling ships as
written and this task lifts it. **This task amends
[ADR 0004](../../adr/0004-dns-sd-txt-is-portable-text.md)** — see "Amend ADR
0004" below.

## Problem

[ADR 0004](../../adr/0004-dns-sd-txt-is-portable-text.md) requires TXT keys to
be "1-9 printable US-ASCII bytes excluding `=`", and `canonical_key` in
`src/discovery/txt.rs` enforces it:

```rust
if !(1..=9).contains(&bytes.len()) || /* ... */ { return None; }
```

A key outside that range is dropped silently by `TextTxtMap::observe_bytes`,
and `keys_are_validated_and_canonicalized` currently asserts that behaviour
(`toolongkey` → gone).

**RFC 6763 §6.4 makes nine characters a SHOULD, not a MUST**: "The key SHOULD
be no more than nine characters long." Only "at least one character" and the
printable-ASCII-excluding-`=` rule are MUSTs. Devices legally exceed the
recommendation, and common ones do:

- the Bonjour Printing Specification uses `printer-type` (12) and
  `printer-state` (13);
- `mopria-certified` (16) is widely advertised by IPP printers.

Two costs, both real:

1. **The data is invisible in the TUI.** A printer's `printer-type` never
   reaches `Entry::txt`, so it cannot appear in the details pane. The user can
   see the key with `avahi-browse` and not in kinjo.
2. **A rule on such a key loads clean and can never fire.** `is_supported_field`
   accepts any non-empty `txt.<key>`, so

   ```toml
   [match.txt.printer-type]
   equals = "3"
   ```

   validates, loads, and never matches — indistinguishable from "that printer
   is not on the network". This is the failure mode round-1 task 003 was raised
   to eliminate (the `service_typ` case), reintroduced through a different
   door.

## Goal

Accept the keys devices actually advertise, without weakening anything ADR 0004
got right.

## Scope

- Remove the 9-byte ceiling from `canonical_key`.
- **Keep every MUST**: at least one byte; printable US-ASCII (`0x20..=0x7e`)
  only; `=` excluded; lowercase canonicalization; case-insensitive
  first-wins duplicate semantics.
- Consider a defensive upper bound at **255 bytes**, the DNS-SD limit on a
  whole TXT string (RFC 6763 §6.1) — a key cannot legally exceed it, so this
  costs nothing, and it keeps the module aligned with ADR 0005's bounded
  posture rather than trusting an adapter to be well-behaved. If you add it,
  say in a comment that it is the protocol's own limit, not a revival of the
  SHOULD.
- Update `keys_are_validated_and_canonicalized`, which currently pins the
  behaviour being changed. Replace `toolongkey` with a case that is still
  invalid (empty, `=`-bearing, non-printable) and add a positive case for a
  real long key — use `printer-type` rather than a synthetic string, so the
  test states the reason the limit was lifted.

## Amend ADR 0004

ADR 0004 is `Accepted` and its key rule is now wrong in one clause. Follow
round 1's precedent (ADR 0001, whose bad argument was recorded *alongside* the
real reason rather than rewritten): amend the ADR in place with a dated note
that

- the "1-9" clause treated a SHOULD as a MUST;
- real devices exceed it, naming `printer-type` / `mopria-certified`;
- the ceiling is lifted while the MUSTs stand;
- the rest of ADR 0004 — text-only values, first-wins, no lossy decode — is
  **unchanged and still correct**.

Do not silently edit the clause: a future reader should be able to see that the
decision moved and why.

## Out of scope

**ADR 0004's "binary values are ignored" clause stands.** Its rationale is
sound and independent of key length: carrying bytes end to end would need a new
public representation for `Entry::txt`, predicates, templates, and prepared
commands, Windows process arguments are Unicode, and the zeroconf adapter
cannot recover bytes its dependency already decoded. Lossless binary support
remains the future breaking design ADR 0004 describes.

The narrower residual — that a **binary value** silently drops its key, so a
rule on it also never matches — is **accepted for now**. It is much rarer than
the long-key case (it needs a device advertising non-UTF-8 in a key someone
wrote a rule against) and cannot be fixed without the representation change
above. If it is ever worth closing, the cheap half is visibility, not bytes:
surface the key as present-but-unusable, the way `TxtValue::Mixed` already
reports a value that no single occurrence agrees on.

## Tests

- `printer-type`, `printer-state`, and `mopria-certified` survive
  normalization with their values intact.
- The MUSTs still reject: empty key, a key containing `=`, a key with a
  non-printable or non-ASCII byte.
- Case-insensitive first-wins still holds for a long key
  (`Printer-Type` then `printer-type` → first wins, canonical lowercase).
- An end-to-end matcher test: a rule with `[match.txt.printer-type]` matches a
  record whose adapter advertised that key. This is the test that would have
  caught the defect — it pins "a rule that loads can fire", not just the
  normalizer's unit behaviour.
- If the 255-byte bound is added: a 256-byte key is rejected, a 255-byte key is
  accepted.

## Definition of Done

- Long keys are accepted; the MUSTs are unchanged; the enshrining test is
  replaced by one that states the reason.
- ADR 0004 amended in place with a dated note, per round 1's precedent.
- `docs/actions.md` — if it documents a key-length limit, correct it.
- Completion gate green.

## Completion Record (2026-07-17)

- Lifted the mistaken nine-byte ceiling and retained every mandatory key rule.
  The defensive maximum is 255 ASCII bytes, the protocol's whole-string bound.
- Tests cover real printer/Mopria keys, long-key first-wins behavior, invalid
  ASCII/control/equals cases, and the 255/256-byte edge.
- An Adapter-to-matcher regression proves `[match.txt.printer-type]` loads and
  matches the normalized advertised value.
- ADR 0004 is visibly amended and `docs/actions.md` corrected. Completion gate
  passed.
