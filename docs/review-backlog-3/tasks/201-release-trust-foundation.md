# Task 201 — Provision release identities and protection settings

| Field | Value |
|---|---|
| Status | `in-progress` |
| Priority | `P0` |
| Workstream | External release trust |
| Depends on | — |
| Likely conflicts | 202, 205, 206 |
| Owner | Repository/tap owner |

## Why This Matters

The current publishers depend on long-lived crates.io and tap tokens and have
no protected environment. The replacement workflows cannot be enabled safely
until narrowly scoped identities and approval boundaries exist.

## Required Outcome

Provision the GitHub App, environments, crates.io trusted publisher, immutable
release setting, and tap branch rules exactly as listed in `../CONTEXT.md`.
Record links or screenshots without copying private-key material. Keep legacy
tokens until the first successful production release, then revoke them.

## Acceptance Criteria

- [x] App installation is restricted to the two release repositories.
- [x] Preparation can create a test PR with a short-lived App token.
- [X] Both environments reject workflow code dispatched from non-`main` refs.
- [x] Release jobs cannot access protected credentials before non-self approval.
- [x] crates.io trusts only the exact release workflow/environment identity.
- [x] Future GitHub releases are immutable.
- [x] Tap `main` requires the Homebrew checks from task 206.
- [x] `kinjo` `main` requires the ordinary CI checks and denies admin bypass.
- [x] `release-preparation` has **no** required reviewer: the Homebrew job runs
      inside the release and would deadlock behind a second approval.

## Completion Record

- **External settings:** Provisioned by the repository/tap owner. The GitHub App
  (installed on `abbyssoul/kinjo` and `abbyssoul/homebrew-abyss`), the
  `release-preparation` and `release` environments with their protection rules,
  and the crates.io trusted publisher bound to this repository/workflow/`release`
  environment are all in place. The ordered setup checklist lives in
  `docs/releasing.md`.
- **Proof:** App installation, environment, branch-rule, immutability, and
  crates.io trusted-publisher settings configured on the two repositories.
- **Remaining before `done`:** Perform the first real `publish=true` release to
  validate the wiring end to end (App token, protected approval, OIDC crates.io
  publish, immutable release, tap PR). This is the last step and is what the task
  is now gated on.
- **Legacy credentials revoked:** No. Revoke `CARGO_REGISTRY_TOKEN` and
  `HOMEBREW_TAP_TOKEN` only after that first production run succeeds end to end.
