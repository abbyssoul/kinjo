# Kinjo Review Backlog — Round 2

This directory turns the second full code review (2026-07-17, at `fdb6a68` on
`main`) into agent-ready implementation tasks, in the same format as
[round 1](../review-backlog/README.md). Round 1 is **complete**; nothing here
reopens it. This round was asked to focus on:

1. **refactoring opportunities** — duplication, readability, maintainability;
2. **bugs and data flow** — what inputs the app receives, from where, and what
   the risks are;
3. **performance** — where the cycles and allocations go, and what is worth
   optimising.

Read [`../review-backlog/CONTEXT.md`](../review-backlog/CONTEXT.md) before
claiming any task: the domain language, invariants, trust model, and completion
gate defined there all still apply. Tasks in this round are numbered from 101
so they can never be confused with round-1 tasks.

## Follow-up Validation Notes (2026-07-17)

A second independent pass validated this backlog at `4640aab`, after the
round-2 report itself was committed. The report is a good foundation, but it
should **not be implemented unchanged**. Each task now ends with a
"Follow-up validation note" that records the evidence and scope corrections
for the implementing agent. Those notes take precedence where they conflict
with the original task text; the original text is retained so the next reviewer
can see how the conclusion changed.

The validation baseline remained green:

```text
cargo fmt -- --check                                      pass
cargo clippy --locked --all-targets --all-features -- -D warnings
                                                            pass
cargo test --locked --all-targets                           416 passed
cargo test --locked --all-targets --all-features            436 lib + 7 integration
```

### Newly identified findings

1. **P1 — discovery resource use is unbounded before `App::records`.** The
   worker-to-UI event channel is unbounded (`src/discovery/worker.rs:96`),
   `drain_discovery` drains until empty (`src/ui/app.rs:420-460`), the mDNS
   liveness map is unbounded, and each probe cycle clones every key and starts
   every resolver through `join_all`. Task 107's proposed final-map cap cannot
   protect those resources or prevent a sustained producer from starving input
   and drawing. Task 107 must be expanded or split and treated as P1; task 108
   should be designed with it.
