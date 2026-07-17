# Task 106 — One XDG config-path derivation

- **Priority**: P2 (maintainability)
- **Status**: ready
- **Depends on**: none
- **Likely conflicts**: none

## Problem

The "`$XDG_CONFIG_HOME`, else `$HOME/.config`" derivation exists twice, with
the same fallback logic implemented independently:

- `src/plumber/config.rs:105-125` (`config_dirs_from`) builds
  `<xdg>/kinjo/commands` or `<home>/.config/kinjo/commands`.
- `src/ui/keymap.rs:465-481` (`config_paths`) builds
  `<xdg>/kinjo/keybindings.toml` or `<home>/.config/kinjo/keybindings.toml`.

Both take `Option<OsString>` for the two env vars precisely so they can be unit
tested without touching process env — good — but the branching precedence is
duplicated. A change to how kinjo resolves its config home (say, honouring
`XDG_CONFIG_HOME` only when absolute, per the XDG spec) would have to be made in
two places and could drift.

## Goal

Extract a single helper that yields the kinjo config directory
(`<config-home>/kinjo`) from the two env-var inputs, and have both call sites
join their leaf (`commands` dir vs `keybindings.toml`) onto it. No behaviour
change.

## Suggested approach

- A small function, e.g. in a shared location both modules already depend on
  (or a new `src/config_home.rs` / a function in `src/lib.rs`), with signature
  like `fn kinjo_config_home(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf>`
  returning `<xdg>/kinjo` or `<home>/.config/kinjo`, `None` when neither is set.
- `config_dirs_from` appends `commands`; `config_paths` appends
  `keybindings.toml`. The existing public signatures of `config_dirs_from` and
  `config_paths` can stay as they are — only their bodies change.

Note the dependency direction rule (`CONTEXT.md`): discovery ← command rules ←
UI. `plumber` (command rules) must not depend on `ui`. So the shared helper
must live where **both** can reach it without `plumber` importing `ui`: a
crate-root module (`src/`) or a genuinely shared low-level module is fine; do
not put it under `ui`.

## Constraints

- No behaviour change: the existing precedence and path-shape tests in both
  modules must pass unchanged (`config_dirs_from_*`, `config_paths_*`).
- Respect the module dependency direction.

## Tests

- Existing tests in both modules stay green.
- Add a direct unit test for the shared helper covering: xdg present, home
  present, both present (xdg wins), neither (`None`).

## Definition of Done

- One derivation, two callers, no drift risk.
- Completion gate green.
