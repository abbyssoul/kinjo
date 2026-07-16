# 008: Revalidate Picker Targets Against Live Discovery

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P0` |
| Workstream | UI / Command rules |
| Depends on | 001, 002, 006 |
| Likely conflicts | 010, 011, 014, 015 |
| Owner | Claude Opus 4.8, on `main` |

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

- [x] Removed targets cannot execute from Action, Instance, or Service pickers.
- [x] Changed hostname/address/TXT cannot execute using the stale value.
- [x] Unrelated events do not silently change the selected target.
- [x] Added/removed candidates reconcile deterministically.
- [x] Final execution always uses current records and current command rules.
- [x] Full validation passes.

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

- **Implemented:** A picker now remembers *what was chosen*, not the data it was
  built from. `PickerAnchor` is either `Row(EntryGroupId)` (a browse row) or
  `Service { command, service }` (a logical service under a command row — the
  command view projects rules, not entries, so its rows are not browse rows and
  cannot be found among them). `resolve_anchor` re-matches it against current
  records on demand.

  `action_matches` and `pending_action` survive as caches but are now rebuilt
  from the anchor by `reconcile_action_pickers` on *every* recompute, and
  `reconcile_service_picker` does the same for the service picker's cursor. Since
  `recompute_visible` is the only thing that can change the records underneath, a
  picker cannot list anything discovery has retracted; and because the event loop
  drains before it draws and handles at most one key per iteration, the key
  handler always acts on exactly the state the user was shown.

  Cursors are restored by identity, never by position: the action picker by
  command name, the service picker by `EntryGroupId`, and the instance picker by
  **task 006's prepared-command key** (`Option<PreparedCommand>`). That last
  choice matters twice. An occurrence id would not work — address-expanded
  candidates of one record all share one `EntryId`, so the cursor would snap back
  to the first address. And because the key *is* the prepared command, "the argv
  under the cursor changed" and "the target is gone" become the same condition,
  which is precisely the required semantics.

  When the chosen thing goes, `abandon_picker` closes the picker with a status
  naming what happened, rather than sliding the cursor onto a neighbour and
  letting a pending Enter run a service the user never selected.

  Already satisfied by earlier tasks and verified intact: refresh, successful
  reload, and terminal discovery failure each close every picker, the last of
  them before clearing records.

- **Tests added/updated:** 7 new tests in `ui::app`:
  `removing_the_selected_service_closes_an_open_action_picker`,
  `removing_the_selected_target_closes_an_open_instance_picker`,
  `updating_the_selected_address_cannot_execute_the_stale_argv`,
  `updating_an_unselected_address_keeps_the_instance_picker`,
  `an_unrelated_removal_keeps_the_picker_and_its_selection`,
  `removing_a_service_above_the_cursor_does_not_retarget_the_service_picker`, and
  `a_txt_update_that_unmatches_the_rule_closes_the_action_picker`.

  The service-picker test is the sharpest: with `[alpha, beta, gamma]` and the
  cursor on `beta` (index 1), removing `alpha` slides `gamma` into index 1, so
  the old code would have run `gamma` on Enter.

- **Documentation updated:** `docs/actions.md` — the *Choosing a target* section
  now states that the list is rebuilt as discovery changes, that a retracted or
  materially changed target closes the picker with a message instead of being
  confirmable, and that the selection never moves onto a neighbour.

- **Validation evidence:** `cargo fmt -- --check` clean; `cargo clippy --locked
  --all-targets --all-features -- -D warnings` clean; 319 tests default, 334
  all-features, 0 failed (from 312/327).

  Five of the seven new tests were confirmed to be genuine regressions by
  temporarily neutering `reconcile_action_pickers`/`reconcile_service_picker`:
  the two removal tests, the stale-argv test, the service-picker retarget test,
  and the TXT-unmatch test all failed, then passed again once restored. The two
  "preserve" tests pass either way by design — they guard against over-closing.

  Not verified interactively: reproducing a live retraction against a real
  network is not reproducible on demand, and the fake backend emits a fixed
  sample set. The tests drive the real `App` with real `DiscoveryEvent`s through
  the real session interface instead.

- **Follow-ups:**
  1. An occurrence whose hostname or port changes is a *new* occurrence, because
     task 001 builds `EntryId` from the resolved endpoint when the adapter names
     no occurrence. So a renamed host reads as remove + add, and a picker
     anchored on it closes as "gone". That is correct and safe, but the wording
     could be more precise about what happened. Worth revisiting only if it
     confuses anyone in practice.
  2. `reconcile_action_pickers` re-prepares each target to match the cursor.
     Same cost note as task 006's: recompute-time, not per-frame, and tiny at
     realistic row sizes.
