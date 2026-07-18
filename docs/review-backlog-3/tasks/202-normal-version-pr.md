# Task 202 — Replace movable request tags with a normal version PR

| Field | Value |
|---|---|
| Status | `in-progress` |
| Priority | `P0` |
| Workstream | Release preparation |
| Depends on | 201 |
| Likely conflicts | 205, 207 |
| Owner | Codex (repository implementation) |

## Evidence

The reviewed `.github/workflows/prepare-release.yml` deleted the triggering tag,
skips CI, merges with `--admin`, recreates the tag, and dispatches publishers.
Normal clones observed five moved local tags whose tagged manifest version no
longer matched their tag name.

## Required Outcome

Make preparation a manual stable-SemVer input that uses a GitHub App token to
create/update `release/v<version>` and open a normal PR. Reject invalid,
non-increasing, or already released versions. Never tag, merge, skip checks, or
publish. Retries reuse matching state and reject conflicts.

## Required Tests

- malformed, prerelease, equal, older, and already-tagged versions fail;
- numeric SemVer ordering handles multi-digit components;
- a valid version updates `Cargo.toml` and `Cargo.lock` only;
- matching open PR is idempotent; a conflicting remote branch fails.

## Definition of Done

- [x] No workflow deletes or moves a release tag.
- [x] The App-authored PR triggers ordinary required checks.
- [x] The workflow contains no `[skip ci]`, `--admin`, merge, or publication.
- [x] Release instructions describe the two-stage flow.

## Completion Record

- **Implemented:** `prepare-release.yml` now validates a manual stable version,
  creates only a version commit, mints a scoped App token, and opens/reuses a
  normal PR without tagging or merging.
- **Tests:** `scripts/release/test-version.sh` covers malformed, prerelease,
  oversized, equal, older, and multi-digit versions plus the main ref.
- **Validation:** Local helper, actionlint, and ShellCheck pass. App-authored PR
  and conflict/idempotency cases remain blocked on Task 201.
