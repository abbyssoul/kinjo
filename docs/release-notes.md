# Release Notes

## Next release

The first release since v0.1.6, and the outcome of a full review of Kinjo
(tracked as tasks 001–021 in `docs/review-backlog/`). Most of it is correctness
work, and several of the bugs it fixes could run a configured command against
the wrong host, a fabricated address, or data discovery had already retracted.

The breaking changes are almost all the same shape: Kinjo used to accept
something it could not honour and quietly do something else instead. It now
either honours the request exactly or refuses it, with an error saying why.

### Breaking changes

#### Real discovery failure no longer invents sample devices

When a real backend failed to start, every failure path silently started the
sample adapter and published fabricated `192.168.1.x` endpoints as ordinary,
actionable entries. Someone whose discovery had failed could launch a configured
SSH or browser action against an address that may well belong to a real,
unrelated machine on their network.

Discovery failure is now an explicit state: the list stays empty, and the top
bar and list body both report what failed and why. Sample records are only ever
produced by an explicitly selected sample backend.

#### Sample discovery is a backend

The `--fake-discovery` flag has been removed. Build or install Kinjo with the
off-by-default `fake` Cargo feature, then select the sample backend through the
same exhaustive interface as every other discovery backend:

```sh
cargo install kinjo --features fake
kinjo --backend fake
```

For source-tree development, use:

```sh
cargo run --features fake -- --backend fake
```

The old flag was removed rather than retained as a deprecated alias because an
alias would preserve two independent ways to select one backend, including
ambiguous combinations with `--backend`. Kinjo is still pre-1.0; making the
backend model unambiguous now is preferable to carrying that ambiguity into the
stable interface.

Library callers must make the same migration. `DiscoveryConfig::fake` and
`DiscoveryOptions::fake()` have been removed; enable the `fake` Cargo feature
and set `DiscoveryConfig::backend` to `DiscoveryBackend::Fake`. The selected
backend is now the only discovery-mode source of truth for both CLI and library
callers.

#### `--service-type` is validated, not reinterpreted

A malformed service type used to broaden observation instead of narrowing it: on
the zeroconf backend it silently became "browse every default type". It is now
validated once, before any adapter starts, and a malformed value is refused:

```console
$ kinjo --service-type bogus
error: invalid value for `--service-type`: `bogus` is not a DNS-SD service
type: a service type begins with `_`. Use a type such as `_ssh._tcp` or
`_dns-sd._udp`, or omit it to browse every service type
```

Accepted syntax is `_<name>._tcp` or `_<name>._udp`, where `<name>` is 1–15
ASCII letters, digits, and internal hyphens, begins and ends alphanumeric, has
no consecutive hyphens, and contains at least one letter. Omitting the flag
still means "browse every service type".

#### The `zeroconf` backend refuses a domain it cannot browse

`zeroconf` accepted `--domain` but could not pass it to its dependency, so it
browsed `local` regardless while appearing to honour the request. It now rejects
any domain other than `local` before starting. Empty, `local`, and `local.` are
canonicalised to `local`. The `mdns-sd` backend still accepts the custom domains
it supports.

#### Address predicates must be satisfied by one address

Each address predicate used to ask whether *any* address satisfied it, so two
predicates could be satisfied by two different addresses even though no single
address satisfied the whole rule — and execution could then fall back to any
address, including one that violated the predicates.

All predicates on one field are now conjunctive for one concrete candidate. A
rule with several address predicates that relied on the old behaviour will stop
matching; that combination never had an address that actually satisfied it.

#### `list-commands` is a real validator

`list-commands` was presented as a validator but was closer to a TOML parser:
unknown fields and placeholders, empty commands, malformed quoting, and
malformed requirement markers survived loading and only failed when the action
was chosen. It now compiles command files into validated rules and fails
strictly on all of those. Files that `list-commands` previously reported as fine
may now be reported as invalid — they were already broken; the failure just
arrived later.

Normal startup semantics are unchanged and remain lenient: valid files load, and
every invalid file is reported as a warning, so one bad system-wide file cannot
stop Kinjo from launching.

#### Discovered values are option-safe by default

Command templates no longer allow a discovered field to select `argv[0]`, or
to lead an argument before a literal `--`. The old shape `ssh {hostname}` could
turn a hostile hostname such as `-oProxyCommand=...` into an SSH option even
though it remained one argv token. Write `ssh -- {hostname}` (as the shipped
rules now do), give the token a safe literal prefix, or set
`action.allow_option_like_values = true` when the target program has no
terminator or the field is intentionally an option value.

This is a deliberate safety-breaking validation change. At normal startup an
older unsafe file is skipped with the existing warning and details-on-exit
diagnostic; `kinjo list-commands` reports the actionable migration error
strictly before launch.

#### Keybinding collisions are configuration errors

A keymap could bind two actions in one mode to the same key; match order then
silently decided which one worked and made the other unreachable. Colliding
bindings within a mode — including common-versus-mode collisions — are now
rejected at load with an error naming the key and both actions.

#### `--config-dir` before `list-commands` is now honoured

