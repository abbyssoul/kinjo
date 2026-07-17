# ADR 0006: Rendered text cannot reorder itself

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-07-17 |
| Decider | Ivan Ryabov (project owner) |
| Context | [round-2 follow-up validation, finding 4](../review-backlog-2/README.md#newly-identified-findings) |

## Context

Round 1 established that every value crossing into terminal bytes passes
through `terminal::text`, which escaped anything `char::is_control()` reports.
In Rust that is Unicode category **Cc** only: C0, DEL, and C1. Bidi formatting
characters are category **Cf**, so `U+202E RIGHT-TO-LEFT OVERRIDE` and its
relatives were passed through unchanged. That was deliberate, not an oversight —
`terminal.rs`'s own test listed `\u{202e}` inside a string named `printable`.

Two properties made this reachable. `unicode-width` reports these characters as
zero columns, so layout treats them as absent and hands them to the terminal
intact; and an override with no matching `U+202C POP DIRECTIONAL FORMATTING`
continues to the end of the paragraph, so its effect is not confined to the
value carrying it.

Discovered service names, hostnames, and TXT values are attacker-controlled
arbitrary UTF-8. The risk is **not** that a name can be misleading — an
attacker can already name a service `nas.local — trusted fileserver` in plain
ASCII, and Kinjo has never claimed a display name is trustworthy. The risk is
that Kinjo **displays the same field it executes**, and a bidi override makes
the two diverge:

- a hostname stored as `\u{202E}lacol.san` renders as `nas.local` in the
  details pane;
- the user consents to `ssh -- {hostname}` on the strength of that rendering;
- the command runs against the raw string, which the attacker's own responder
  answers for, with their own address.

The consent was obtained against a rendering that did not match the value. A
dangling override is worse than mis-rendering one field: it reorders Kinjo's
own labels further along the same line, so untrusted data rearranges trusted UI.

The cost of escaping is smaller than it first appears. Hebrew and Arabic
letters carry **inherent** strong RTL directionality in the Unicode
bidirectional algorithm; they render correctly with no formatting characters at
all. The explicit embeddings and overrides are needed only for unusual
mixed-direction text, effectively never in a service name.

## Decision

Text rendered by Kinjo cannot reorder itself or its surroundings.
`terminal::text` escapes bidi formatting characters with the same `\u{...}`
notation it already uses for controls outside a byte:

- `U+061C` ARABIC LETTER MARK
- `U+200E`, `U+200F` LEFT-TO-RIGHT / RIGHT-TO-LEFT MARK
- `U+202A`–`U+202E` the embeddings, the pop, and the overrides
- `U+2066`–`U+2069` the isolates and their pop

That is the complete Unicode bidi formatting set, chosen as a set rather than
as the subset an attack happens to use today.

Escaping stays at the presentation boundary only. Discovered values remain
exact for matching and interpolation, per the round-1 rule this ADR does not
touch: a value is escaped when it becomes terminal bytes, never before.

This reverses the round-1 decision that bidi formatting is `printable`. The
test that asserted it (`terminal.rs`) is updated rather than deleted, so the
reversal is visible.

## Consequences

Positive:

- A rendered value cannot claim to be a different value, so a user's consent is
  given against what will actually run.
- Untrusted text cannot reorder adjacent trusted UI text.
- One rule covers every surface, because every surface already crosses
  `terminal::text`.

Negative:

- A name legitimately using `U+200E`/`U+200F` to fix up a mixed LTR/RTL run —
  a Hebrew name followed by a Latin model number, say — renders with a visible
  escape instead of the intended ordering. This is the real cost of the set
  decision, and it is accepted: an exotic name rendering awkwardly is a smaller
  harm than a hostname that lies about where a command will connect.
- Naturally RTL names are unaffected, so the cost does not fall on RTL users
  generally.

## Rejected alternatives

- **Accept as residual risk:** defensible on reach — terminal bidi support is
  inconsistent (VTE implements it, xterm optionally, Alacritty and kitty
  largely do not), so the attack is inert on many terminals. Rejected because
  the fix is cheap and total, the blast radius on the terminals that *do*
  implement bidi is a spoofed execution target, and "safe depending on the
  user's terminal emulator" is not a property this trust model wants to state.
- **Escape only the overrides (`U+202D`/`U+202E`):** rejected as fixing the
  attack that was demonstrated rather than the capability behind it. The
  embeddings and isolates reorder too.
- **Strip rather than escape:** rejected for consistency with controls, which
  are made visible rather than removed. Silently deleting a character the
  device advertised is the erasure this project already refuses elsewhere.
- **Rely on `unicode-width` for safety:** rejected as a category error. Width
  is a layout question; these characters are zero-width *and* reordering, which
  is exactly why the layout math never noticed them.
