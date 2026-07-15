# 010: Render Mode-Aware Aggregate Views

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P1` |
| Workstream | UI / Discovery grouping |
| Depends on | 001 |
| Likely conflicts | 006, 008, 011, 012, 013, 014, 015 |
| Owner | agent-a3a8a1ba3c4773d15 |

## Why This Matters

Logical service, Host, and Service Type tabs all reuse `EntryGroup`, which copies
service type, hostname, port, and TXT fields from its sorted first child. In an
aggregate Host or Service Type row those fields are not invariants. The details
pane can therefore claim that a host has one arbitrary service/port/TXT set, or
that a service type belongs to one arbitrary host.

Deepen the browse projection so each grouping mode exposes only facts valid for
that mode. This improves locality: render should not need to infer aggregate
semantics from a generic data bag.

## Evidence

Re-verified on `dd872b7` before implementing. Every claim held; the line numbers
had moved (tasks 001 and 012 landed in between), so they are restated here:

- `src/discovery/entry.rs:337-347` (was 257-267): one `EntryGroup` shape stores
  scalar metadata.
- `src/discovery/entry.rs:373-385` (was 278-301): those scalars are copied from
  `instances[0]` after sorting.
- `src/ui/app.rs:240-266` (was 243-253): all non-command grouping modes use the
  same groups.
- `src/ui/render.rs:266-437` (was 260-365): row/details render scalar fields as
  facts regardless of grouping mode.

Two further defects found while verifying:

- `group_entries` bucketed into a `HashMap` and sorted only by `(label,
  service_type)`, so rows with duplicate labels *and* equal types had no total
  order and could shuffle between recomputations.
- The Services tab counted `records.len()` (occurrences, not logical-service
  rows) and the Hosts tab counted only resolved hostnames, omitting the
  unresolved row it displays. Both contradicted the required count definitions.

## Required Outcome

- Logical-service rows/details show fields invariant across all occurrences and
  list concrete occurrences. Differing TXT/address data is displayed per
  occurrence (or explicitly as mixed), never copied from the first occurrence.
- Host rows/details show the hostname, counts, and child services; they do not show
  one arbitrary service type, port, domain, or TXT set as host-wide data.
- Service Type rows/details show the type, counts, and child services/hosts; they
  do not show one arbitrary host, port, or TXT set as type-wide data.
- Unresolved hosts remain grouped/labeled distinctly without colliding with a
  literal sentinel hostname.
- Command grouping retains its command-specific projection.
- Matching/invocation continues to operate on concrete child entries, not display
  aggregates.
- Row identity and ordering are deterministic and total, including duplicate labels.
- Counts have exact meanings: the Services tab counts logical-service rows; Hosts
  counts resolved host rows plus one unresolved row when present; Types counts
  distinct service types; Commands counts configured rules. Host details show
  logical-service and occurrence counts; Type details show logical-service,
  resolved-host, and occurrence counts.
- Same-host filtering is available only when the selected projection has one
  invariant hostname (Logical Service or Host). Service Type and Command views
  report it unavailable instead of using a first child's hostname.

## Implementation Constraints

- Prefer mode-specific projection types or an enum with mode-specific data over
  optional fields whose meaning callers must infer.
- Keep discovery entries/grouping reusable without importing rendering types.
- Do not duplicate raw entry collections in several parallel structures without
  one owner maintaining their invariants.
- Keep this task focused on projection semantics; task 015 owns final App/render
  encapsulation after all behavior work lands.
- Preserve selection by structured row identity across recomputation.

## Suggested Implementation Sequence

1. Add heterogeneous Host and Service Type rendering/model tests.
2. Define mode-aware row/detail projections with explicit child lists and IDs.
3. Move aggregate construction behind a browse-model interface used by render.
4. Update render paths to consume only valid fields for the active projection.
5. Add total ordering using structured row identity as the final key.

## Non-Goals

- Changing how command predicates match concrete entries.
- Redesigning the visual theme.
- Picker scrolling; task 011 follows this projection work.
- Completing the broad App refactor; task 015 finishes architecture cleanup.

## Acceptance Criteria / Definition of Done

- [x] Heterogeneous Host details contain all child services and no arbitrary
      representative metadata.
- [x] Heterogeneous Service Type details contain all child hosts/services and no
      arbitrary representative metadata.
- [x] Logical-service and command views retain their valid details/actions.
- [x] Differing logical-service TXT/address values appear per occurrence or as
      explicitly mixed, never as arbitrary group-wide values.
- [x] Tab/detail counts follow the exact definitions above.
- [x] Same-host filtering is disabled outside invariant-host projections.
- [x] Matching/invocation targets concrete children correctly.
- [x] Group ordering is stable across input permutations/recomputations.
- [x] Rendering tests cover empty, unresolved, homogeneous, and heterogeneous data.
- [x] Full validation passes.

## Required Tests

- One host offering SSH and HTTP on different ports/TXT values.
- One service type offered by several hosts with different metadata.
- Duplicate labels differing by structured identity have deterministic order.
- Unresolved-host aggregate versus literal `"<unresolved host>"` hostname.
- Invoking a command from each aggregate uses a concrete child.

## Validation

```sh
cargo test --locked discovery::entry
cargo test --locked ui::app
cargo test --locked ui::render
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
  - `src/discovery/entry.rs`: replaced the one-shape `EntryGroup` with a
    mode-aware browse projection.
    - `BrowseMode` (LogicalService / Host / ServiceType) is now the projection
      input. `GroupingMode::browse_mode()` returns `None` for `Command`, which
      lists rules rather than projecting entries — this deletes the old "if
      Command ever reaches `group_entries` it acts like logical service"
      fallback instead of documenting around it.
    - `EntryGroupId` is a structured enum (`LogicalService{..}` / `Host(HostKey)`
      / `ServiceType`) instead of a formatted `String`. `HostKey::Unresolved` is
      a variant, so the unresolved row is an *identity*, not a sentinel label,
      and `join_key`/`hostname_key` (which existed only to make string keys
      unambiguous) are gone.
    - `GroupFacts` is one variant per mode: `LogicalService{name, service_type,
      domain, hostname, port}`, `HostAggregate{hostname}`,
      `ServiceTypeAggregate{service_type}`. An aggregate has no service-type,
      port, or TXT *field to read*, so the old bug is unrepresentable rather
      than merely unwritten.
    - The key fix: `row_of()` builds a row's identity and its facts from the
      same fields, and `browse_groups` takes the facts from the bucket key. A
      row's facts are therefore true of every occurrence by construction — no
      value is copied from `instances[0]` anymore.
    - `RowHost` (`Resolved`/`Unresolved`/`Varies`) and `RowServiceType`
      (`Invariant`/`Varies`) let callers ask what a row can *truthfully* say,
      rather than inferring meaning from an `Option`. `RowHost::Varies` is what
      makes the same-host filter's unavailability a type-level fact.
    - `TxtValue::{Shared, Mixed}`: TXT is not part of the logical-service key, so
      occurrences may disagree; disagreement is now stated, not resolved by
      picking one occurrence's value.
    - Derived-on-demand `child_services()` (a `ChildService` display summary,
      *not* a second copy of the entries), `logical_service_count()`,
      `resolved_host_count()`, `occurrence_count()`, `service_types()`, `txt()`.
      The row's `instances` stay the single owner of the entries.
    - `browse_row_count()` counts a projection's rows without building them, so
      tab counts and lists share one definition.
    - Row order is now total: `(label, EntryGroupId)`, bucketed via `BTreeMap`.
      Occurrences within a row sort by visible fields then `EntryId`.
  - `src/ui/app.rs` (confined to the grouping/projection area): `tab_counts`
    computed per recompute via `count_tabs`; selection preserved by structured
    `id()`; `toggle_same_host_filter` rewritten to branch on `RowHost` and to
    report unavailability in the service-type and command views (clearing an
    active filter still works from any view).
  - `src/ui/render.rs`: `service_row` and the details pane now dispatch on
    `GroupFacts`. Host rows show `N svc` + the types actually offered; type rows
    show `N svc` + resolved host count. Host details list every child service
    with its own type/port; type details list every child host. All discovered
    text still goes through task 012's `display::text`. Renamed the "instances"
    detail section/badge to "occurrences" to match the domain language.
  - `src/plumber/mod.rs`: `group.instances` → `group.instances()`. The rule
    engine still matches concrete entries only; no predicate behavior changed.

