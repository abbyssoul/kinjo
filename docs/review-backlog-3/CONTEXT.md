# Round 3 Context — Release Integrity and Trust Boundaries

## Release Invariants

Every implementation must preserve all of these:

1. A release tag is created once and never moved or reused.
2. `vX.Y.Z`, the tagged `Cargo.toml`, the crates.io version, package filenames,
   Homebrew URLs, and release title all describe the same version.
3. The release target is the exact validated `main` commit.
4. Builds execute without repository-write, registry, or tap credentials.
5. Publication uses only artifacts that passed the candidate gate.
6. An existing checkpoint is reusable only when its SHA and hashes match.
   Conflicting external state fails closed.
7. A failed candidate build creates no tag, crate, public release, or tap PR.
8. A published GitHub release and its assets are immutable.
9. The workflow that starts a release does not report success before every
   required downstream job reaches a terminal successful state.

No registry combination can be transactional. The chosen order is therefore:

1. build and validate everything privately;
2. create/resume a matching draft GitHub release;
3. publish crates.io with a short-lived OIDC token;
4. publish the prepared immutable GitHub release;
5. open and validate the Homebrew tap PR.

A failure after step 3 can temporarily expose the crate before GitHub release
publication. That is accepted because retrying can publish the existing draft;
the inverse order would leave a public GitHub release missing its crate.

## Workflow Interfaces

### Prepare Release

- Trigger: `workflow_dispatch` on protected `main`.
- Input: `version`, required stable SemVer without a leading `v`.
- Output: an idempotent `release/v<version>` PR created by the release GitHub
  App. It never tags, merges, or publishes.

### Release

- Trigger: `workflow_dispatch` on protected `main`.
- Inputs: `version` and boolean `publish`, default `false`.
- `publish=false`: run every validation and packaging job, but make no external
  mutation.
- `publish=true`: require the protected `release` environment before accessing
  OIDC or write credentials.

### Reusable validation

The Rust CI, audit, Nix, and workflow lint suites accept `workflow_call` and
operate on the caller's SHA. Their existing PR/push/schedule behavior remains.

### Homebrew handoff

The handoff carries the immutable tag, release SHA, and expected asset names.
It opens or updates a tap PR; it never pushes tap `main`.

## Trust Boundaries

| Actor/data | Trust | Allowed authority |
|---|---|---|
| Pull-request source and Cargo dependencies | untrusted code | read-only checkout and caches |
| Candidate build jobs | trusted workflow, executes dependency code | repository contents read only |
| Release GitHub App | privileged automation | contents and pull requests in `kinjo` and the tap only |
| Protected release publisher | approved exact main SHA | OIDC, attestations, and GitHub release write |
| crates.io token | short lived | publish `kinjo` only through trusted publishing |
| Homebrew formula inputs | immutable public release assets | create/update one versioned tap PR |

`actions/checkout` must use `persist-credentials: false` in any job that
executes Cargo or other repository code. Jobs needing write access must not
compile source.

## External Prerequisites

Task 201 records owner-controlled changes that cannot be represented solely in
this repository:

- a GitHub App installed on `abbyssoul/kinjo` and
  `abbyssoul/homebrew-abyss`, with metadata read plus contents/pull-request
  write;
- `release-preparation` and `release` GitHub environments, both restricted to
  protected `main`;
- `release` additionally requires a non-self reviewer and no admin bypass;
- crates.io trusted publisher bound to this repository, the release workflow,
  and the `release` environment;
- immutable releases enabled for future releases;
- tap branch protection requiring its Homebrew validation workflow.

Secrets/variables:

- `RELEASE_APP_ID` — environment variable;
- `RELEASE_APP_PRIVATE_KEY` — environment secret;
- remove `CARGO_REGISTRY_TOKEN` and `HOMEBREW_TAP_TOKEN` only after their
  replacements pass a production release.

## Completion Gate

Every workflow task runs the relevant subset and records results:

```sh
actionlint -no-color
shellcheck -x scripts/*.sh scripts/release/*.sh
scripts/release/test-version.sh
scripts/release/test-homebrew-formula.sh
scripts/release/test-artifacts.sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

GitHub-only acceptance additionally requires native x86/ARM Nix builds,
release-package smoke tests, Homebrew checks on both macOS architectures, and
verification of generated attestations.

## Non-Goals

- New Windows release packages.
- Apple signing/notarization.
- New package formats or registries.
- Rewriting prior tags or releases.
- Changing Kinjo's Rust API or runtime behavior.
