# Task 110 — Input handling polish: modifiers, bursts, label escaping

- **Priority**: P2 (input correctness/UX)
- **Status**: ready
- **Depends on**: none
- **Likely conflicts**: 103, 105 (same files)

Three small, independent input-path defects. They can be done together or split;
none is large.

## A. `typed_char` ignores SUPER/META/HYPER

`typed_char` (`src/ui/app.rs:1428-1436`) decides whether an unbound key event
should be typed into the search query. It excludes CONTROL and ALT so that a
modifier chord is treated as a (do-nothing) shortcut rather than text:

```rust
let modified = key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
```

It does **not** exclude `KeyModifiers::SUPER`, `META`, or `HYPER`. Terminals
that negotiate the kitty keyboard protocol (which crossterm supports and
ratatui can enable) report these. So a Super-`z` chord, which the user meant as
a window-manager or app shortcut, types `z` into the search box. The same
reasoning that excludes CONTROL/ALT ("a modified key is a shortcut, not text")
applies to all non-SHIFT modifiers.

**Fix:** treat any modifier other than SHIFT as "modified". Either intersect
against the full set (`CONTROL | ALT | SUPER | META | HYPER`) or, more robustly,
`modified = !(key.modifiers - KeyModifiers::SHIFT).is_empty()`. Keep SHIFT
folded into the character (the existing `shifted_characters_type_into_the_search_query`
test must still pass).

**Test:** a Super-modified (and Meta-modified) char does not type into search,
alongside the existing control-chord test
(`an_unbound_control_chord_does_not_type_into_the_search_query`).

## B. Input is processed one event per frame

The event loop (`src/ui/app.rs:374-393`) reads at most one event per iteration,
and each iteration pays a full `autoresize` + `update_layout` + `terminal.draw`
before the next `event::poll`. A burst — a mouse-wheel spin, a held arrow key,
a fast typist — is therefore serviced at the redraw rate (~8Hz worst case),
lagging behind the hand.

**Fix:** after handling one event, drain any further immediately-available
events (`event::poll(Duration::ZERO)` loop) before drawing, so a burst is
applied in one frame. Bound the drain (e.g. a max count, or "until no event is
ready") so a flood cannot starve the draw entirely. A quit or an exec-hand-off
mid-drain must still take effect immediately (return as today).

This composes with task 103 (skip dead frames): together they make the loop do
work when there is input or animation and rest otherwise. If 103 lands first,
build the drain on top of its change; if this lands first, 103 rebases.

**Test:** feeding several `Down` key events between draws moves the selection by
that many rows in one `drain`-and-render cycle (drive through `handle_key` in a
loop, or add a small drain helper that is unit-testable).

## C. A control character bound as a key reaches the footer unescaped

Keybinding labels are produced by `KeySpec::label` (`src/ui/keymap.rs:414-433`)
and rendered into the footer and help by `render.rs` (e.g. `render_footer`
`1038-1067`, `help_lines` `1271-1296`). Most label paths go through
`display::text`, but the footer hint keys are formatted directly
(`format!(" {key} ")`, `render.rs:1043`) without escaping. A keybinding file
can bind a single literal character key including a control character
(`char_or_function_key` accepts any single `char`, `keymap.rs:448-452`), so a
crafted keybindings file could put a raw control byte on the footer.

Keybinding files are trusted local config (`CONTEXT.md`), so this is
low-severity — but the round-1 rule is that *every* value reaching terminal
bytes crosses `terminal::text` first, and this one path skips it.

**Fix:** either escape the key label where the footer builds its spans (wrap the
`key` in `display::text`), or reject control characters in `KeySpec::parse`
(a control char is never a usable key binding anyway). Prefer rejecting at
parse time — it is the earlier, more honest boundary and gives the user a
config error instead of a mojibake footer.

**Test:** a keybindings file binding a control character is rejected at load
(if fixed at parse time), or the footer/help escape it (if fixed at render).

## Constraints

- SHIFT stays folded into characters (A).
- Quit/exec must remain immediate during a drain (B).
- Render stays pure over `App` (ADR 0002) (C).
- Coordinate file ownership with 103 (loop/render) and 105 (app.rs) if they are
  in flight.

## Definition of Done

- All three fixed (or the ones taken, with the others left as noted follow-ups).
- Tests as above; existing input tests unchanged.
- Completion gate green.