- **Tests added/updated:** 20 net new (184 → 204).
  - `discovery::entry`: heterogeneous host row lists both services and exposes
    `RowServiceType::Varies`; heterogeneous service-type row lists every host and
    exposes `RowHost::Varies` with an exact resolved-host count; duplicate labels
    keep one order across input permutations; the unresolved row vs. a literal
    `"<unresolved host>"` hostname (distinct ids, deterministic order); TXT
    shared-vs-mixed (including a key only one occurrence carries); row counts
    agree with the rows each projection builds. Replaced the now-meaningless
    `command_mode_group_key_behaves_like_logical_service` with a `browse_mode`
    mapping test.
  - `ui::app`: exact tab-count definitions; every tab's count equals its row
    count; host aggregate invokes a concrete child (`ssh nas.local`); service-type
    aggregate opens the picker and runs the chosen child (`echo 10.0.0.2`);
    command row runs a concrete service; same-host filter offered only by
    invariant-host projections; an active filter clears from any view; unresolved
    row reports no resolved host; selection survives recomputation by identity.
  - `ui::render`: host details list every service and show no representative TXT;
    type details list every host and show no representative TXT; logical-service
    details show shared TXT and flag differing values while showing per-occurrence
    addresses; unresolved host row renders as unresolved; every view renders with
    nothing discovered; tab counts render from `App::tab_counts`.
  - Verified beyond assertions by rendering the Host and Service Type tabs
    against heterogeneous data and reading the panes.

