# Task 111 — Portable DNS-SD TXT semantics

- **Priority**: P1 (network data correctness)
- **Status**: done
- **Depends on**: none
- **Likely conflicts**: 102 (field lookup)

## Problem

The mdns-sd Adapter compared TXT keys case-sensitively, collected duplicates
last-wins, and decoded opaque values with `String::from_utf8_lossy`. That
disagreed with RFC 6763 and with Kinjo's exact-text matching/interpolation
model. The zeroconf Adapter had its own independent conversion.

## Owner decision

[ADR 0004](../../adr/0004-dns-sd-txt-is-portable-text.md) chooses a portable
text-only model: canonical valid keys, first-wins, and exact UTF-8 values.
Binary values are ignored rather than changed into replacement text.

## Scope

- One private normalization Module shared by both discovery Adapters.
- Case-insensitive, lowercase, first-wins key handling.
- Invalid keys and non-UTF-8 values never become actionable.
- Case-insensitive rule lookup for public manually constructed entries.
- Regression tests and user documentation.

## Definition of Done

- RFC key/duplicate semantics are tested.
- Invalid UTF-8 first-wins behavior is tested.
- Default and all-feature completion gates pass.
- Task status and validation record are updated.

## The 9-byte key ceiling — ruled, deferred to task 112 (2026-07-17)

> **This does not block task 111. Finish it as scoped and ship the ceiling as
> written.** The owner has ruled that keys longer than nine bytes should be
> supported ("if devices in the wild use more than 9-byte TXT keys, there's no
> reason not to support them in the TUI"), but this task was already in flight
> when the finding landed, so the change is
> [task 112](112-long-txt-keys.md), which also amends ADR 0004. The analysis
> below is retained as the evidence behind that ruling.

**Review finding against the implementation in `src/discovery/txt.rs`.**

ADR 0004 and `canonical_key` require keys to be 1–9 bytes. **RFC 6763 §6.4 makes
that a SHOULD, not a MUST** — "The key SHOULD be no more than nine characters
long". Devices legally exceed it, and common ones do:

- the Bonjour Printing Specification uses `printer-type` (12) and
  `printer-state` (13);
- `mopria-certified` (16) is widely advertised by IPP printers.

`TextTxtMap::observe_bytes` drops those keys silently, and
`keys_are_validated_and_canonicalized` currently enshrines that (`toolongkey`
→ gone).

The consequence is the failure mode round 1 spent task 003 eliminating. A user
writes:

```toml
[match.txt.printer-type]
equals = "3"
```

`is_supported_field` accepts any non-empty `txt.<key>`, so the rule loads
clean — and then **never matches**, which is indistinguishable from "that
printer is not on the network". That is precisely the `service_typ` defect,
reintroduced through a different door: a rule that is accepted at load and
cannot fire.

The same argument applies to the ADR's "binary values are ignored" clause. Not
*inventing* text (the lossy-decode fix) is right; silently *erasing* advertised
data is a different failure, and it is the one round 1 cared about more. A key
that vanishes without trace is less honest than one surfaced as
present-but-unusable — which is exactly what `TxtValue::Mixed` already does for
the disagreement case.

**Ruling: accept keys longer than nine bytes.** Keep the printable-ASCII-
minus-`=` rule, which *is* a MUST, and drop the length ceiling — real data
stops disappearing and the RFC is still honoured. Implemented by
[task 112](112-long-txt-keys.md), which lifts the ceiling, amends ADR 0004's
"1-9 printable US-ASCII bytes" clause, and replaces
`keys_are_validated_and_canonicalized`'s `toolongkey` assertion.

The related residual — a **binary** value silently dropping its key — is
**accepted for now** and is not task 112's business either. ADR 0004's
text-only rationale is independent of key length and still sound; see task
112's "Out of scope".

## Completion Record (2026-07-17)

- Added one private `TextTxtMap` normalization Module shared by the mDNS and
  zeroconf Adapters. Keys are lowercase/case-insensitive first-wins; invalid
  keys and non-UTF-8 values never become lossy actionable text.
- An invalid-UTF-8 first value still claims its key. Public manually built
  entries retain case-insensitive TXT lookup compatibility.
- ADR 0004 and action documentation record the portable text-only model. Task
  112 subsequently corrected only the key-length clause; the accepted binary
  value residual remains unchanged.
- Default and all-feature completion gates passed.