2. **P1 — DNS-SD TXT handling needs its own task and owner decision.**
   `txt_map` (`src/discovery/mdns.rs:364-377`) compares keys case-sensitively,
   makes exact duplicates last-wins through collection into a map, and converts
   values with `String::from_utf8_lossy`. DNS-SD keys are case-insensitive,
   first duplicate wins, and values are opaque bytes
   ([RFC 6763 sections 6.4-6.5](https://datatracker.ietf.org/doc/html/rfc6763#section-6.4)).
   This conflicts with `CONTEXT.md`'s promise to preserve raw discovered values.
   Before implementation, create a task that normalises valid keys at the
   adapter seam and decides whether the product is deliberately text-only or
   carries raw bytes through matching/interpolation.
3. **P2 — the public execution surface can panic.** `PreparedCommand::argv` is
   public, but `plumber::exec::exec` indexes `argv[0]`; a library caller can
   construct an empty command. Fold validation or construction-atomicity into
   task 109.
4. **P3 — terminal escaping still permits bidi visual spoofing.** ANSI/control
   injection is covered, but bidi formatting characters such as U+202E remain
   displayable and can visually reorder discovered labels. Record this as an
   accepted residual risk or extend task 110/terminal escaping deliberately.

### Corrections to prioritisation and ordering

- Task 101's option-injection risk is real, but its recommended documentation
  path does not make shipped rules safe by default. It cannot be both P0 and
  "done" through documentation alone; the owner must choose implementation or
  accepted risk. The trust model must also address interpolation into `argv[0]`.
- Tasks 102 and 103 are the strongest performance findings, but "largest
  first" has not been measured. Add a representative benchmark before ranking
  or declaring an optimisation successful.
- Task 106 describes a real XDG conformance bug, not merely duplication: empty
  `XDG_CONFIG_HOME` is unset and relative values are invalid under the
  [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/).
- Task 107 should no longer depend on task 102 as a whole. Network ingress
  bounds are safety work and should land before optional projection
  optimisation; final record-cap work may still coordinate with 102.
- Recommended order: resolve 101 and the TXT-data decision; implement bounded
  discovery ingress/probing (107 with 108); harden the public execution surface
  (109); benchmark and then implement 102/103; finish 106 and the lower-risk
  maintainability/polish tasks.

## Review Summary

### Overall verdict

The codebase is in good shape. Round 1's invariants held everywhere this
review looked: occurrence identity is separate from grouping, discovery
options are validated once at the start seam, untrusted text crosses
`src/terminal.rs` before reaching terminal bytes, interpolation cannot reshape
an argv, pickers revalidate against live records, and layout is a per-frame
value with no renderer write-back. **No P0 correctness bug was found in what
ships today.** The findings below are one trust-model gap that deserves a
decision, a set of real performance costs that are currently invisible only
because LANs are small, and maintainability debt concentrated in two places
(`App`'s modal state and `render.rs`'s repeated row-building patterns).

### Data flow and input risks

The program has six input sources. For each, what arrives and what guards it:

| Source | Enters at | Trust | Guards found | Residual risk |
|---|---|---|---|---|
| mDNS/DNS-SD network records | `discovery::mdns`, `discovery::zeroconf` | untrusted | structured identities (no joined-string keys); `terminal::text` escaping at every render/process-output boundary (verified across `render.rs` and `lib.rs`); compile-time argv tokenization + fuzz targets | **leading-dash option injection** (task 101); unbounded record growth (task 107) |
| CLI arguments | `ui::cli::parse_from` | untrusted | non-exiting parser; control chars escaped in error paths; discovery options validated once (`DiscoveryConfig::validate`) | none found |
| Command files (TOML) | `plumber::config` | trusted local config | full semantic validation at load; strict for `list-commands`, lenient at startup, transactional on reload | none found |
| Keybinding files | `ui::keymap` | trusted local config | unknown actions/keys rejected; collision and quit-reachability validation | a control character bound as a key reaches the footer unescaped (folded into task 110) |
| Environment (`PATH`, `XDG_*`, `HOME`, exe path) | `plumber::config`, `ui::keymap`, `plumber::exec` | semi-trusted | none needed beyond normal resolution | XDG derivation duplicated in two modules (task 106) |
| Signals / terminal events | `lib.rs::sighup`, crossterm | OS | SIGHUP hangup-vs-reload discrimination; release-key filtering; clicks bounded by the layout snapshot | `run()` is not re-entrant as a library call (task 109); SUPER/META-modified keys type into search (task 110) |

The one finding worth escalating: the injection barrier stops a discovered
value from **adding or splitting arguments**, but not from being an argument
that the launched program parses as an **option**. A hostile device
advertising hostname `-oProxyCommand=…` against a `ssh {hostname}` rule
produces `argv = ["ssh", "-oProxyCommand=…"]` — one token, exactly as
designed, and still an injection at the semantic level. Task 101 records the
options and asks for an owner decision.

### Bugs

No shipping-path correctness bug was found. Three lesser defects:

- **Probe cycles starve browse events** (`src/discovery/mdns.rs:215-222`): the
  nested `select!` that runs a liveness probe cycle does not poll
  `browser.recv()`, so browse events queue for up to `PROBE_TIMEOUT` (5s)
  every 30s once services are being probed. Task 108.
- **`kinjo::run()` is not re-entrant** (`src/lib.rs:72`, `src/lib.rs:234`):
  `color_eyre::install` fails on a second call in one process, and the SIGHUP
  `OnceLock` keeps routing reloads to the *first* app's flag. The binary never
  hits this; a library consumer calling `run()` twice does. Task 109.
- **Modifier handling is incomplete** (`src/ui/app.rs:1428-1436`):
  `typed_char` excludes CONTROL and ALT but not SUPER/META/HYPER, which
  terminals using the kitty keyboard protocol do report — an unbound
  Super-chord types its letter into search. Task 110.

### Performance

The event loop redraws every ≤120ms and fully recomputes the projection on
every tick that saw a discovery event. Costs found, largest first:

1. **The recompute clone cascade** (`src/ui/app.rs:502-539`). One recompute
   clones every record out of the `BTreeMap`, clones the survivors again in
   `filter.apply`, buckets them **four times** (once per tab count via
   `browse_row_count`, which builds string-cloning `EntryGroupId`s and throws
   them away, then once more in `browse_groups`, which clones every record
   into its bucket), then runs the rule engine per row — where every match
   clones the whole `CommandConfig` and `distinct_targets` prepares every
   candidate with an O(n²) seen-list. During an active browse this pipeline
   runs on nearly every tick; while typing in search it runs per keystroke,
   with a fresh `searchable_text()` allocation per record per keystroke.
   Task 102.
2. **Details rows are built twice per tick, and frames are drawn that cannot
   have changed** (`src/ui/app.rs:405-414`, `src/ui/render.rs:456-463` and
   `465-484`). `update_layout` builds the full `Vec<Row>` of the details pane
   just to count its lines; `render_details` builds the identical rows again.
   Independently, the loop redraws at ~8Hz forever — including when the
   session has ended, no search cursor is blinking, and no event arrived.
   ratatui diffs the buffer so terminal I/O is cheap, but the row/span
   allocation is real, and `last_seen.elapsed()` (`src/ui/render.rs:573`)
   makes even "still" frames differ. Task 103.
3. **The command view doubles down** (`src/ui/app.rs:709-760`):
   `recompute_command_groups` clones every `CommandConfig` out of the engine,
   then clones the whole `EntryGroup` once per matching command. Folded into
   task 102.
4. **Input bursts are processed one event per frame**
   (`src/ui/app.rs:374-393`): each key or wheel step pays a full layout +
   draw before the next is read, so a mouse-wheel burst or held key lags
   behind the hand. Folded into task 110.

None of this is misbehaviour at today's scale — a LAN with a few hundred
services works fine. But the cost is quadratic-ish in places, entirely
allocation-bound, and all of it sits on the interactive path. Task 107 covers
the adversarial end of the same spectrum: a hostile or merely huge network can
grow `records` without bound, and the recompute pipeline amplifies it.

### Refactoring opportunities

- **`App`'s modal state is seven parallel fields** (`src/ui/app.rs:184-232`):
  `mode`, `picker_anchor`, `action_matches`, `action_index`, `pending_action`,
  `instance_index`, `service_picker_index` — with invariants like
  "`pending_action` is `Some` iff mode is `InstancePicker`" enforced by
  discipline. `reconcile_action_pickers` (617-689) is the direct cost of this
  shape, and the three near-identical picker key handlers are its shadow.
  Task 105.
- **`render.rs` repeats three patterns**: the `├─`/`└─` tree-branch row loop
  appears three times (`render.rs:556-577`, `646-690`, `947-966`); the
  description fallback chain is hand-rolled four times with **two different
  precedence orders** — action-first in the browse details and action picker
  (779-785, 1144-1152), metadata-first in the command view (858-863, 914-918).
  That difference is intentional per `docs/actions.md` ("`action.description`
  … is shown in the action picker"), which is exactly why it should be two
  named helpers instead of four inline chains a future edit can silently
  unify the wrong way. Task 104.
- **XDG path derivation exists twice** (`src/plumber/config.rs:105-125`,
  `src/ui/keymap.rs:465-481`): same `XDG_CONFIG_HOME`-else-`HOME/.config`
  fallback, two implementations that can drift. Task 106.

Noted but deliberately **not** tasked (cost exceeds value today):
`EntryGroup` stores `label` beside the `facts` it is derived from (constructed
together, cannot drift in practice); `Entry` doubles as "candidate with
narrowed addresses" in `CommandConfig::candidates` (a `Candidate` newtype
would be clearer but touches every engine path); `lib.rs::write_commands`
spells its three column widths out by hand.

## Agent Workflow

Identical to round 1 — see [`../review-backlog/README.md`](../review-backlog/README.md#agent-workflow).
In short: claim a `ready` task whose dependencies are `done`, re-verify its
evidence on the current branch, set it `in-progress` with an owner, stay in
scope, add regression tests through the module's interface, run the completion
gate, record completion, update this index.

Status values: `ready`, `blocked`, `in-progress`, `done`.

## Task Index

| ID | Priority | Status | Task | Depends on | Likely conflicts |
|---|---|---|---|---|---|
| 101 | P0 | done | [Leading-dash option injection from discovered values](tasks/101-option-injection-from-discovered-values.md) | — (decision: ADR 0003) | 102 |
| 102 | P1 | done | [Recompute pipeline: remove the clone cascade](tasks/102-recompute-clone-cascade.md) | — | 103, 105, 107 |
| 103 | P1 | done | [Render pipeline: build details once, skip dead frames](tasks/103-render-once-per-tick.md) | 102 | 102, 104, 110 |
| 104 | P2 | done | [Render duplication: tree rows and description precedence](tasks/104-render-duplication.md) | — | 103 |
| 105 | P2 | done | [Consolidate App's modal/picker state](tasks/105-picker-state-consolidation.md) | 102 | 102, 103, 110 |
| 106 | P2 | done | [One XDG config-path derivation](tasks/106-shared-xdg-derivation.md) | — | — |
| 107 | P1 | done | [Bound hostile record growth](tasks/107-bound-record-growth.md) | — (decision: ADR 0005) | 102 |
| 108 | P1 | done | [Probe cycles must not starve browse events](tasks/108-probe-starvation.md) | 107 | 107 |
| 109 | P2 | done | [Library re-entrancy of `run()`](tasks/109-library-reentrancy.md) | — | — |
| 110 | P2 | done | [Input handling polish: modifiers, bursts, label escaping](tasks/110-input-polish.md) | — | 103, 105 |
| 111 | P1 | done | [Portable DNS-SD TXT semantics](tasks/111-portable-dns-sd-txt.md) | — (decision: ADR 0004) | 102 |
| 112 | P1 | done | [Support TXT keys longer than nine bytes](tasks/112-long-txt-keys.md) | 111 | 111, 102 |
| 113 | P2 | done | [Retire the dead `prepare` failure path](tasks/113-retire-dead-prepare-failure-path.md) | 101 | — |

Priority meanings (unchanged from round 1):

- **P0**: correctness or safety; schedule before feature work.
- **P1**: validation, UX correctness, or a refactor needed by later work.
- **P2**: maintainability/deepening after behavior has regression coverage.

## Completion (2026-07-17)

All twelve tasks are done. ADRs 0003-0005 record the enduring safety/data-flow
decisions; task 112 visibly amends ADR 0004's mistaken nine-byte TXT-key clause.

The representative recompute workload (2,000 entries, 24 rules, all views and
active search) fell from 776.871 ms to 234.750 ms for 12 projections: 64.739 to
19.562 ms per projection, a 69.8% reduction. Dirty rendering, one per-frame
details projection, bounded input/event draining, and construction-atomic modal
state reduce the work outside that measured pipeline as well.

Fake-backend smoke checks covered lists, tab counts, details, command view, and
instance picker behavior. A live mDNS smoke run discovered and rendered the
local network successfully. The completion gate and the command-preparation
fuzz target build passed; exact final counts are recorded under the gate below.

## Workstreams and Ordering

This is the order the now-complete implementation followed after the owner
decisions were recorded:

```text
Safety (decided; do first):
               101   ADR 0003 — option-safe templates, literal argv[0]
               107 + 108   ADR 0005 — bounded ingress + probe concurrency.
                     One piece of work in src/discovery/; design together.
               111 → 112   ADR 0004 — portable TXT semantics, then lift the
                     9-byte key ceiling. 112 amends ADR 0004; it is split out
                     only because 111 was already in flight when the finding
                     landed. Same module — serialize, do not parallelize.
               109   public-surface hardening (empty-argv panic) — independent

Performance (measure first, per task 102's follow-up note):
               102 → 103        (both reshape app.rs/render.rs; serialize)

Maintainability: 105 (after 102 — same file, and 102 may dissolve some fields)
                 104 (render.rs; coordinate with 103)
                 106 (independent; XDG conformance, not just duplication)

Polish:        110 (app.rs/render.rs; coordinate with 103 and 105)
```

Two dependency edges from the original plan are **removed**: 107 no longer
depends on 102 (ingress bounds are safety work and must not wait on optional
projection optimisation), and 108 is no longer independent (ADR 0005 makes it
the probe half of 107's defence).

The bidi/U+202E finding was closed in task 110: the terminal presentation Module
now escapes bidi formatting characters visibly, alongside C0/C1 controls.

## Validation Baseline

Recorded on 2026-07-17 at `fdb6a68` (head of `main`, round-1 backlog closed),
before any round-2 change:

```text
cargo fmt -- --check                                      pass
cargo clippy --locked --all-targets --all-features -- -D warnings
                                                            pass
cargo test --locked --all-targets                           416 passed
cargo test --locked --all-targets --all-features            436 lib + 7 integration
```

## Completion Gate

Unchanged from round 1: every task must satisfy its own DoD and finish with

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

Final result on 2026-07-17:

```text
cargo fmt -- --check                                      pass
cargo clippy --locked --all-targets --all-features -- -D warnings
                                                            pass
cargo test --locked --all-targets                           436 + 6 integration
cargo test --locked --all-targets --all-features            462 + 7 integration
cargo check --manifest-path fuzz/Cargo.toml --bin prepare_command
                                                            pass
```

The one ignored unit test is the manual release benchmark recorded in task 102.

Tasks changing parsing, entry identity, grouping, or interpolation must
consider the fuzz targets in `fuzz/fuzz_targets/` (see `CONTRIBUTING.md`).
Task 101 in particular must extend `prepare_command`'s oracle if it changes
what a prepared argv may contain. Tasks that change what the user sees must be
driven with `scripts/drive-tui.sh` and looked at, per the round-1 context.

## Backlog Maintenance

Same rules as round 1: tasks stay self-contained; newly discovered scope
becomes a follow-up task rather than a completion-note aside; enduring
decisions become ADRs under `docs/adr/` — task 101's resolution in particular
is expected to produce one.
