# Task 206 — Deliver Homebrew through a validated tap PR

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P1` |
| Workstream | Homebrew |
| Depends on | 201, 205 |
| Likely conflicts | 207 |
| Owner | Codex (Kinjo implementation); tap owner (tap workflow/rules) |

## Required Outcome

Use a short-lived App token to create/update a versioned tap branch and PR.
Download only immutable release assets, rewrite formula URLs/hashes, assert all
six exact results, and reject downgrades. Never push tap `main`.

Add a required tap-side workflow on native macOS ARM and Intel that audits,
installs, and executes the changed formula. The release records the PR URL and
waits for successful tap checks; tap protections govern merge.

## Required Tests

- missing checksum marker fails before commit;
- mismatched URL/hash and stale version fail;
- matching open PR is updated idempotently;
- both native Homebrew checks install and report the expected version;
- the token is restricted to the App installation and absent from clone URLs.

## Completion Record

- **Kinjo workflow:** Downloads only immutable release assets, validates three
  URLs and checksum markers exactly once, rejects downgrades, pushes a
  versioned App-authored branch, opens/reuses a PR, and waits for its checks.
- **Tap workflow/branch rules:** Native ARM/Intel workflow template is in
  `homebrew-tap-check.yml`; copying it and making both jobs required is pending.
- **Validation:** `scripts/release/test-homebrew-formula.sh` proves migration,
  exact update, downgrade and invalid-checksum rejection, and missing-marker
  failure. Tap PR execution remains blocked on Tasks 201 and 205.
