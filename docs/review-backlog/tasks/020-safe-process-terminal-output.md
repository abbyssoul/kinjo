# 020: Render All Process-Owned Terminal Output Safely

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P0` |
| Workstream | Terminal safety / Composition |
| Depends on | 012 |
| Likely conflicts | 007, 009, 015 |
| Owner | `/root/implement_task_020` (`agent/task-020`) |

## Why This Matters

Task 012 made dynamic text inert at the Ratatui display seam, but Kinjo also owns
terminal output before the TUI starts and after it tears down. Those paths still
print configuration metadata, command metadata, and execution errors directly.
A command file or discovered value containing a control character can therefore
reach the terminal raw even though the in-TUI representation is safe.

This is a follow-up to task 012, not a reopening of it. The Ratatui boundary was
implemented and validated correctly; the midpoint review found a second output
boundary that the original source audit did not cover.

## Evidence

Re-validated on 2026-07-16:

- `src/ui/display.rs` protects strings passed through Ratatui, but it is not used
  by direct stdout/stderr or by the binary's returned error report.
- `src/lib.rs:75-79` prints lenient configuration warnings with `eprintln!`.
  Paths and parser diagnostics can include configuration-controlled text.
- `src/lib.rs:81-83` joins the raw prepared argv into a post-TUI hand-off error.
  Prepared argv may contain discovered network values by design.
- `src/lib.rs:141-150` prints `list-commands` name, description, mode, and raw
  command metadata directly. Its fixed-width formatting also measures scalar
  content rather than escaped terminal display columns.
- `src/plumber/exec.rs:70-78` embeds the attempted program name directly in
  spawn/exec errors.
- `src/plumber/template.rs:383-388` intentionally permits a placeholder in argv
  position zero. A discovered hostname can therefore become the program name;
  if execution fails, that untrusted value reaches Kinjo's error output.
- Clap parse/validation failures can exit and write their own diagnostic before
  the normal `run` return path (`ui::cli::parse` and
  `cli.discovery_options().unwrap_or_else(|err| err.exit())`). Those diagnostics
  may echo user-supplied option values and need the same untrusted-value audit.
- `src/main.rs` returns an eyre report to the runtime, so the final formatting
  boundary is not explicit or independently testable.

## Required Outcome

- Define one terminal-text boundary for every string Kinjo itself writes to
  stdout or stderr, including configuration warnings, `list-commands`, and the
  complete CLI/post-TUI error report or error chain.
- Reuse task 012's escaping semantics: C0, DEL, C1, and other Unicode controls
  render as visible inert notation; ordinary Unicode remains unchanged.
- `list-commands` escapes all dynamic columns and aligns them by terminal display
  width after escaping. Control sequences cannot change rows or column layout.
- Execution failures may identify the attempted command/program, but only through
  the safe terminal representation.
- Keep raw `PreparedCommand` argv unchanged all the way into `Command::new`,
  `.args`, and successful `exec`. Output safety must never alter what the user
  asked Kinjo to execute.
- Make the final process-owned error formatting boundary explicit enough to test
  with a writer/capture rather than relying on a human inspecting stderr.
- Preserve error causes and actionable context; escaping must not flatten the
  error chain into a vague generic failure.
- CLI usage/help behavior remains idiomatic, but any user-supplied value repeated
  in a parse or discovery-option error is rendered inert. Preserve framework
  styling only where it cannot become an untrusted control-character bypass.

## Implementation Constraints

- Put shared terminal escaping at a dependency-neutral layer. Do not make
  `plumber` depend on the UI renderer merely to format an error.
- Route human-readable output through writer-taking helpers so tests can inspect
  exact bytes without redirecting the test process's global stdout/stderr.
- Escape once at the final terminal boundary. Do not pre-escape stored command
  metadata, paths, error values, `Entry` fields, or argv.
- Audit all process-owned print/report paths after the refactor; a helper that
  leaves one direct dynamic `println!` or returned raw report is incomplete.
- Child-process output is not Kinjo-owned and must remain attached/forwarded as it
  is today. Do not sanitize or reinterpret a child program's stdout/stderr.

## Suggested Implementation Sequence

1. Add capture tests containing ESC, BEL, CR/LF, tab, DEL, C1, wide Unicode, and
   combining text in each direct-output source.
2. Move/generalize task 012's safe-text and display-width behavior into a neutral
   terminal presentation module without changing Ratatui behavior.
3. Extract writer-based warning and `list-commands` formatting helpers.
4. Audit/capture Clap parse and semantic-validation errors that can exit early.
5. Make `main` own safe final report emission and a deliberate exit status, or
   introduce an equivalent explicit report boundary.
6. Route exec/spawn context through that boundary while preserving raw argv.
7. Audit direct output macros and run both TUI and non-TUI regression suites.

## Non-Goals

- Sanitizing argv passed to a child or replacing argv execution with shell text.
- Filtering output produced by child programs after Kinjo launches them.
- Rejecting command files or discovery records solely because they contain
  control characters.
- Redesigning the `list-commands` columns beyond safe width-aware rendering.

## Acceptance Criteria / Definition of Done

- [x] No dynamic control character reaches any Kinjo-owned stdout/stderr path raw.
- [x] Config warnings and complete error reports are safe and retain their causes.
- [x] `list-commands` escapes every dynamic field and aligns by display columns.
- [x] A discovered control character in argv zero cannot escape through a failed
      hand-off report.
- [x] Successful execution receives byte-for-byte-equivalent raw argv values.
- [x] Child output behavior is unchanged.
- [x] A source audit finds no direct dynamic terminal output bypass.
- [x] Full validation passes.

## Required Tests

- Writer capture for config warnings containing control characters in a path and
  diagnostic message.
- CLI parse and discovery-option errors containing controls in the rejected value;
  assert safe diagnostics, preserved usage context, and the intended exit code.
- Writer capture for every `list-commands` column with control, CJK, emoji, and
  combining text; assert both inert bytes and display-column alignment.
- Failed exec/spawn where a placeholder-derived program name contains ESC/newline;
  assert safe final stderr while the executor received the original program.
- Multi-cause eyre report containing unsafe text at more than one cause level.
- Regression proving task 012's Ratatui buffer and raw matching/argv tests remain
  unchanged.

## Validation

```sh
cargo test --locked terminal
cargo test --locked plumber::exec
cargo test --locked ui::render
cargo run --locked -- list-commands --config-dir actions
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:** Moved task 012's escaping into the dependency-neutral
  `terminal` module and added Unicode display-width measurement there. The
  composition root now owns explicit stdout/stderr writers and exit codes. The
  binary uses `process_main() -> ExitCode`, while the source-compatible public
  `run() -> color_eyre::eyre::Result<()>` wrapper returns only a static summary
  after detailed diagnostics have crossed the safe writer boundary;
  configuration warnings, Clap diagnostics, structured discovery-option usage
  errors, `list-commands`, and complete eyre cause chains all cross the safe
  boundary exactly once. `list-commands` sizes its columns after escaping.
  Stored metadata, discovery errors, `Entry` values, and prepared argv remain
  raw. The executor and child stdio paths were not changed.
