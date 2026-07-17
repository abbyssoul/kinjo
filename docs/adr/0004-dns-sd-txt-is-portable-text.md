# ADR 0004: DNS-SD TXT is portable text

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-07-17 |
| Decider | Ivan Ryabov (project owner) |
| Context | [round-2 follow-up validation](../review-backlog-2/README.md#newly-identified-findings) |

## Context

DNS-SD TXT values are opaque bytes, keys compare case-insensitively, and the
first duplicate key wins. The mdns-sd Adapter exposed those bytes, but Kinjo
used `String::from_utf8_lossy`, compared keys case-sensitively, and collected
duplicates last-wins. That changed network data before matching and
interpolation.

Carrying bytes end-to-end would require a new public representation for
`Entry::txt`, predicates, templates, rendering, and prepared commands. Unix
process arguments can contain arbitrary non-NUL bytes, whereas Windows process
arguments are Unicode. The zeroconf Adapter also receives TXT values as
`String`, so it cannot recover bytes already decoded by its dependency.

## Decision

Kinjo's portable product model is text-only for this release:

- TXT keys must be 1-9 printable US-ASCII bytes excluding `=`, are canonicalized
  to lowercase ASCII, compare case-insensitively, and use first-wins semantics.
- Key-only entries have the empty string as their text value.
- A value is actionable only when it is valid UTF-8. Binary values are ignored,
  never converted with replacement characters.
- An invalid-UTF-8 first value still owns its key, so a later case-variant
  duplicate cannot replace it.
- Rule lookup of manually constructed `Entry` values remains
  ASCII-case-insensitive for library compatibility.

The normalization Module is private to discovery and shared by the concrete
Adapters. It adds Locality without creating a new public Seam.

## Amendment: long keys (2026-07-17)

The `1-9` clause above treated RFC 6763 section 6.4's `SHOULD` as a `MUST`.
Real devices exceed that recommendation: Bonjour printers advertise keys such
as `printer-type` and `printer-state`, and Mopria devices advertise
`mopria-certified`. Dropping them made valid network data invisible and let a
valid rule name a field that the discovery Adapter could never produce.

That length clause is superseded. Keys may be 1-255 printable US-ASCII bytes
excluding `=`; 255 is the protocol limit of the whole length-prefixed TXT
string, not a replacement recommendation. Lowercase canonicalization,
case-insensitive first-wins identity, exact UTF-8 text values, and rejection of
lossy/binary values are unchanged and remain the decision.

## Consequences

Positive:

- Matching and interpolation receive exactly the text advertised, never a
  lossy invention.
- Key identity and duplicate handling follow RFC 6763.
- The Interface remains portable across Unix and Windows and does not break the
  public `Entry::txt` type.

Negative:

- Valid binary DNS-SD TXT values are not available for matching or command
  interpolation.
- The zeroconf Adapter cannot reconstruct original bytes or duplicate ordering
  after its dependency has exposed a string map.
- Lossless binary support remains a future breaking design that must specify a
  cross-platform textual/argv encoding.