- **Documentation updated:** `README.md` (tab count meanings, aggregate row/detail
  semantics, same-host availability), `docs/keybindings.md` (`same_host`
  availability per view).

- **Validation evidence:** on top of `dd872b7` (worktree fast-forwarded from the
  stale base `87239fd` before starting).
  - `cargo fmt -- --check`: clean.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`: clean.
  - `cargo test --locked --all-targets`: 204 passed, 0 failed.
  - `cargo test --locked --all-targets --all-features`: 209 passed, 0 failed.

- **Follow-ups:**
  - **For task 006 (grouped action targeting).** A rule with no
    instance-specific field (e.g. `ssh {hostname}`) that matches several children
    of one row runs the *first* child with no picker, because `needs_instance` is
    driven by the template/predicate fields rather than by whether the candidates
    prepare distinct argument vectors. From a Host or Service Type row this is now
    much easier to hit. It contradicts CONTEXT's "if multiple candidates prepare
    distinct argument vectors, the user must choose a target", but the fix is
    task 006's `needs_instance`/targeting decision, not this task's projection
    work, so it was left alone. Test
    `a_service_type_row_targets_the_concrete_child_the_user_picks` uses an
    address-templating rule to exercise the picker path around it.
  - **For task 011 (scrollable pickers).** Aggregate detail panes are now longer
    (a host lists every service), which makes detail/picker overflow more common.
  - **For task 015 (App/render encapsulation).** `App::tab_counts` is a
    render-facing cache on `App`; it belongs to whatever browse-model type 015
    settles on. `EntryGroup::child_services()` is derived per frame — fine at
    discovery scale, but 015 may want it memoized with the rest of the model.
