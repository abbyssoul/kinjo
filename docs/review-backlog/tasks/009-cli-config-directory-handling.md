# 009: Make `--config-dir` Placement Unambiguous

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P1` |
| Workstream | CLI / Configuration |
| Depends on | — |
| Likely conflicts | 003, 020 |
| Owner | task-009 agent (branch `worktree-agent-aa89145600ebca200`) |

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

Reproduced again at midpoint validation on 2026-07-16 against the current
branch:

```sh
cargo run --locked --quiet -- --config-dir actions list-commands
# header only
cargo run --locked --quiet -- list-commands --config-dir actions
# five bundled rules
```

`src/ui/cli.rs:77-83` still reads only the subcommand context when
`list-commands` is present, so the accepted root-position value is still lost.

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

- [x] Before-subcommand and after-subcommand invocations produce identical dirs.
- [x] Mixed repeated flags preserve left-to-right overlay order.
- [x] The confirmed reproduction produces the same bundled command set both ways.
- [x] Run and list default-directory behavior remains covered.
- [x] CLI help is accurate.
- [x] Full validation passes.

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

- **Implemented:** `src/ui/cli.rs`. The two duplicated `--config-dir` definitions
  are replaced by one `config_dir_arg()` builder, registered at the root and on
  `list-commands`. `parse_from` now merges both contexts through a single
  `merge_config_dirs`, replacing the old branch that read only `sub` and silently
  dropped root-position values. `list-commands` gained a `long_about` stating the
  default-vs-explicit directory policy. No change to overlay precedence, load
  policies, or the flag set.

  Clap's `global(true)` was evaluated first, as the task's preferred option, and
  **rejected on evidence**: a global option does not append across levels. A
  subcommand occurrence *replaces* the root's values outright, and does so in the
  root matches too, so the root value becomes unrecoverable. Probed on clap 4.6.1:

  ```text
  global(true), argv: --config-dir a list-commands --config-dir b
    root ["b"]   sub ["b"]      # `a` is gone from both contexts
  ```

  That is the same silent discard this task removes, so the one merge path is
  used instead. Because the option is not global, Clap accepts it only in the
  context it was written in: the root matches hold exactly the occurrences before
  the subcommand and `sub` exactly those after it. Chaining root-then-sub is
  therefore command-line occurrence order by construction, not by coincidence.

- **Tests added/updated:** The defect-encoding test
  `parses_list_commands_config_dirs_from_subcommand_context` (which asserted the
  root value *disappears*) is removed. `src/ui/cli.rs` adds: single dir
  before/after equivalence; repeated dirs before/after equivalence; four mixed
  dirs preserving occurrence order; a root dir preceding a later subcommand dir;
  no-explicit-dir reporting none; and `--config-dir` documented in both help
  contexts. `src/ui/config.rs` adds (additively, no restructuring): overlay order
  through `load_matcher` over six real temp-directory argvs, plus run and
  `list-commands` default-directory coverage parsed from real argv.

  The order tests prove order, not membership: `base` and `overlay` both define
  `ssh`, so only the layer applied *last* survives, and reversing the command line
  reverses the winner. `base` also defines a rule `overlay` lacks, so dropping
  either directory loses a rule (`command_count` 2). Verified by mutation —
  reintroducing the old `merge_config_dirs(sub, None)` fails exactly the five new
  order/equivalence tests and nothing else. An earlier draft of the mixed case did
  *not* discriminate (loading only `[overlay]` yields the same winner) and was
  strengthened until it did.

- **Documentation updated:** `README.md` records that `--config-dir` may be
  written on either side of `list-commands`, repeated on both, and always
  overlays in command-line order. `docs/actions.md` is owned by task 007 and
  needed no correction: its "each `--config-dir` in the order given" was already
  the intended contract — it was simply untrue for root-position flags before
  this fix, and is now honored.

- **Validation evidence:** The reproduction is fixed; both placements print the
  identical five bundled rules (`diff` of the two outputs is empty):

  ```text
  cargo run --locked -- --config-dir actions list-commands   # 5 rules
  cargo run --locked -- list-commands --config-dir actions   # 5 rules, identical
  ```

  Full gate on `worktree-agent-aa89145600ebca200`:

  ```text
  cargo fmt -- --check                                          pass
  cargo clippy --locked --all-targets --all-features -- -D warnings
                                                                pass
  cargo test --locked --all-targets                             330 passed
  cargo test --locked --all-targets --all-features              355 passed
  ```

  Real app driven, not just tests: `scripts/drive-tui.sh run 'Tab Tab Down Down
  Enter'` reports `commands 5`, confirming the run path still loads `./actions`
  via `--config-dir`. Both `kinjo --help` and `kinjo list-commands --help` show
  the option.

- **Follow-ups:** None required. Optional: no `docs/adr/` directory exists yet;
  if one is created, the "not `global(true)`, one merge path" rationale above is
  a candidate to record there rather than only in the `config_dir_arg` doc
  comment. Task 015 may also revisit `parse_from`'s length, but the merge path is
  already isolated behind two small functions.
