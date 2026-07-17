# Contributing

Thanks for helping improve `kinjo`. This project is a Rust terminal UI for
browsing DNS-SD services and launching configured actions.

## Development Setup

Install the Rust toolchain from `rust-toolchain.toml`:

```sh
rustup show
```

On Debian or Ubuntu, install the native dependencies used by discovery and the
terminal UI stack:

```sh
sudo apt-get update
sudo apt-get install -y clang libavahi-client-dev libxcb-shape0-dev libxcb-xfixes0-dev xorg-dev
```

For real mDNS discovery, the Avahi daemon must be available on the system. For
UI and command development, you can run without Avahi by enabling the
off-by-default `fake` feature and selecting that backend.

## Local Commands

Build the project:

```sh
cargo build --locked
```

Run tests:

```sh
cargo test --locked
```

Check formatting:

```sh
cargo fmt -- --check
```

Run lint checks:

```sh
cargo clippy --locked --all-targets -- -D warnings
```

Run the TUI with sample records:

```sh
cargo run --features fake -- --backend fake
```

Validate command configs:

```sh
cargo run -- list-commands
```

Validate command configs from a specific directory:

```sh
cargo run -- list-commands --config-dir ./actions
```

Build a Debian package if `cargo-deb` is installed:

```sh
cargo deb
```

## Driving the TUI

Tests assert what the app computes. They cannot tell you what a person sees, and
a rendering, keybinding, or picker change is only really verified by running it.
`scripts/drive-tui.sh` runs the app in a detached tmux pane, sends keys, and
prints the rendered screen — no live network needed, and no terminal of your own
tied up:

```sh
scripts/drive-tui.sh run 'Tab Tab Down Down Down Enter'   # one shot
scripts/drive-tui.sh --help                               # keys, options
```

It builds with the `fake` feature and defaults to `--backend fake --config-dir
actions`: the sample backend plus the bundled rules, which is the reproducible
way to exercise the UI. Pass other runtime arguments after `--`. For a longer
investigation, hold a session open and look between steps:

```sh
scripts/drive-tui.sh start
scripts/drive-tui.sh keys Tab Tab
scripts/drive-tui.sh shot
scripts/drive-tui.sh stop
```

Set `KINJO_COLS`/`KINJO_ROWS` to check a size — a narrow or short terminal is
where layout bugs live:

```sh
KINJO_COLS=60 KINJO_ROWS=18 scripts/drive-tui.sh run 'Down'
```

The sample records are chosen so the UI's real behavior is reachable: SSH is
advertised on two hosts, so the `_ssh._tcp` service-type row asks which host to
act on, and one service carries several addresses. If you change them, keep them
able to demonstrate what the app does.

CI runs shallow assertions against the same driver at 100×30 and 60×18. Run the
exact smoke check locally with:

```sh
scripts/smoke-tui.sh
KINJO_COLS=60 KINJO_ROWS=18 scripts/smoke-tui.sh
```

The smoke check waits with a bounded timeout, verifies stable semantic text and
a view change, then requires a clean quit. On failure it prints the captured
screen so the CI log contains the evidence rather than only an assertion name.

## Fuzzing

The parser and the discovery entry model are exercised by [`cargo-fuzz`]
(libFuzzer) targets in `fuzz/`. They require a nightly toolchain; the helper
script installs `cargo-fuzz` if needed:

```sh
scripts/fuzz.sh            # run every target for 60s each
scripts/fuzz.sh 300        # 5 minutes per target
scripts/fuzz.sh 120 parse_command   # one target
```

Targets. Besides panic-safety, each target asserts semantic properties
(round-trips, id/grouping invariants, the argument-injection barrier) — a
plain "doesn't crash" oracle cannot see the wrong-output bugs that spaces or
separator characters in service names and command values tend to cause:

- `parse_command`: the command/action file parser (`MatcherBuilder::add_str`)
  must parse or error on arbitrary bytes, never panic.
- `command_roundtrip`: arbitrary field values serialized into a well-formed
  command file must load back unchanged.
- `prepare_command`: tokenizing and interpolating action templates; untrusted
  field values must never add, remove, or reshape argv entries.
- `discovery_entry`: building, id-resolving, and grouping arbitrary entries;
  id equality must match field-tuple equality and grouping must preserve
  entries and per-group field agreement.
- `decode_dns_sd`: the DNS-SD decimal-escape name decoder; escape-free input
  is identity and reference-encoded bytes round-trip.

CI runs a short soak on every push/PR and a longer one on a weekly schedule
(`.github/workflows/fuzz.yml`); any crash inputs are uploaded as artifacts.

[`cargo-fuzz`]: https://github.com/rust-fuzz/cargo-fuzz

## Project Layout

- `src/discovery/`: the discovery layer — produces `Entry` values from a
  `DiscoverySession` that owns the running adapter (mDNS plus the feature-gated
  fake backend).
- `src/plumber/`: the rules engine — command-file parsing, matching, and
  execution. `Matcher` is the engine kinjo ships; the public `RuleEngine` trait
  is the one seam a dependent crate can substitute, per
  [ADR 0001](docs/adr/0001-rule-engine-is-a-supported-extension-point.md). If you
  change that trait, you are changing published API — `tests/rule_engine_extension.rs`
  implements it the way an outside crate would and will tell you.
- `src/ui/`: CLI parsing, config/keymap loading, app state, and rendering.
- `src/lib.rs` / `src/main.rs`: the library composition root and its thin binary.
- `fuzz/`: `cargo-fuzz` targets; `scripts/fuzz.sh`: the fuzz runner.
- `scripts/drive-tui.sh`: runs the TUI in tmux and prints what it renders.
- `actions/`: bundled command examples installed as system command defaults in
  the Debian package.
- `docs/actions.md`: custom command file reference.
- `docs/keybindings.md`: keybinding configuration reference.
- `docs/releasing.md`: protected two-stage release and recovery runbook.
- `docs/adr/`: architectural decisions and the reasoning behind them. Read the
  relevant one before reversing a design choice that looks arbitrary.
- `.github/workflows/`: CI, fuzzing, and release packaging workflows.

## Contribution Guidelines

Keep changes focused and easy to review. If a change affects command files,
keybindings, or user-facing behavior, update the matching README or `docs/`
page in the same pull request.

Before opening a pull request, run:

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

When reporting bugs, include:

- operating system and version
- how `kinjo` was installed
- command used to run it
- whether `--backend fake` works in a build with the `fake` feature
- relevant command or keybinding config snippets
- the full error output
