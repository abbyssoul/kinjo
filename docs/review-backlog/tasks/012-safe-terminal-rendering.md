# 012: Render Untrusted Text Safely and Measure Display Width

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P0` |
| Workstream | UI / Safety |
| Depends on | — |
| Likely conflicts | 010, 011, 013, 014 |
| Owner | Codex (`main`) |

## Why This Matters

Discovery names and TXT data originate on the local network and are passed into
Ratatui spans. Terminal control characters can alter terminal state rather than
render as inert data. Separately, right alignment counts Unicode scalar values,
so wide or combining characters produce incorrect layout.

Safety belongs at the display seam: command matching/interpolation still need the
raw network value, while every renderer receives an inert display representation.

## Evidence

- `src/discovery/mdns.rs:273-301`: network names/host/TXT become entry strings,
  including lossy binary TXT conversion without display sanitization.
- `src/discovery/entry.rs:222-238`: decoded display names can contain arbitrary
  decoded bytes/characters.
- `src/ui/render.rs:260-390`: names, hosts, domains, and TXT data are inserted into
  spans/cells as raw strings.
- Other render paths display command metadata, status, domain, and search text
  without one shared safe-text interface.
- `src/ui/render.rs:1043-1048`: right alignment uses `chars().count()` rather than
  terminal display width.

## Required Outcome

- Add one display-text function/module used for every dynamic string rendered to
  the terminal, including discovered data, CLI domain, status/error text, config
  metadata, and user search text.
- Escape Unicode control characters into visible inert notation. Use `\\xNN` for
  ASCII/C1 byte-range controls where practical and `\\u{...}` for other control
  characters. Newline, carriage return, tab, BEL, ESC, DEL, and C1 controls must
  never reach the terminal backend raw.
- Preserve ordinary Unicode text unchanged. Do not mutate stored/raw Entry or
  command values; command interpolation must receive the original data.
- Perform truncation/alignment after escaping and use terminal column width.
- Replace scalar-count layout with Ratatui/Unicode-width display-width helpers.
- Keep narrow-area arithmetic saturating and panic-free.

## Implementation Constraints

- Sanitize at rendering/display conversion, not discovery ingestion.
- Avoid ad hoc sanitization at individual fields; render should make unsafe raw
  insertion difficult by construction.
- Tests must inspect the final TestBackend buffer or safe display values, not only
  an intermediate helper.
- Do not strip printable data silently; make controls visible for diagnosis.
- Treat this as output safety only, not shell escaping. Process execution remains
  argv-based and governed by command-rule tasks.

## Suggested Implementation Sequence

1. Add helper and TestBackend regressions containing ESC/BEL/newline/C1 controls.
2. Route all dynamic render strings through the safe display interface.
3. Replace width calculations with display-column width.
4. Add wide CJK, emoji, combining-mark, and escaped-control alignment tests.
5. Review every `Span::raw`, `Span::styled`, `Cell`, `Line`, and formatted dynamic
   string in `render.rs` for bypasses.

## Non-Goals

- Rejecting network records that contain controls.
- Normalizing Unicode or blocking bidirectional formatting characters unless a
  concrete terminal-control risk is demonstrated separately.
- Changing raw matching/interpolation semantics.
- Redesigning colors/layout.

## Acceptance Criteria / Definition of Done

- [x] No untrusted/dynamic control character reaches the rendered buffer raw.
- [x] Controls are represented visibly and consistently.
- [x] Raw Entry/config values remain unchanged for matching and execution.
- [x] Wide and combining Unicode align according to terminal columns.
- [x] Narrow/zero-width areas remain panic-free.
- [x] A source audit finds no dynamic render path bypassing safe display conversion.
- [x] Full validation passes.

## Required Tests

- Service name/TXT containing `ESC [ 2 J`, BEL, CR/LF, tab, DEL, and C1.
- Dynamic status/config/domain text containing controls.
- CJK/emoji/combining characters in left/right aligned spans.
- Safe output remains searchable/matchable through unchanged raw Entry data.
- Very narrow terminal with escaped text wider than the area.

## Validation

```sh
cargo test --locked ui::render
cargo test --locked discovery::entry
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:** Added `ui::display::text` as the display seam: it preserves
  printable Unicode and renders C0, DEL, and C1 controls as visible `\\xNN`
  escapes. Routed discovered fields, CLI domain, status/search text, command
  metadata, composite endpoints, chips, titles, and popup content through that
  seam before Ratatui truncation/alignment. Replaced scalar-count padding with
  `Span::width()` terminal-column measurements and saturating arithmetic.
- **Tests added/updated:** Added exact control-escape coverage and final
  `TestBackend` regressions for discovery/TXT data, domain/status/search text,
  command metadata, unchanged raw matching values, CJK/emoji/combining text,
  escaped-control alignment, and a one-column terminal.
- **Documentation updated:** Marked task 012 done in this record and the backlog
  index.
- **Validation evidence:** `cargo test --locked ui::render` (11 passed),
  `cargo test --locked discovery::entry` (26 passed), `cargo fmt -- --check`,
  `cargo clippy --locked --all-targets --all-features -- -D warnings`,
  `cargo test --locked --all-targets` (184 passed), and
  `cargo test --locked --all-targets --all-features` (189 passed).
- **Follow-ups:** None.
