# 006: Disambiguate Grouped Action Targets by Prepared Command

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P0` |
| Workstream | Command rules / UI |
| Depends on | 005 |
| Likely conflicts | 007, 008, 010 |
| Owner | Claude Opus 4.8, on `main` |

## Why This Matters

Host and service-type rows can contain several entries. The current matcher asks
for an instance picker only when predicates/templates mention address or port.
A rule such as `ssh {hostname}` in a service-type group can therefore execute the
lexically first host without asking. Name, hostname, type, domain, TXT, port, and
address can all vary across grouped candidates.

Disambiguation should be based on observable execution, not a hard-coded list of
fields: if candidates prepare different argument vectors, the user chooses.

## Evidence

- `src/plumber/mod.rs:97-102`: `needs_instance` is derived from limited field checks.
- `src/plumber/mod.rs:104-110`, `205-207`: only address/port are considered
  instance-specific.
- `src/ui/app.rs:747`: when `needs_instance` is false, the first matching record
  is executed.
- `src/discovery/entry.rs:269-301`: host/service-type groups may contain
  heterogeneous entries while scalar fields come from the first child.

Re-verified after tasks 004 and 010 landed (line numbers above updated):

- The defect stands and was independently rediscovered by both the 004 and 010
  agents: `needs_instance` still keys off template/predicate *fields* rather than
  whether candidates prepare distinct argv, so a rule with no instance-specific
  field (`ssh {hostname}`) matching several children still runs the first child
  with no picker. This contradicts CONTEXT's "if multiple candidates prepare
  distinct argument vectors, the user must choose a target."
- Task 010 makes it **easier to hit**: aggregate rows now legitimately present
  heterogeneous children, so more real selections reach this path. 010 routed its
  own invocation test around the defect via an address-templating rule rather than
  broadening scope — that test is a candidate to retarget once this task lands.
- The last evidence bullet is **obsolete**: task 010 removed first-child scalar
  copying (`GroupFacts` has one variant per mode, so an aggregate has no
  service-type/port/TXT field to read). The heterogeneity it describes is still
  real; the "scalars from the first child" mechanism is gone.
- Task 004 narrowed the blast radius: address predicates are now conjunctive per
  candidate, so `matches_group` yields only addresses satisfying the whole rule.
  Distinct-argv counting replaces `needs_instance`, it does not have to re-derive
  which addresses are valid.
- 004 also noted a rule with an `address` predicate but no `{address}`/`{port}`
  template still reports `needs_instance` and can expand to several candidates
  preparing **identical** argv — a redundant picker. That is the inverse of this
  task's other half ("identical prepared commands may collapse") and the same
  distinct-argv counting should resolve both.

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

- [x] `{hostname}`, `{name}`, `{service_type}`, `{domain}`, `{port}`, `{address}`,
      and `txt.*` differences cause selection when prepared argv differ.
- [x] Constant commands or candidates producing identical mode/argv run once
      without a picker.
- [x] No group path executes an arbitrary first child.
- [x] Picker labels identify the actual target sufficiently to choose safely.
- [x] `docs/actions.md` matches the generalized selection behavior.
- [x] Full validation passes.

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

- **Implemented:** `needs_instance` and its `is_instance_field` helper are gone.
  `CommandConfig::distinct_targets` prepares every candidate through the task 005
  compiled rule and keeps one per distinct `PreparedCommand`, in discovery order.
  `MatchResult` now carries `targets` (the distinct execution targets) instead of
  `matching_records` + `needs_instance`, and answers `needs_selection()` —
  `targets.len() > 1`. `PreparedCommand` gained `PartialEq`/`Eq`, which is where
  "the same effective command" is now defined: equality of mode plus complete
  argv. The UI asks the rule (`if action.needs_selection()`) rather than
  re-deriving the decision; `render` uses the same answer for its
  `⊙ choose instance` marker, so hint and behavior cannot disagree.

  Both halves of the task fall out of the one change: a differing field that no
  list called instance-specific (`{hostname}`, `{name}`, `txt.*`) now produces
  distinct argv and therefore a picker, and 004's inverse case — an `address`
  predicate with no `{address}` template — produces identical argv and therefore
  collapses, removing the redundant picker.

  A candidate that fails to prepare is *kept* as a target rather than dropped, so
  a rule that cannot run for the chosen service can never silently run against a
  different one. Selecting it reports the failure.

- **Tests added/updated:** 7 new tests. In `plumber`:
  `a_hostname_template_over_several_hosts_asks_which_host` (the task's stated
  reproduction), `a_txt_template_over_differing_values_asks_which_service`,
  `several_services_preparing_one_identical_command_do_not_ask`,
  `different_services_rendering_the_same_command_collapse`,
  `a_candidate_that_cannot_prepare_is_offered_rather_than_skipped`,
  `addresses_preparing_the_same_command_do_not_ask_which_address` and
  `addresses_preparing_different_commands_ask_which_address`. In `ui::app`:
  `a_missing_requirement_reports_failure_without_trying_another_target`.

  Retargeted `ui::app::tests::a_service_type_row_targets_the_concrete_child_the_user_picks`
  from `PING_ADDR` to `SSH` (`ssh {hostname}`), as this task's evidence
  anticipated: task 010 had routed it around the defect via an address-templating
  rule. Address expansion stays covered by
  `instance_picker_disambiguates_then_executes_chosen_address`.

  Replaced `instance_specific_predicates_and_templates_request_instance_selection`,
  which asserted the per-rule `needs_instance` flag directly — a concept this task
  removes. Its real content (address predicate + hostname template) survives as
  `addresses_preparing_the_same_command_do_not_ask_which_address`.

- **Documentation updated:** `docs/actions.md` replaces the "instance-specific
  fields such as `{address}` or `{port}`" explanation with a *Choosing a target*
  section describing the actual rule: candidates preparing identical commands
  collapse and run without asking; differing commands require selection; any
  varying placeholder therefore causes the question.

- **Validation evidence:** `cargo fmt -- --check` clean; `cargo clippy --locked
  --all-targets --all-features -- -D warnings` clean; 312 tests default, 327
  all-features, 0 failed (from 305/320). `cargo run -- list-commands --config-dir
  actions` still validates all 5 bundled rules — including the shipped `ssh`
  rule, which is `ssh {hostname}` and so was exposed to this defect with the
  stock configuration. The fuzz crate still builds; no target references the
  changed API.

  The three new selection tests were confirmed to be genuine regressions by
  temporarily restoring the old heuristic in `needs_selection`: all three failed,
  then passed again once reverted.

  Not verified interactively: the fake backend advertises a single `_ssh._tcp`
  service, so it cannot present a service-type row with two hosts. The UI-level
  coverage drives the real `App` with real key events instead.

- **Follow-ups:**
  1. `matches_group` now prepares every candidate on each `recompute_visible`.
     This is a state-change path, not a per-frame one, and the cost is small
     string rendering, but if a very large network makes recomputation visible,
     preparing lazily or caching per row is the obvious answer.
  2. The fake backend cannot reproduce heterogeneous aggregate rows (one SSH
     service, one host). A second SSH host would make this task's behavior
     demonstrable by hand and is worth considering with task 016's owner.