- **Tests added/updated:** Added nine writer/process regression tests covering
  C0, DEL/C1, ESC, BEL, CR/LF, tab, CJK, emoji, combining text, Clap stdout vs
  stderr and exit codes, structured discovery usage errors, multi-cause eyre
  reports, display-column alignment, and a placeholder-derived unsafe argv-zero
  failure and the separate library/binary entrypoint signatures. Updated CLI
  semantic-error assertions to use the new structured `DiscoveryUsageError`
  while preserving their flag/remedy expectations.
- **Documentation updated:** Marked task 020 done in this task and the backlog
  index. No command-file or keybinding interface changed, so no user
  configuration documentation required an update.
- **Validation evidence:** On 2026-07-16, `cargo test --locked terminal`,
  `cargo test --locked plumber::exec`, and `cargo test --locked ui::render`
  passed; `cargo run --locked -- list-commands --config-dir actions` rendered
  five aligned rules; `cargo fmt -- --check` passed; all-target/all-feature
  Clippy passed with `-D warnings`; default all-target tests passed 329/329 and
  all-feature all-target tests passed 344/344. A source audit found no dynamic
  direct-output macro, Clap `.exit()`, or returned-report bypass in `src/`.
- **Follow-ups:** Tasks 009 and 019 touch the CLI/composition surface and must
  preserve the non-exiting parse path plus the explicit safe process-output
  boundary when they are implemented.
