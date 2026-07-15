# Kinjo Review Backlog Context

This document is the shared context for the tasks in [`tasks/`](tasks/). Read it
before claiming a task. Each task repeats the facts needed for that change, but
this document records the cross-cutting language, invariants, and decisions that
must remain consistent across workstreams.

## Product and Data Flow

Kinjo is a terminal browser and launcher for DNS-SD services:

1. A discovery adapter observes DNS-SD announcements and emits entry events.
2. Entries are filtered and grouped into logical services, hosts, service types,
   or matching commands.
3. Loaded command rules match entry fields and prepare an argument vector.
4. The UI lets the user select a target and either forks the command or hands it
   off after restoring the terminal.

The current source is split across:

- `src/discovery/`: entry identity/grouping and the fake, mdns-sd, and optional
  zeroconf discovery adapters.
- `src/plumber/`: command-file loading, rule matching, interpolation, and process
  launch.
- `src/ui/`: CLI/configuration, keybindings, application state, filtering, and
  rendering.
- `src/lib.rs`: composition, signal handling, terminal lifetime, and exec handoff.

The dependency direction is discovery ← command rules ← UI. Preserve that
direction: discovery must not depend on command rules or UI, and command rules
must not depend on UI.

## Domain Language

- **Entry**: one discovered DNS-SD record with registration fields and any
  resolved host, addresses, port, and TXT data.
- **Registration**: the DNS-SD `(name, service type, domain)` identity advertised
  by a device.
- **Occurrence**: one observable instance of a registration from a discovery
  adapter. On mdns-sd this may be tied to a network interface. Occurrences must
  remain independently removable even when the UI groups them together.
- **Logical service**: one user-facing service produced by grouping compatible
  occurrences. Grouping is a presentation operation; it must not erase the
  occurrence identity needed for updates and removals.
- **Entry group**: a UI grouping of entries. Logical service, host, service type,
  and command groupings have different invariants and must not pretend that a
  first child's fields describe an aggregate.
- **Command rule**: a validated command configuration containing metadata,
  predicates, requirements, and a command template.
- **Candidate**: a concrete entry/address that satisfies a complete command rule
  and can be used to prepare an action.
- **Prepared command**: the final argument vector plus fork/execute mode. Network
  values may fill an existing argument but must never reshape the argument vector.
- **Picker**: a modal selection of an action, service, or concrete occurrence.
- **Discovery session**: the owned lifetime of a running discovery adapter,
  including its event receiver and cancellation/join behavior.

## Architecture Language

Use these terms consistently in design notes and completion records:

- **Module**: anything with an interface and implementation.
- **Interface**: everything callers must know, including invariants and errors.
- **Implementation**: behavior hidden behind a module's interface.
- **Depth**: leverage delivered behind a small interface.
- **Seam**: where an interface lives and behavior can be substituted.
- **Adapter**: a concrete implementation satisfying an interface at a seam.
- **Leverage**: capability callers receive from module depth.
- **Locality**: change, bugs, knowledge, and verification concentrated together.

Apply the deletion test to proposed abstractions: if deleting a module makes its
complexity vanish, it was shallow. A seam with one adapter is hypothetical; a
seam with two adapters is real. The discovery backend seam is real. The current
`RuleEngine` seam has only `Matcher` and is intentionally reviewed in task 015.

## Trust and Safety Model

- Service names, hostnames, TXT keys/values, addresses, ports, and discovery
  status details originate outside the process. Treat discovered text as
  untrusted.
- Preserve raw discovered values for matching and command interpolation, but
  render a terminal-safe representation that cannot emit control sequences.
- Command files and keybinding files are user-controlled local configuration.
  They are trusted to request programs, but malformed or impossible rules must
  fail validation before they can be selected.
- Interpolation occurs after tokenization. A discovered value may replace text
  inside one token; it must never add, remove, or split arguments.
- Real discovery failure must never create actionable sample devices. Sample
  records are permitted only when the user explicitly selects fake discovery.

## Required Invariants and Decisions

These decisions are defaults for every task. Do not silently revisit them inside
an implementation task; propose an ADR or request direction if new evidence
shows one is untenable.

### Discovery

- Occurrence identity and logical grouping are separate concerns.
- Removing one occurrence must preserve other live occurrences in the same
  logical service.
- A configured service type is either valid and honored exactly or rejected.
- The zeroconf adapter must reject a non-default domain while its dependency
  cannot honor domain selection. It must not silently browse another domain.
- Failed startup, stopped workers, and disconnected event channels are explicit
  discovery states. They do not fall back to sample entries.
- Explicit fake mode continues to stream sample records and remains suitable for
  development and smoke tests.

### Command Rules and Execution

- All predicates on one field are conjunctive for one concrete candidate. In
  particular, address predicates cannot be satisfied by different addresses.
- If multiple candidates prepare distinct argument vectors, the user must choose
  a target. Identical prepared commands may collapse without a redundant picker.
- Strict loading rejects malformed rules. Normal startup may continue with valid
  files and warnings. A live reload is transactional: any invalid file keeps the
  previously active rule set.
- `list-commands` is a real validator, not merely a TOML parser.
- Quoted empty arguments are valid. Unterminated quotes, dangling escapes,
  unknown fields/placeholders, empty commands, and malformed requirements are not.

### UI

- A picker must not execute stale cloned discovery data. The target is resolved
  and rematched against current state before preparation/execution.
- Host and service-type views show aggregate facts and children, not arbitrary
  values copied from the first entry.
- Picker selection remains visible at every supported terminal size.
- Help and footer hints reflect active keybindings. Same-mode collisions are
  configuration errors rather than hidden match-order precedence.
- Search is append-only: Backspace and Delete remove the last character. Escape
  and Enter close editing while retaining the active query; the clear action is
  separate.
- Render/layout calculations use terminal display width, not Unicode scalar count.

## Compatibility and Scope Constraints

- Preserve the documented command-file format unless a task explicitly rejects
  input that was malformed, unsupported, or semantically impossible.
- Keep both default and `zeroconf` feature builds working on supported platforms.
- Do not add a new seam solely for testing. Prefer testing through the module's
  real interface; private internal seams are acceptable when behavior genuinely
  varies.
- Update `README.md`, `docs/actions.md`, or `docs/keybindings.md` with any
  user-visible behavior or configuration change.
- Existing user changes in the worktree belong to the user. Never discard or
  overwrite unrelated edits.

## Baseline and Validation

At backlog creation, the worktree was clean and the following passed:

- `cargo fmt -- --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked --all-targets` (163 tests)
- `cargo test --locked --all-targets --all-features` (166 tests)

The full completion gate for implementation tasks is:

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

Use narrower targeted tests during development. Run the full gate before marking
a task done unless the task explains an environment limitation in its completion
record.

## Source Documentation

- [`README.md`](../../README.md): product behavior and architecture overview.
- [`docs/actions.md`](../actions.md): command-file interface.
- [`docs/keybindings.md`](../keybindings.md): keybinding interface.
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md): development and validation workflow.

No project ADRs existed when this backlog was created. Decisions above capture
the agreed defaults for these tasks; enduring decisions discovered during
implementation should be recorded explicitly rather than left only in code.
