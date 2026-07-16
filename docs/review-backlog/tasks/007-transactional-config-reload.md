# 007: Make Live Command Reload Transactional and Observable

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P1` |
| Workstream | Command rules / UI |
| Depends on | 005 |
| Likely conflicts | 006, 015, 020 |
| Owner | agent (branch `worktree-agent-ade224c3abe0b2da0`) |

## Why This Matters

Normal startup intentionally skips malformed command files so one shared/system
file cannot prevent Kinjo from launching. The same lenient behavior is unsafe for
SIGHUP reload: a temporary edit error can replace a working rule set with a
partial one. Reload warnings are reduced to a count and then lost, so the user
cannot inspect which files failed after the status line changes.

Because an active matcher already exists during reload, use an all-or-nothing
policy and preserve diagnostics.

## Evidence

- `src/plumber/config.rs:148-173`: lenient loading returns a partial builder plus
  warning strings.
- `src/ui/config.rs:11-24`: normal Run loading always uses the lenient path.
- `src/ui/app.rs:738-756`: reload replaces the matcher even when warnings exist.
- `src/ui/app.rs:748-754`: only warning count is retained in status.
- `src/lib.rs:53-68`: exit output contains startup warnings only; later reload
  diagnostics are unavailable.

Midpoint validation on 2026-07-16 confirmed the task is still needed and is no
longer dependency-blocked:

- `src/ui/config.rs:10-24` still returns a leniently built matcher with warnings
  for normal Run configuration.
- `src/ui/app.rs:1067-1085` still installs the returned matcher even when the
  same reload produced warnings, so a mixed valid/invalid overlay can replace a
  complete working ruleset with a partial one.
- `reload_reports_skipped_config_files` in `src/ui/app.rs` currently encodes the
  partial-swap behavior. Replace that expectation with the transactional policy;
  do not mistake the existing test for proof the defect is fixed.

## Required Outcome

- Preserve lenient startup semantics: load every valid command and retain detailed
  warnings for invalid files.
- Reload validates the complete configured overlay transactionally. If any file
  or directory entry fails, keep the currently active rule set unchanged.
- A fully valid reload atomically swaps rules, recomputes matches, and closes any
  picker derived from the old rules.
- Failed reload reports that the old rules remain active and retains full source
  paths/error messages for later inspection and terminal output at shutdown.
- A later successful reload clears or archives the current failure state according
  to one documented policy; default: retain only the latest reload diagnostics.
- Strict directory enumeration propagates entry errors rather than discarding
  them through `entry.ok()`.

## Implementation Constraints

- Use the compiled validator from task 005; do not create a second validation path.
- Do not mutate the active matcher until validation is complete.
- Keep SIGHUP handler work async-signal-safe; it continues to set only an atomic flag.
- Diagnostics must survive ordinary status updates.
- Do not fail/exit the TUI solely because a reload failed.

## Suggested Implementation Sequence

1. Add tests proving invalid reload preserves active matches/actions.
2. Separate startup and reload loading policies at the configuration interface.
3. Build a candidate matcher and diagnostics before swapping.
4. Store latest diagnostics in application/composition state and print on exit.
5. Make strict and lenient directory-entry error handling explicit.
6. Update SIGHUP/action documentation.

## Non-Goals

- File watching or automatic reload.
- Changing overlay precedence.
- Making initial startup strict.
- General status-history UI beyond retaining/printing actionable diagnostics.

## Acceptance Criteria / Definition of Done

- [x] Invalid sole or mixed reload leaves the old rule set and match results intact.
- [x] Successful reload swaps all rules together and invalidates old rule pickers.
- [x] Detailed latest reload diagnostics remain inspectable and are printed after
      terminal teardown.
- [x] Directory-entry read failures are errors/warnings under the correct policy.
- [x] Startup remains lenient and `list-commands` remains strict.
- [x] Documentation explains startup versus reload behavior.
- [x] Full validation passes.

## Required Tests

- Working matcher + malformed sole file + SIGHUP: working action remains.
- Mixed valid/invalid overlay: no partial swap.
- Valid overlay: atomic swap and recomputed command groups.
- Failure followed by success: latest-diagnostics policy.
- Injected directory-entry iterator error in strict and lenient modes.

## Validation

```sh
cargo test --locked ui::app::tests::reload
cargo test --locked ui::config
cargo test --locked plumber::tests::load
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
  - `src/ui/config.rs`: the two loading policies are now separate, named entry
    points. `load_matcher` is unchanged (lenient Run, strict `list-commands`).
    New `reload_matcher` is transactional: it compiles the whole configured
    overlay through the *same* validator and returns `Ok(Matcher)` only when
    nothing was rejected, otherwise `Err(Vec<String>)` of diagnostics. The
    candidate matcher is built here, before any caller can install it, so a
    rejected reload cannot touch the rules in force. Private
    `reload_matcher_from(dirs)` holds the policy so it is testable without the
    machine's system/user config dirs — same split as `plumber::config_dirs_from`.
    Rationale for lenient-startup vs strict-reload is recorded on both functions.
  - No second validation path: reload builds through
    `plumber::load_from_dirs_lenient`, which already reports *every* invalid file
    with its full path, and applies the all-or-nothing decision to its result.
    This is why a rejected reload names all bad files, not just the first.
  - `src/ui/app.rs`: `ConfigLoader` is now `Fn(&Cli) -> ReloadOutcome`, a new
    two-case enum (`Loaded(Box<dyn RuleEngine>)` / `Rejected(Vec<String>)`).
    Validity is decided outside the app; `reload_config` either swaps the whole
    matcher, clears diagnostics, closes pickers and recomputes, or touches
    nothing but the status and the retained diagnostics. The old
    `Ok((matcher, warnings))` shape — which installed a matcher *and* reported
    warnings, i.e. the partial swap — is gone by construction.
  - New `App::reload_diagnostics` retains the latest rejected reload's full
    source paths and messages, independent of the transient status line.
    Documented policy is latest-only: a success clears it, a later failure
    replaces it.
  - `src/lib.rs`: the composition root maps `ui::config::reload_matcher` into
    `ReloadOutcome`; diagnostics are taken out of the app before it is dropped
    and printed after terminal teardown. `write_config_warnings` generalized to
    `write_config_diagnostics(writer, label, ...)`, so startup skips print as
    `warning:` and a rejected reload as `reload rejected:` — different problems
    needing different action. Both still go through `terminal::text` escaping
    (task 020's boundary), since every message quotes untrusted file content.
  - `src/plumber/config.rs`: `toml_files_in` no longer discards entry errors via
    `entry.ok()`; it propagates them. Each policy then decides: `load_from_dirs`
    fails (now naming the directory, not a bare `Permission denied`), and
    `load_from_dirs_lenient` warns and skips the layer. Consequence, deliberate:
    a mid-directory entry error drops that layer with a warning instead of
    silently shortening it — and it now blocks a reload rather than quietly
    removing rules.
  - SIGHUP handling is untouched: the handler still only sets an atomic flag,
    and a failed reload never tears down the TUI.
- **Tests added/updated:**
  - `src/ui/app.rs` — **replaced** `reload_reports_skipped_config_files`, which
    encoded the partial-swap behavior, and `failed_reload_keeps_the_current_commands`
    with the transactional policy:
    `rejected_reload_keeps_the_working_rules_runnable` (malformed sole file: the
    action still *runs*, argv asserted, not just a command count),
    `a_mixed_valid_and_invalid_overlay_does_not_partially_swap`,
    `rejected_reload_retains_full_diagnostics_across_status_updates` (proves the
    detail survives an unrelated status update),
    `a_successful_reload_clears_the_previous_failures_diagnostics` and
    `a_second_rejected_reload_replaces_the_earlier_diagnostics` (latest-only
    policy, both directions), `a_valid_reload_swaps_every_rule_together_and_rebuilds_command_groups`
    (atomic swap visible in the rule-projecting command view), and
    `a_rejected_reload_leaves_an_open_picker_alone` (the mirror of the existing
    picker-invalidation test: nothing changed, so nothing is stale).
  - `src/ui/config.rs` — `startup_load_keeps_valid_commands_and_warns_about_invalid_ones`
    and `reload_refuses_an_overlay_with_any_invalid_file` are the same overlay
    under the two policies; plus `reload_accepts_a_fully_valid_overlay_in_precedence_order`,
    `a_rejected_reload_reports_every_invalid_file`, and
    `reload_ignores_directories_that_do_not_exist` (absence is not invalidity —
    a default install would otherwise never reload). Confined to loading policy;
    `cli.rs` and the config-dir ordering tests were left for task 009.
  - `src/plumber/mod.rs` — `an_unreadable_directory_fails_strictly_and_warns_leniently`.
  - `src/lib.rs` — `rejected_reload_details_are_labelled_apart_from_startup_warnings`;
    existing control-escaping test retargeted to `write_config_diagnostics`.
- **Documentation updated:**
  - `docs/actions.md`: new "Reloading While It Runs" section — the `SIGHUP`
    interface, the all-or-nothing rule, a table contrasting startup with reload,
    *why* they differ, what a rejection reports, the exit-output format, and the
    latest-only policy. "Parser Notes" now points at it rather than implying the
    startup policy is the only one.
  - `README.md`: the `SIGHUP` paragraph states the reload is transactional and
    links to the section.
- **Validation evidence:**
  - `cargo fmt -- --check` pass; `cargo clippy --locked --all-targets
    --all-features -- -D warnings` pass (no issues); `cargo test --locked
    --all-targets` 334 passed; `cargo test --locked --all-targets --all-features`
    359 passed. Targeted: `reload` filter 16 passed, `ui::config` 9 passed,
    `plumber::tests::load` 3 passed, `unreadable` 1 passed.
  - **Driven in the real TUI** (`scripts/drive-tui.sh`, `--backend fake` with a
    scratch `--config-dir` of two valid rules, `kill -HUP <pane pid>`; 200x24 so
    the right-aligned status is not truncated):
    - Rejected reload with a mixed overlay (`{hostnmae}` typo in one of two
      files): header still `commands 2`, the details pane still showed the *old*
      compiled template `ping -c 1 {hostname}`, and the status read
      `config reload rejected: 1 invalid config file(s); keeping the 2 comman…`.
      The TUI stayed up.
    - On quit, after terminal teardown, stderr carried the full detail:
      `reload rejected: /…/rules/ping.toml: unknown service field 'hostnmae' in
      'ping -c 1 {hostnmae}'; supported fields are …`.
    - Startup with that same broken file still launched, loading `ssh` and
      skipping the bad file (lenient startup preserved).
    - Fixing the file and adding a third rule, then SIGHUP: `commands 1-3/3` with
      all three rules present at once and status `reloaded 3 command(s)`.
    - Quitting after that success printed the startup `warning:` line but **no**
      `reload rejected:` line — the latest-only policy observed end to end.
- **Follow-ups:**
  - A rejected reload's detail is only readable on exit; the status line reports
    counts and truncates on narrow terminals. Task 011 owns modal viewports — a
    "show me the reload errors now" surface would fit there, but it is
    explicitly outside this task's non-goals (no general status-history UI).
  - A per-entry `read_dir` iterator error (as opposed to an unreadable
    directory) cannot be injected without a test-only seam, which CONTEXT.md
    forbids; the shared `io::Result` path both policies take is covered instead
    via directory permissions, skipped automatically when running as root.
