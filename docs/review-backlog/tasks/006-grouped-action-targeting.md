# 006: Disambiguate Grouped Action Targets by Prepared Command

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P0` |
| Workstream | Command rules / UI |
| Depends on | 005 |
| Likely conflicts | 007, 008, 010 |
| Owner | Unclaimed |

## Why This Matters

Host and service-type rows can contain several entries. The current matcher asks
for an instance picker only when predicates/templates mention address or port.
A rule such as `ssh {hostname}` in a service-type group can therefore execute the
lexically first host without asking. Name, hostname, type, domain, TXT, port, and
address can all vary across grouped candidates.

Disambiguation should be based on observable execution, not a hard-coded list of
fields: if candidates prepare different argument vectors, the user chooses.

## Evidence

- `src/plumber/mod.rs:77-83`: `needs_instance` is derived from limited field checks.
- `src/plumber/mod.rs:104-119`: only address/port are considered instance-specific.
- `src/ui/app.rs:679-692`: when `needs_instance` is false, the first matching
  record is executed.
- `src/discovery/entry.rs:269-301`: host/service-type groups may contain
  heterogeneous entries while scalar fields come from the first child.

Concrete reproduction for a regression test: group two `_ssh._tcp` entries on
different hosts in `GroupingMode::ServiceType`; invoke `ssh {hostname}`. Current
behavior runs the first hostname instead of opening a picker.

## Required Outcome

- For a selected group/rule, prepare every valid concrete candidate through the
  compiled rule from task 005.
- Group candidates by effective execution identity: mode plus complete argv.
- If there is one effective command, execute it without a redundant picker.
- If there are multiple effective commands, require explicit target selection.
- Picker labels make differing service/host/address/port values understandable.
- No candidate is silently chosen merely because its differing field is not
  address or port.
- Candidates missing a referenced runtime field are excluded as defined in task
  005. Structural template failures cannot survive compilation. A rule-wide
  runtime failure such as a missing mandatory requirement leaves the action
  visible but selecting it reports the failure; it never auto-runs another
  candidate.

## Implementation Constraints

- Candidate generation and prepared-command equality belong in the command-rule
  module; picker presentation belongs in UI.
- Do not compare raw templates or a manually maintained field list.
- Preserve deterministic candidate order and collapse exact duplicate execution
  results without losing a useful display label.
- Keep command interpolation's argument-injection barrier intact.

## Suggested Implementation Sequence

1. Add failing grouped hostname/name/TXT regression tests.
2. Have the validated rule produce prepared candidates with stable target identity.
3. Replace `needs_instance` heuristics with distinct prepared-command counting.
4. Update picker rendering/labels for heterogeneous candidates.
5. Update action documentation's instance-selection explanation.

## Non-Goals

- Rebuilding aggregate row/detail rendering; task 010 owns presentation.
- Handling discovery changes while the picker is open; task 008 follows this task.
- Adding a user preference to auto-select a target.

## Acceptance Criteria / Definition of Done

- [ ] `{hostname}`, `{name}`, `{service_type}`, `{domain}`, `{port}`, `{address}`,
      and `txt.*` differences cause selection when prepared argv differ.
- [ ] Constant commands or candidates producing identical mode/argv run once
      without a picker.
- [ ] No group path executes an arbitrary first child.
- [ ] Picker labels identify the actual target sufficiently to choose safely.
- [ ] `docs/actions.md` matches the generalized selection behavior.
- [ ] Full validation passes.

## Required Tests

- Service-type group, two hosts, `ssh {hostname}`: picker then chosen host argv.
- Host group with several matching service names/TXT values: picker.
- Several records producing identical argv: no picker.
- Several addresses with different prepared argv: picker.
- Candidate missing a referenced field is excluded; if none remain the action is
  absent. A missing mandatory requirement reports failure without fallback.

## Validation

```sh
cargo test --locked plumber
cargo test --locked ui::app::tests::invoke
cargo test --locked ui::app::tests::instance_picker
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
