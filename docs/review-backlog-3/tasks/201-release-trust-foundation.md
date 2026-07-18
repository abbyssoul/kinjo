# Task 201 — Provision release identities and protection settings

| Field | Value |
|---|---|
| Status | `blocked` |
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

- **External settings:** Not changed from this repository. The exact setup is
  now an ordered checklist in `docs/releasing.md`.
- **Proof:** Pending App, environment, branch-rule, immutability, and crates.io
  owner access.
- **Legacy credentials revoked:** No. Revoke only after the first production
  run succeeds end to end.
