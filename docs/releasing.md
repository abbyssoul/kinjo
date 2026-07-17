# Releasing Kinjo

Kinjo releases use two manual workflows. Preparation opens a normal version
pull request; publication runs only after that pull request is reviewed,
checked, and merged. Never create or push a release tag by hand.

## One-time repository setup

Complete and record Task 201 in
[`review-backlog-3`](review-backlog-3/tasks/201-release-trust-foundation.md)
before enabling the workflows:

1. Install a release GitHub App on `abbyssoul/kinjo` and
   `abbyssoul/homebrew-abyss`. Grant metadata read and repository contents and
   pull-request read/write; do not grant administration or workflow write.
2. Create a `release-preparation` environment restricted to protected `main`.
   Add `RELEASE_APP_ID` as an environment variable and
   `RELEASE_APP_PRIVATE_KEY` as an environment secret.
3. Create a `release` environment restricted to protected `main`, require a
   non-self reviewer, and prevent administrators from bypassing the rule.
4. Configure the crates.io trusted publisher for owner `abbyssoul`, repository
   `kinjo`, workflow `release.yml`, and environment `release`.
5. Enable immutable GitHub releases for new releases.
6. Copy the [tap check workflow](review-backlog-3/homebrew-tap-check.yml) to
   `homebrew-abyss/.github/workflows/kinjo.yml` and make its two jobs required
   on tap `main`. `Formula/kinjo.rb` already carries the three
   `# kinjo-*-sha256` comments the previous workflow relied on, so it needs no
   hand edit: the first tap PR migrates its source URL from the generated tag
   archive to the uploaded `kinjo-<version>.tar.gz` asset automatically.
7. Require the ordinary CI checks on `kinjo`'s protected `main` and disallow
   administrator bypass. Preparation deliberately opens a normal PR and merges
   nothing; without required checks, a red version PR could still be merged by
   hand.

Keep the legacy crates.io and tap tokens until the first production run proves
OIDC and App authentication. They are not referenced by the new workflows and
must be revoked after that run.

## Prepare a version

1. Ensure `main` contains every intended change and the release notes.
2. Run **Prepare Release** from `main` with a stable `MAJOR.MINOR.PATCH` version
   and no leading `v`.
3. Review the resulting `release/v<version>` pull request. It may change only
   `Cargo.toml` and `Cargo.lock`.
4. Wait for the normal required checks and merge through the ordinary protected
   branch path. Do not use an administrator bypass.

A repeat dispatch reuses an identical open branch/PR and rejects different
content, an existing tag/release, a non-increasing version, or a non-`main`
dispatch.

## Rehearse without publishing

Run **Release** from `main` with the merged version and `publish=false`. This
executes the reusable Rust, audit, Nix, and workflow-lint gates; builds and
executes native packages on Linux x86/ARM and macOS ARM/Intel; verifies the
crate; stages the SBOM and artifacts internally; and asserts that the staged set
is exactly what the publisher expects. It creates no tag, release, crate, or tap
pull request.

The dry run and the publisher share `scripts/release/check-artifacts.sh`, so an
artifact naming or packaging mistake fails here rather than after the `release`
environment has been approved.

Record the workflow URL in Task 208. Do not proceed if any architecture was
skipped or if the workflow commit is no longer the current `main` commit.

## Publish

Run **Release** again from `main` with the same version and `publish=true`.
Confirm the exact commit and version at the protected `release` approval. The
workflow then:

1. reruns the complete candidate gate;
2. creates or resumes a matching draft and uploads only new or byte-identical
   assets plus `SHA256SUMS`;
3. records provenance and CycloneDX SBOM attestations;
4. publishes crates.io through its short-lived trusted-publisher token and
   verifies the registry checksum;
5. publishes the immutable GitHub release; and
6. opens or reuses a versioned Homebrew tap PR and waits for required checks.

The workflow is globally serialized and never cancels an active publication.
GitHub Actions retains at most one pending run for a concurrency group, so do
not queue multiple release dispatches while another release is running.

## Recovery

Always retry the same **Release** input from the same unchanged `main` commit.
Never delete or move a tag, replace an asset, or bump the version solely to
recover automation.

| Failure point | External state | Recovery |
|---|---|---|
| Candidate gate or before approval | none | Fix the cause and rerun; use `publish=false` first. |
| Draft creation or asset upload | matching draft/tag and possibly some assets | Rerun. Matching tag SHA and asset bytes are reused; conflicts stop the run. |
| After crates.io publication | public crate plus matching draft | Rerun. The registry checksum is verified, then the draft is published. |
| After GitHub publication | immutable release and crate | Rerun. Public state is verified without replacement, then Homebrew resumes. |
| Tap branch/PR/checks | immutable upstream release plus partial tap state | Fix the tap check or matching version branch and rerun. A conflicting branch fails closed. |

If crates.io contains the version with another checksum, a tag targets another
commit, or an existing asset has different bytes, stop and investigate. Those
states are deliberately not repaired automatically.

After publication, verify the release assets against `SHA256SUMS`, check the
crate version, and verify an artifact attestation, for example:

```sh
gh attestation verify kinjo-X.Y.Z-aarch64-apple-darwin.tar.gz \
  --repo abbyssoul/kinjo
```

Record the production workflow, release, crates.io version, attestation, and
tap PR links in Task 208 before revoking legacy credentials.
