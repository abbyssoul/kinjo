# Task 209 — Pin the release to the merged version commit, not moving main

| Field | Value |
|---|---|
| Status | `in-progress` |
| Priority | `P0` |
| Workstream | Publication |
| Depends on | 202, 204 |
| Likely conflicts | 203, 205, 207 |
| Owner | Codex (repository implementation) |

## Evidence

The first production `Release` run failed after a clean `Prepare Release`
because valid Dependabot PRs merged into `main` in between. `preflight` required
`GITHUB_SHA` to still equal the live `main` tip
(`gh api .../commits/main`), so any merge during the two-stage flow aborted the
release. Under continuous merging this fails almost every time, and it also
breaks the retry-safe recovery model: a retry after further merges would target
a newer commit than the first attempt and collide with the draft/tag that
attempt created.

## Root Cause

The release pinned itself to `main`'s moving tip instead of the immutable commit
the version bump produced. Invariant 3 ("the exact validated `main` commit") was
read as "current `main` HEAD" rather than "the reviewed commit that carries this
version."

## Required Outcome

`Release` resolves its target commit from the merged `release/v<version>` PR that
`Prepare Release` opens — its `mergeCommit` is immutable and identical on every
retry. An optional `sha` input overrides the lookup for hand-merged cases. The
commit is validated as an ancestor of `main` carrying the requested version, and
**every** job (validation, packaging, staging, publication, Homebrew) checks out
that pinned commit rather than the caller's tip. The live-HEAD equality check is
removed.

## Required Tests

- `release_validate_sha` accepts a 40-hex SHA and rejects empty, short,
  over-long, uppercase, and non-hex input;
- a version with no merged PR and no `sha` fails closed with a clear message;
- an explicit non-ancestor `sha` is rejected (GitHub run);
- two dispatches with `main` advanced between them resolve the same commit
  (GitHub run).

## Definition of Done

- [x] Target commit comes from the merged PR (or an explicit `sha`), not `main`
      HEAD.
- [x] The live-HEAD equality check is gone.
- [x] All reusable and packaging workflows accept a `ref` and check it out.
- [x] `release_validate_sha` is covered by `test-version.sh`.
- [ ] A `main`-moves-between-dispatches rerun resolves one commit on GitHub.

## Completion Record

- **Implemented:** `release.yml` preflight resolves the merged `release/v<ver>`
  PR merge commit (optional `sha` override), validates ancestry and manifest,
  and pins it. `ci-test`, `audit`, `nix`, `actionlint`, `release-deb`,
  `release-macos`, `source-package`, `stage`, `publish`, and the Homebrew
  handoff all check out the pinned commit. `preflight` gained
  `pull-requests: read`.
- **Tests:** `scripts/release/test-version.sh` covers `release_validate_sha`.
- **Validation:** Local actionlint, ShellCheck, and the three helper suites
  pass. Ancestry rejection and the moves-between-dispatches rerun need a GitHub
  run.
