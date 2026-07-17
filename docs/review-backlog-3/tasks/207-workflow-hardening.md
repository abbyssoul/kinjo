# Task 207 — Pin and bound the workflow supply chain

| Field | Value |
|---|---|
| Status | `in-progress` |
| Priority | `P1` |
| Workstream | Workflow security |
| Depends on | — |
| Likely conflicts | Every workflow task |
| Owner | Codex (repository implementation) |

## Required Outcome

Pin every external action to a verified full SHA with a version comment. Pin
Cargo release/audit/fuzz/coverage/SBOM tools to exact versions. Use a master
history SHA plus explicit dated nightly for fuzzing. Use explicit release
runner labels, minimal permissions, disabled checkout credentials, typed/capped
manual inputs, explicit timeouts, and safe concurrency.

Keep Dependabot updates for pinned actions. Split the shared setup action so
jobs install only the Linux packages they need.

## Required Tests

- a repository search finds no movable external `uses:` reference;
- all tools print the expected pinned version;
- manual fuzz duration rejects out-of-range input;
- actionlint and ShellCheck pass;
- build jobs cannot perform authenticated pushes.

## Completion Record

- **Pins/permissions:** Every external action is pinned to a full commit/digest;
  Cargo tools are exact versions; build checkouts disable persisted credentials;
  publication permissions exist only on the protected publisher.
- **Runtime bounds:** Hosted runner labels and timeouts are explicit, fuzzing
  uses a dated nightly and validates a typed 1–900 second input, and Avahi is
  installed only for jobs that build the zeroconf feature/package.
- **Validation:** Repository searches find no movable `uses:` or unversioned
  workflow tool entry; local actionlint, ShellCheck, helper tests, and the full
  Rust gate pass. Installed tool-version and hosted-runner checks await GitHub.
