# Task 204 — Build and stage verified artifacts without write credentials

| Field | Value |
|---|---|
| Status | `in-progress` |
| Priority | `P0` |
| Workstream | Release artifacts |
| Depends on | 203 |
| Likely conflicts | 205, 207 |
| Owner | Codex (repository implementation) |

## Required Outcome

Introduce the release workflow's default-safe `version` and `publish=false`
interface. Build the crate, Debian packages, and native macOS archives using
read-only jobs with checkout credentials disabled. Stage artifacts internally
with checksums and CycloneDX SBOM.

Use Ubuntu 24.04 x86/ARM and macOS 15 ARM/Intel. Run package dry-run,
release-profile execution, Debian install, archive content/permission, and
version checks before artifacts leave the build jobs.

## Required Tests

- wrong branch, SHA, or manifest version fails before building;
- both macOS binaries execute on their native runner;
- both Debian packages install and report the expected version;
- archive/package contents include the binary, license, README, and commands;
- `publish=false` creates no external state;
- build jobs have no write, OIDC, registry, or App credentials.

## Completion Record

- **Implemented:** `release.yml` defaults to `publish=false`, validates the
  exact main SHA, reuses all validation workflows, and stages a verified crate,
  deterministic source archive, all-target SBOM, native macOS archives, and
  x86/ARM Debian packages. Build jobs are contents-read with checkout
  credentials disabled.
- **Correction (review of this task):** the crate was uploaded as
  `target/package/kinjo-X.crate` alongside two repository-root files.
  `upload-artifact` roots an artifact at the least common ancestor of its paths,
  so the crate downloaded to `dist/target/package/` and the publisher's
  existence check failed — after the `release` approval, and invisibly to a
  `publish=false` rehearsal, which never ran the publisher. The crate is now
  moved beside its siblings before upload, and a new `stage` job runs
  `scripts/release/check-artifacts.sh` in **both** modes so the dry run asserts
  the same artifact set the publisher requires.
- **Artifacts verified:** Workflow checks package/archive contents,
  architecture, executable permission, installation, and reported version.
- **Tests:** `scripts/release/test-artifacts.sh` covers the complete set, a
  missing crate, an unexpected file, an absent directory, a non-stable version,
  repeated runs, and the least-common-ancestor regression above.
- **Validation:** YAML and embedded shell pass locally. Native package jobs and
  no-mutation dry run remain to be proven on GitHub.
