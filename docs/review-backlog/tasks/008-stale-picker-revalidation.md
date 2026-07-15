# 008: Revalidate Picker Targets Against Live Discovery

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P0` |
| Workstream | UI / Command rules |
| Depends on | 001, 002, 006 |
| Likely conflicts | 010, 011, 014, 015 |
| Owner | Unclaimed |

## Why This Matters

Action and instance pickers retain cloned match/entry snapshots. Discovery Upsert
and Remove events recompute visible rows but do not invalidate those snapshots.
The user can therefore confirm a picker after its target disappeared or changed
and execute a command with stale hostname/address/TXT data.

Picker state should store stable identities and resolve through the current browse
model immediately before preparation/execution.

## Evidence

- `src/ui/app.rs:204-229`: discovery events update records and recompute rows but
  do not close or rebuild pickers.
- `src/ui/app.rs:457-500`: action/instance picker handlers read cloned state.
- `src/ui/app.rs:577-600`: action matches are cloned into modal state.
- `src/ui/app.rs:679-692`: pending actions contain cloned matching records.
- `src/ui/app.rs:695-725`: execution does not re-resolve/rematch current records.
- `src/ui/app.rs:746-747,776-793`: reload and explicit refresh close pickers,
  showing that invalidation exists only for those paths.
- Service picker state also relies on the mutable selected command-group row rather
  than a stable command identity.

## Required Outcome

- Modal state stores stable command, group, occurrence, and/or prepared-candidate
  identities instead of authoritative cloned discovery data.
- Before execution, resolve the target against current records, rerun the current
  rule, and confirm the chosen effective command still exists.
- If the selected target was removed, stopped matching, or changed effective argv,
  do not execute it. Close or rebuild the picker with a clear status.
- Unrelated discovery changes preserve a valid open picker and its selected
  identity where practical.
- If a live change adds/removes choices, selection remains on the same identity or
  clamps predictably; never transfer selection silently to a different target.
- Refresh and successful config reload continue to invalidate derived modal state.
- Terminal real-discovery failure closes every picker before clearing records.

## Implementation Constraints

- Use structured occurrence identity from task 001 and prepared candidate identity
  from task 006.
- Do not compare display labels as identity.
- Keep revalidation in the browse/action model; render remains a consumer.
- Avoid holding references across event-loop iterations.

## Suggested Implementation Sequence

1. Add Remove/Upsert tests while each picker mode is open.
2. Replace cloned authoritative modal data with stable selection identities.
3. Reconcile open modal state after discovery recomputation.
4. Re-resolve and rematch once more immediately before execution.
5. Define clear status messages for removed/changed/no-longer-matching targets.

## Non-Goals

- Pausing discovery while a picker is open.
- Redesigning picker scrolling; task 011 owns visibility.
- Persisting modal state through refresh or successful command reload.

## Acceptance Criteria / Definition of Done

- [ ] Removed targets cannot execute from Action, Instance, or Service pickers.
- [ ] Changed hostname/address/TXT cannot execute using the stale value.
- [ ] Unrelated events do not silently change the selected target.
- [ ] Added/removed candidates reconcile deterministically.
- [ ] Final execution always uses current records and current command rules.
- [ ] Full validation passes.

## Required Tests

- Open ActionPicker, remove selected occurrence, press Enter: no execution.
- Open InstancePicker, update selected address, press Enter: stale argv impossible.
- Open ServicePicker, remove/reorder its service choices via discovery: the stored
  identity never retargets to the row that inherited its old index.
- Terminal real-discovery failure closes every picker and executes nothing.
- Unrelated Upsert while picker open: target remains selectable.
- Candidate stops matching due to TXT update: clear failure and no launch.

## Validation

```sh
cargo test --locked ui::app::tests::invoke
cargo test --locked ui::app::tests::instance_picker
cargo test --locked ui::app::tests::command_view
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