`--config-dir` was accepted before the subcommand and then silently discarded,
so `kinjo --config-dir ./rules list-commands` validated the *default*
directories and reported success for configuration it had never loaded.
Placement before or after the subcommand is now equivalent, and repeated flags
keep command-line order so overlay precedence is preserved. This can surface
real failures in command lines that previously appeared to pass.

### Discovery

- **Occurrences are tracked per interface.** The mdns-sd adapter exposes an
  interface index that Kinjo discarded, so two occurrences of one registration
  on different interfaces could overwrite each other, and a removal on one
  interface could delete the remaining live occurrence — losing addresses or a
  whole service. Occurrence identity and logical grouping are now separate:
  removing one occurrence leaves its live siblings alone.
- **Discovery has an explicit lifecycle.** The session now owns its adapter, its
  events, and its shutdown as one value, so a receiver whose producer has died
  is no longer indistinguishable from a quiet network. A finished sample stream,
  a failed startup, and a stopped browse are distinct, persistent states.

### Command rules and execution

- **Grouped rows ask which target to run against.** A host or service-type row
  can hold several services. Kinjo only offered a picker when the rule mentioned
  address or port, so `ssh {hostname}` on a service-type row would silently run
  against the lexically first host. Disambiguation is now based on what would
  actually be executed: if candidates prepare different commands, you choose;
  identical ones collapse without a pointless picker.
- **Pickers cannot run stale data.** Pickers held cloned snapshots that
  discovery events never invalidated, so you could confirm one after its target
  had disappeared or changed and run a command against a stale hostname or
  address. Pickers now hold identities and re-resolve against current discovery
  before preparing or executing; a target that is gone says so instead.
- **Config reload is transactional.** A SIGHUP reload with a temporary edit
  error could replace a working rule set with a partial one. A reload now
  validates the whole configured overlay first: if any file fails, the running
  rule set stays, and the full diagnostics are kept for inspection rather than
  reduced to a count and lost with the next status message.

### The TUI

- **Aggregate rows no longer claim a child's fields as their own.** Host and
  service-type rows copied service type, hostname, port, and TXT from an
  arbitrary first child, so a host's details could assert one arbitrary
  service/port/TXT set. Each view now shows only facts true of the whole row,
  and lists its children with their own.
- **Discovered text cannot drive your terminal.** Names and TXT data come off
  the network. They are now rendered as inert text, at every output path —
  inside the TUI, and in the process-owned output before it starts and after it
  exits. Matching and interpolation still use the raw values. Layout also
  measures display width, so wide and combining characters no longer skew
  alignment.
- **Pickers, modals, and help stay on screen.** A selected row could sit outside
  a modal's visible height, letting you run a target you could not see; resizing
  could hide a visible one. Every scrollable surface now derives its window from
  one calculation, so the selection is always visible and long content scrolls.
- **Hints reflect your keymap.** The footer, help, and popup hints were
  hard-coded, so after rebinding they told you to press keys that no longer
  worked. They now show the effective bindings, and an unbound action advertises
  nothing.
- **Search behaves as documented.** Delete now works like Backspace in the
  append-only editor. Escape and Enter close editing and keep the active query;
  the configured clear action remains the only full clear.
- **The activity indicator says what discovery is doing.** It only animates
  while a browse is actually running: a finished sample stream settles to a
  still `✓`, and a failed or stopped browse shows `✗`. Both used to go on
  spinning, implying background activity that had already ended. The empty-list
  message is rendered from the same fact, so the two cannot disagree. `r`
  (refresh) starts a new browse and returns the indicator to spinning. Neither
  ending retries by itself.
- **The type filter counts only what is currently advertised.** The `types n/m`
  chip counts the types on the link right now. A type that disappeared used to
  stay in the count, so a list showing nothing could still report `types 1/1`.
  Switching a type off remains a preference that survives the device
  disappearing and reappearing, but a type nobody advertises is now counted on
  neither side of the chip.
- **Details scrolling follows the selected row.** The pane keeps its scroll
  position while the same row stays selected, including as discovery re-reports
  it. Moving to another row starts that row's details from the top, and a row
  that shortens — or a terminal that grows — pulls the view back to the end of
  the content instead of leaving it past the end.

### Fixes

- **Kinjo no longer outlives the terminal it was started in.** Closing a
  terminal, ending a `tmux` session, or dropping an SSH connection left Kinjo
  running — orphaned, invisible, and spinning at 100% CPU on a terminal that no
  longer existed. It now exits, as it would have before it learned to reload on
  `SIGHUP`.

  The cause was that reload feature: handling `SIGHUP` also replaced its default
  action, which was to terminate. Kinjo now tells the two meanings of that one
  signal apart — a `SIGHUP` that arrives while the terminal is still there is a
  reload request, and one that arrives because the terminal has gone is a
  hangup. Reloading with `kill -HUP` is unchanged.

### Packaging and tooling

- **Nix flake.** Kinjo can be installed and run through Nix; see the Nix / NixOS
  section of `README.md`. The flake is checked and built in CI.
- **`kinjo --version`** prints the version.
- **CI now runs the application.** Nothing previously started the binary, so a
  change that rendered nothing or panicked on first draw would have passed every
  check. A smoke test drives the real TUI against the sample backend at a
  default and a narrow terminal size.
