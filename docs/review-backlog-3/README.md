# Kinjo Review Backlog — Round 3

This directory turns the GitHub Actions CI/CD review performed on 2026-07-17
at `6eed89e` into agent-ready tasks. Implementation began at `211aa7e`; the
intervening commit changes only a fuzz oracle and does not alter the findings.

Read [`CONTEXT.md`](CONTEXT.md) before claiming a task. It defines the release
invariants, trust boundaries, external prerequisites, and completion gate.

## Overall Verdict

The ordinary CI suite has good breadth: Rust runs on all three operating
systems, Linux exercises all features, the rendered TUI is smoke-tested, and
audit/fuzz/CodeQL/Sonar/Nix checks cover distinct risks. The CD design is much
weaker than that CI foundation:

- a pushed tag is deleted and recreated at another commit, leaving stale tags
  in normal clones and breaking the identity expected of a release tag;
- the version PR skips normal CI and is merged with administrator bypass;
- the published release precedes asynchronous publishers, so preparation can
  succeed while one or more public channels fail;
- a retry after release creation skips the dispatches that may have failed;
- build jobs retain write-capable checkout credentials and published assets
  can be replaced with `--clobber`;
- Homebrew checksum substitutions are not validated and update the tap by a
  direct push without Homebrew tests.

The target is a two-stage flow:

```text
Prepare Release(version)
  -> GitHub App creates a normal version PR
  -> required PR checks and human merge

Release(version, publish=false by default)
  -> reusable validation gate on exact main SHA
  -> read-only native package builds and smoke tests
  -> protected environment approval when publish=true
  -> draft release + immutable tag and assets
  -> crates.io through OIDC
  -> publish immutable GitHub release
  -> checked Homebrew tap PR
```

## Agent Workflow

1. Claim a `ready` task whose dependencies are `done`.
2. Revalidate its evidence against the current workflows.
3. Mark it `in-progress` and add an owner before editing.
4. Preserve unrelated work and coordinate any listed conflicts.
5. Keep the task inside its required outcomes and non-goals.
6. Run its targeted checks plus the shared completion gate.
7. Fill its completion record, mark it `done`, and update this index.

Status values are `ready`, `blocked`, `in-progress`, and `done` with the same
meaning as the earlier review backlogs.

## Task Index

| ID | Priority | Status | Task | Depends on | Likely conflicts |
|---|---|---|---|---|---|
| 201 | P0 | blocked | [Provision release identities and protection settings](tasks/201-release-trust-foundation.md) | — | 202, 205, 206 |
| 202 | P0 | in-progress | [Replace movable request tags with a version PR](tasks/202-normal-version-pr.md) | 201 | 205, 207 |
| 203 | P1 | in-progress | [Make CI checks reusable as a release gate](tasks/203-reusable-validation-gate.md) | — | 204, 207, 208 |
| 204 | P0 | in-progress | [Build and stage verified artifacts without write credentials](tasks/204-read-only-artifact-staging.md) | 203 | 205, 207 |
| 205 | P0 | blocked | [Publish synchronously through an immutable retry-safe release](tasks/205-retry-safe-publication.md) | 201, 202, 204 | 206, 207 |
| 206 | P1 | blocked | [Deliver Homebrew through a validated tap PR](tasks/206-homebrew-pr.md) | 201, 205 | 207 |
| 207 | P1 | in-progress | [Pin and bound the workflow supply chain](tasks/207-workflow-hardening.md) | — | every workflow task |
| 208 | P2 | blocked | [Close coverage gaps and rehearse recovery](tasks/208-closeout-and-rehearsal.md) | 201–207 | — |

Priority meanings:

- **P0:** release correctness, integrity, or credential isolation.
- **P1:** a required validation or hardening layer around the P0 design.
- **P2:** operational proof and maintainability after the new path exists.

## Workstreams and Ordering

```text
External trust: 201 -> 202 -----------+
                                       +-> 205 -> 206 -> 208
Validation:     203 -> 204 -----------+                  ^
Hardening:      207 -----------------------------------+
```

Task 207 pins actions, caps timeouts, and narrows permissions across every
workflow. That work does not depend on the publication design and was in fact
done first; it is ordered late only because it touches the same files.

Tasks 201 and 203 may proceed concurrently. Task 201 requires repository,
crates.io, and tap-owner access; code agents must not pretend those settings
exist. The implementation for later tasks is staged in this branch, but their
status remains `in-progress` or `blocked` until the external prerequisites and
GitHub-only architecture/recovery checks are actually satisfied.

## Findings Mapped to Tasks

| Finding | Resolution |
|---|---|
| Tags are deleted and moved; five of eight local release tags observed during review resolve to a mismatching manifest version | 202, 205 |
| Public release creation and publisher dispatch are asynchronous and non-retryable | 205 |
| Version PR uses `[skip ci]`, `--admin`, and only one Linux test command | 202, 203 |
| Debian/macOS publishers omit tag/SHA validation and replace assets | 204, 205 |
| Write checkout credentials are live during Cargo builds | 204 |
| Homebrew checksum rewrites can silently miss and push directly to main | 206 |
| Downstream releases/tap updates are not serialized | 205, 206 |
| crates.io and tap use long-lived secrets | 201, 205, 206 |
| Several actions, build tools, runners, and nightly Rust are mutable | 207 |
| Nix ignores source paths and does not build AArch64 | 203, 208 |
| Release profiles/packages are not smoke-tested before publication | 204 |
| No SBOM, checksums, or verifiable build provenance are published | 204, 205 |
| Most jobs have the six-hour default timeout; fuzz input is unbounded | 207 |
| Sonar covers default features and does not wait for its quality gate | 208 |

## Baseline Validation

At review time:

```text
actionlint -no-color       pass
shellcheck -x scripts/*.sh pass
git status --short         clean
```

Line numbers in tasks are evidence anchors, not authority. Revalidate them
before making changes.

## Current Implementation Validation

The repository-side implementation is present, but is not considered complete
until Task 201 and the GitHub-only gates are recorded. Local validation on
2026-07-17 passed:

```text
actionlint -no-color
shellcheck -x scripts/*.sh scripts/release/*.sh
scripts/release/test-version.sh
scripts/release/test-homebrew-formula.sh
scripts/release/test-artifacts.sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
git diff --check
```

See [`../releasing.md`](../releasing.md) for setup, operation, and recovery.
