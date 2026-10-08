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
   on tap `main`. `Formula/kinjo.rb` is generated: each tap PR rewrites the
   whole file from `scripts/release/update-homebrew-formula.sh`, so change the
   template there, never the tap copy. The first release rendered by it
   replaces the older formula's Linux source build with the static Linux
   archives; no tap edit is needed.
7. Require the ordinary CI checks on `kinjo`'s protected `main` and disallow
   administrator bypass. Preparation deliberately opens a normal PR and merges
   nothing; without required checks, a red version PR could still be merged by
   hand.
8. After the first publication, confirm that the `ghcr.io/abbyssoul/kinjo`
   container package is public and linked to this repository. The release
   workflow pushes it with `GITHUB_TOKEN`, so no further credential is needed.
9. For the Snap Store: register the `kinjo` name (`snapcraft register kinjo`),
   request classic confinement for it on the Snapcraft forum, and keep a store
   login exported with `snapcraft export-login` in the `SNAPCRAFT_STORE_CREDENTIALS`
   repository secret. Once classic confinement is granted, set the repository
   variable `SNAP_STORE_PUBLISH` to `true`. Until then the snaps are release
   assets only and the upload job is skipped.
10. For the Launchpad PPA (optional; every PPA job is skipped until all three
    values below are set):
    1. Create a PPA on Launchpad, named `kinjo` unless you set `PPA_NAME`. In
       its settings, enable the amd64 and arm64 processors. Those are the
       architectures the release workflow test-builds.
    2. Create a signing key with a passphrase, publish it with `gpg
       --keyserver keyserver.ubuntu.com --send-keys <fingerprint>`, and register
       it at `https://launchpad.net/~<user>/+editpgpkeys`.
    3. Add the ASCII-armoured secret key (`gpg --armor --export-secret-keys
       <fingerprint>`) as the repository secret `PPA_GPG_PRIVATE_KEY` and its
       passphrase as `PPA_GPG_PASSPHRASE`.
    4. Set the repository variable `PPA_USERNAME` to your Launchpad user name,
       and `PPA_NAME` if the PPA is not called `kinjo`.

    A partial configuration is skipped too, with a warning on the run summary
    that names what is missing.

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

Run **Release** from `main` with the merged version and `publish=false`. It does
not matter how much has merged into `main` since the version PR: **Release** pins
itself to the merge commit of the merged `release/v<version>` PR, not to `main`'s
current tip, so unrelated merges (Dependabot and the like) neither change nor
fail the release. That pinned commit is validated as an ancestor of `main`
carrying the requested version, and every job — validation, packaging, and
publication — checks out that exact commit.

The version you release is therefore the state of `main` when the version PR
merged; commits that landed afterward ride the next version. If you must release
a different commit (for example a hand-merged one), pass its full 40-hex `sha`;
it is validated the same way.

The run executes the reusable Rust, audit, Nix, and workflow-lint gates; builds
and executes native packages on Linux x86/ARM and macOS ARM/Intel, plus static
musl archives for Linux x86/ARM, and classic snaps of those binaries, each
installed and run on its runner; builds and smoke-tests the container image
natively on Linux x86/ARM; verifies the crate; stages the SBOM and artifacts internally; and asserts that the staged set
is exactly what the publisher expects. It creates no tag, release, crate, or tap
pull request.

With the PPA configured, the run also builds Debian source packages for each
Ubuntu series in `release-ppa.yml`, crates vendored, and builds each one for
amd64 and arm64 in a clean container of that series with networking cut off,
the way Launchpad does. A dependency raising its minimum Rust beyond the
series' `rustc-1.91` package, for example, fails here instead of in a Launchpad
email. These jobs do not gate the GitHub release.

The dry run and the publisher share `scripts/release/check-artifacts.sh`, so an
artifact naming or packaging mistake fails here rather than after the `release`
environment has been approved.

Record the workflow URL in Task 208. The workflow logs print `Releasing
v<version> from <sha>`; note that commit — reruns and the publish step must
resolve to it. Do not proceed if any architecture was skipped.

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
6. opens or reuses a versioned Homebrew tap PR and waits up to an hour for its
   required checks. The PR's formula installs the macOS and static Linux
   archives, whose digests must match the published `SHA256SUMS`;
7. in parallel with the tap PR, pushes the two smoke-tested images staged by
   the candidate gate as `ghcr.io/abbyssoul/kinjo:<version>-amd64` and
   `-arm64`, joins them into the multi-arch `<version>` and `latest` tags, and
   attests the image's provenance. Images are never rebuilt after approval;
8. with `SNAP_STORE_PUBLISH` set, verifies the staged snaps against the
   published `SHA256SUMS` and uploads them to the Snap Store's stable channel;
9. with the PPA configured, signs the tested source packages, uploads them as
   `<version>-1~<series>1`, and waits up to 30 minutes for Launchpad to accept
   each one. Launchpad then builds them on its own schedule; a failed build is
   reported only by email and on the PPA page.

The workflow is globally serialized and never cancels an active publication.
GitHub Actions retains at most one pending run for a concurrency group, so do
not queue multiple release dispatches while another release is running.

## Recovery

Always retry the same **Release** with the same `version`. Retrying resolves the
same merged-PR commit, so `main` may move freely between attempts. If you pinned
an explicit `sha`, pass the same one again. Never delete or move a tag, replace
an asset, or bump the version solely to recover automation.

| Failure point | External state | Recovery |
|---|---|---|
| Candidate gate or before approval | none | Fix the cause and rerun; use `publish=false` first. |
| Draft creation or asset upload | matching draft/tag and possibly some assets | Rerun. Matching tag SHA and asset bytes are reused; conflicts stop the run. |
| After crates.io publication | public crate plus matching draft | Rerun. The registry checksum is verified, then the draft is published. |
| After GitHub publication | immutable release and crate | Rerun. Public state is verified without replacement, then Homebrew resumes. |
| Container image push | immutable release and crate, possibly some image tags | Rerun. The rerun builds and smoke-tests its own staged images, then pushes them and recreates the tags. |
| Snap Store upload | immutable release and crate, possibly one architecture uploaded | Rerun. An architecture whose stable channel already has the version is skipped. |
| Launchpad PPA upload | immutable release and crate, possibly some series uploaded | Read Launchpad's rejection email, fix the cause, and rerun. A series the PPA already has is skipped. |
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
