# 007: Make Live Command Reload Transactional and Observable

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P1` |
| Workstream | Command rules / UI |
| Depends on | 005 |
| Likely conflicts | 006, 015 |
| Owner | Unclaimed |

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

- [ ] Invalid sole or mixed reload leaves the old rule set and match results intact.
- [ ] Successful reload swaps all rules together and invalidates old rule pickers.
- [ ] Detailed latest reload diagnostics remain inspectable and are printed after
      terminal teardown.
- [ ] Directory-entry read failures are errors/warnings under the correct policy.
- [ ] Startup remains lenient and `list-commands` remains strict.
- [ ] Documentation explains startup versus reload behavior.
- [ ] Full validation passes.

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
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
