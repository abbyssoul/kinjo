# Task 205 — Publish synchronously through an immutable retry-safe release

| Field | Value |
|---|---|
| Status | `blocked` |
| Priority | `P0` |
| Workstream | Publication |
| Depends on | 201, 202, 204 |
| Likely conflicts | 206, 207 |
| Owner | Codex (repository implementation); repository owner (enablement) |

## Required Outcome

Replace the three fire-and-forget publishers with one synchronous job graph.
After protected approval, create/resume a matching draft, upload non-clobbering
assets, attest provenance/SBOM, publish crates.io through OIDC, and publish the
immutable GitHub release. Publication is globally serialized and queued rather
than cancelled.

Each retry probes its checkpoint. Matching SHA/hashes are success; conflicting
tags, drafts, assets, or crates fail closed. Publisher jobs download staged
artifacts and do not compile with repository-write credentials.

## Required Tests

- failures before approval leave no public state;
- identical draft/assets resume, different state fails;
- existing crates.io checksum is accepted only when it matches the candidate;
- completed release rerun verifies and succeeds without mutation;
- parent workflow cannot succeed while required publication jobs fail;
- attestations verify against the repository and exact commit.

## Completion Record

- **Implemented:** One protected synchronous graph now creates/resumes a draft,
  rejects tag/asset/hash conflicts, produces checksums and attestations,
  publishes through crates.io OIDC, verifies the registry checksum, publishes
  the immutable release, and waits for Homebrew. `--clobber` and asynchronous
  publisher dispatches are gone.
- **Correction (review of this task):** the crate that cargo uploads was
  compared against the staged, attested artifact *after* `cargo publish`
  returned, so a mismatch would surface only once the version was permanent on
  crates.io — breaking invariant 5. The job now runs `cargo package` and `cmp`
  first and publishes only the bytes that matched.
- **Recovery scenarios exercised:** Local validators only; external checkpoint
  rehearsals remain blocked on Task 201.
- **Validation:** Actionlint and ShellCheck pass. Protected approval, OIDC,
  immutable release, attestation, and rerun behavior require GitHub acceptance.
