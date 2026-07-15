# 009: Make `--config-dir` Placement Unambiguous

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P1` |
| Workstream | CLI / Configuration |
| Depends on | — |
| Likely conflicts | 003 |
| Owner | Unclaimed |

## Why This Matters

`--config-dir` is defined at both the root command and `list-commands`, but the
parser reads only the subcommand matches when `list-commands` is present. A
syntactically accepted flag before the subcommand is silently discarded. This is
especially dangerous because the command still succeeds and appears to validate
configuration it never loaded.

## Evidence

- `src/ui/cli.rs:50-55`: the subcommand branch collects only from `sub`.
- `src/ui/cli.rs:112-120`: root defines repeatable `--config-dir`.
- `src/ui/cli.rs:155-162`: subcommand defines another copy.
- `src/ui/cli.rs:239-258`: the existing test explicitly expects the root value to
  disappear when a subcommand value is present.

Confirmed reproduction at review time:

```sh
kinjo --config-dir actions list-commands           # printed only the header
kinjo list-commands --config-dir actions           # listed five commands
```

## Required Outcome

- Define one logical global repeatable `--config-dir` option.
- For `list-commands`, placement before or after the subcommand is equivalent.
- Mixed/repeated placement preserves command-line occurrence order so overlays
  retain documented precedence.
- Normal Run behavior and `list-commands` default-directory policy remain unchanged:
  explicit list directories validate only those layers; no explicit directories
  use normal defaults.
- Accepted arguments are never silently ignored.

## Implementation Constraints

- Prefer Clap's global argument support or one well-tested merge path rather than
  duplicated definitions.
- Keep help output concise and show the option in the appropriate contexts.

## Suggested Implementation Sequence

1. Replace the existing disappearing-value test with before/after/mixed regressions.
2. Consolidate the Clap definition and collection path.
3. Verify overlay order through `ui::config` using actual temporary directories.
4. Update command examples if help placement changes.

## Non-Goals

- Changing directory overlay precedence.
- Changing lenient/strict load policies.
- Adding new configuration flags.

## Acceptance Criteria / Definition of Done

- [ ] Before-subcommand and after-subcommand invocations produce identical dirs.
- [ ] Mixed repeated flags preserve left-to-right overlay order.
- [ ] The confirmed reproduction produces the same bundled command set both ways.
- [ ] Run and list default-directory behavior remains covered.
- [ ] CLI help is accurate.
- [ ] Full validation passes.

## Required Tests

- One root-position directory.
- One subcommand-position directory.
- Several directories before, after, and mixed around the subcommand.
- Overlay replacement proves order, not just parsed vector equality.
- No explicit directory preserves existing defaults.

## Validation

```sh
cargo test --locked ui::cli
cargo test --locked ui::config
cargo run --locked -- --config-dir actions list-commands
cargo run --locked -- list-commands --config-dir actions
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
