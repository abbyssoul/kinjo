# 010: Render Mode-Aware Aggregate Views

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P1` |
| Workstream | UI / Discovery grouping |
| Depends on | 001 |
| Likely conflicts | 006, 008, 011, 012, 013, 014, 015 |
| Owner | Unclaimed |

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

- `src/discovery/entry.rs:257-267`: one `EntryGroup` shape stores scalar metadata.
- `src/discovery/entry.rs:278-301`: those scalars are copied from `instances[0]`.
- `src/ui/app.rs:243-253`: all non-command grouping modes use the same groups.
- `src/ui/render.rs:260-365`: row/details render scalar fields as facts regardless
  of grouping mode.

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

- [ ] Heterogeneous Host details contain all child services and no arbitrary
      representative metadata.
- [ ] Heterogeneous Service Type details contain all child hosts/services and no
      arbitrary representative metadata.
- [ ] Logical-service and command views retain their valid details/actions.
- [ ] Differing logical-service TXT/address values appear per occurrence or as
      explicitly mixed, never as arbitrary group-wide values.
- [ ] Tab/detail counts follow the exact definitions above.
- [ ] Same-host filtering is disabled outside invariant-host projections.
- [ ] Matching/invocation targets concrete children correctly.
- [ ] Group ordering is stable across input permutations/recomputations.
- [ ] Rendering tests cover empty, unresolved, homogeneous, and heterogeneous data.
- [ ] Full validation passes.

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
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
