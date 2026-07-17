# Task 208 — Close coverage gaps and rehearse recovery

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P2` |
| Workstream | Operations |
| Depends on | 201–207 |
| Likely conflicts | — |
| Owner | Codex (documentation/coverage); release owner (rehearsal) |

## Required Outcome

Make Sonar wait for the quality gate with all-feature coverage. Document the
two-stage release, environment approval, idempotent checkpoints, and recovery
procedures. Run a complete `publish=false` rehearsal and record its workflow
URLs/results. Use the next genuine version as the production acceptance test;
do not create disposable immutable tags.

Recovery documentation must cover failures before draft creation, after draft
assets, after crates.io publication, after immutable release publication, and
during tap PR creation.

## Definition of Done

- [ ] Every review finding maps to completed work or an explicit accepted risk.
- [ ] Dry-run validation and all architecture matrices are green.
- [ ] Operators can resume every checkpoint without replacing public state.
- [ ] The first production release proves OIDC, immutability, attestations, and
      the Homebrew PR path before legacy tokens are revoked.

## Completion Record

- **Dry run:** Pending Task 201 and a merged workflow run.
- **Production release:**
- **Legacy credentials revoked:**
- **Residual risks:** The Sonar job now waits for the quality gate with
  all-feature coverage, and `docs/releasing.md` covers every checkpoint. GitHub
  Actions concurrency preserves the active publication but retains only one
  pending run, so operators must not queue multiple release dispatches.
