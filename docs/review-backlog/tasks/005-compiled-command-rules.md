# 005: Compile and Validate Command Rules at Load Time

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P1` |
| Workstream | Command rules |
| Depends on | 004 (`done`) |
| Likely conflicts | 006, 007, 015 |
| Owner | agent-af100d3ff63576edf (branch `worktree-agent-af100d3ff63576edf`) |

## Why This Matters

`list-commands` is presented as a validator, but unknown fields/placeholders,
empty commands, malformed command-line quoting, and malformed requirements can
survive loading and fail only when selected. Parsing, matching, requirement
handling, tokenization, interpolation, and launch ordering are spread across
three modules and the UI.

Deepen the command-rule module: loading should produce a valid executable rule
whose interface owns matching, candidate generation, and preparation. That gives
startup, validation, reload, UI, and tests one source of truth.

## Evidence

- `src/plumber/config.rs:247-286`: predicate paths are collected without checking
  supported entry fields.
- `src/discovery/entry.rs:176-190`: unknown fields simply return `None` later.
- `src/plumber/exec.rs:16-31`: templates are tokenized/interpolated at execution.
- `src/plumber/exec.rs:170-197`: placeholders are discovered only during runtime
  interpolation.
- `src/plumber/exec.rs:199-235`: tokens are emitted only when text is non-empty;
  quoted empty arguments disappear and a final backslash is silently consumed.
- `src/plumber/exec.rs:89-117`: requirements are parsed only during execution.
- `src/ui/app.rs:695-725`: UI knows the ordering requirements → prepare → mode
  → launch/handoff.
- `docs/actions.md:50-60`: `list-commands` is documented as validation.
- `docs/actions.md:83-85`: documentation says requirements are not checked, while
  runtime currently blocks missing mandatory executables found through PATH lookup.

## Required Outcome

- Command-file loading compiles raw TOML into validated command rules.
- Reject unsupported predicate fields except arbitrary non-empty `txt.<key>` fields.
- Reject unknown/malformed placeholders, empty metadata names, empty executable
  commands, malformed quotes/escapes, and malformed requirement markers.
- Use this exact non-shell token grammar: unquoted whitespace separates arguments;
  single and double quotes remove their delimiters and preserve their contents;
  adjacent quoted/unquoted fragments form one argument; backslash escapes exactly
  the next Unicode scalar inside or outside quotes; a dangling backslash or open
  quote is invalid. Preserve quoted empty arguments, including at the end.
- Use this exact placeholder grammar: `{field}` references a supported field;
  `{{` emits a literal `{`; a lone `}` remains literal for compatibility; an empty,
  nested, unknown, or unterminated placeholder is invalid.
- Use this exact requirement grammar after trimming: `<nonempty program>` or
  `<nonempty program>, optional`. Reject extra commas and every other suffix.
- A record missing a supported field referenced by a predicate/template produces
  no executable candidate; the action is not offered for that record.
- Store parsed requirements and command-template tokens so matching/preparation do
  not reinterpret raw strings on each invocation.
- Keep interpolation safe: discovered values cannot change argument count or token
  boundaries.
- The command-rule interface exposes validated metadata for rendering and one
  preparation operation for a chosen candidate. The UI should not reimplement
  requirement/template ordering.
- `list-commands` fails strictly; normal startup remains lenient and reports every
  invalid file as a warning.

## Implementation Constraints

- Preserve documented TOML and interpolation syntax for valid configurations.
- Avoid exposing the compiled representation's implementation details to UI.
- Keep process spawning/exec platform behavior behind the command-rule module.
- Do not retain both raw and compiled forms unless raw text is genuinely needed
  for display; displayed command text may be preserved as metadata.
- Apply the deletion test to helper modules and the `RuleEngine` trait, but defer
  removal of the public seam to task 015 to limit this task's blast radius.
- Extend relevant fuzz targets/oracles for load-time validation and argv shape.

## Suggested Implementation Sequence

1. Add strict/lenient validation regressions for every rejected form.
2. Extract a compiled template tokenizer that preserves token-start state.
3. Parse/validate fields, placeholders, and requirements in `MatcherBuilder`.
4. Move candidate preparation and requirement checking onto the validated rule.
5. Simplify UI execution ordering to consume the rule's result.
6. Update action documentation with the exact non-shell quoting/brace grammar,
   mandatory versus optional requirements, executable/PATH lookup, blocking
   behavior, and the fact that Kinjo does not install dependencies.

## Non-Goals

- Changing overlay precedence or command identity rules.
- Adding shell expansion, environment interpolation, pipelines, or redirects.
- Transactional SIGHUP policy; task 007 consumes the validated loader.
- Removing `RuleEngine`; task 015 owns final seam cleanup.

## Acceptance Criteria / Definition of Done

- [x] `list-commands` rejects every unsupported field/placeholder and malformed
      template/requirement with source file and actionable context.
- [x] Lenient startup skips each invalid file and retains valid rules.
- [x] Valid quoted empty arguments are preserved exactly in prepared argv.
- [x] A discovered value containing spaces, quotes, separators, or braces cannot
      reshape argv.
- [x] UI no longer owns requirement/template preparation ordering.
- [x] `docs/actions.md` documents validation and quoting accurately.
- [x] Parser/prepare fuzz targets remain meaningful and pass their smoke runs.
- [x] Full validation passes.

## Required Tests

- Unsupported `service_typ`; valid `type` alias and arbitrary `txt.<key>`.
- Unknown, empty, nested, and unterminated placeholders.
- Empty name/command and whitespace-only executable.
- `cmd "" next`, `cmd ''`, adjacent quoted fragments, escaped whitespace.
- Dangling backslashes and unterminated quotes.
- Valid/invalid mandatory and `, optional` requirements.
- Strict failure versus lenient warnings with source paths.

## Validation

```sh
cargo test --locked plumber
cargo test --locked ui::config
cargo run --locked -- list-commands --config-dir actions
# Run relevant fuzz smoke targets per CONTRIBUTING.md when available.
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
  - New `src/plumber/template.rs`: `CommandTemplate`, the compiled argv *shape*
    of an action. `compile` implements the exact token grammar (unquoted
    whitespace separates; quotes are removed and contents preserved; adjacent
    fragments form one argument; backslash escapes the next scalar inside and
    outside quotes; dangling backslash and unterminated quote are invalid;
    quoted empty arguments are preserved, including at the end) and the exact
    placeholder grammar (`{field}`, `{{` → literal `{`, lone `}` literal,
    empty/nested/unknown/unterminated invalid). Tokenizing and placeholder
    parsing are one pass — separating them would make `\{name}` (an escaped
    literal brace) indistinguishable from a placeholder.
  - `plumber::is_supported_field` defines the rule vocabulary (`name`, `type`,
    `service_type`, `domain`, `hostname`, `address`, `port`, non-empty
    `txt.<key>`). Placed in `plumber`, not `discovery`, because it is the rule
    language's vocabulary — and `src/discovery/**` belongs to task 003.
  - `CommandAction` now holds a private compiled `template` and is constructible
    only via `CommandAction::compile`, so an action that exists is one that can
    be prepared. `command` is retained solely as display metadata (explicitly
    permitted by the constraints); `prepare` never re-reads it.
  - `Requirement::parse` implements the exact requirement grammar; rules store
    `Vec<Requirement>`, never raw strings. `Display` round-trips the grammar so
    the UI can render dependencies without keeping the raw text alongside.
  - `CommandConfig::run` is the single execution operation: requirements →
    prepare → mode → fork/handoff, returning `ActionOutcome::{Forked, Handoff}`.
  - `CommandConfig::candidates` now also requires every non-address field the
    template names to resolve, so a record missing one yields no candidate and
    the action is not offered (rather than offered and then refused).
  - `needs_instance`/`uses_address` are structural (`template.references(..)`)
    instead of `contains("{address}")` on raw text, so a literal `{{address}}`
    is no longer mistaken for an instance-specific template.
  - Error attribution moved to the one boundary that knows the file
    (`MatcherBuilder::add_str`), and `add_file` now names the path on read
    failure — every lenient-startup warning names its source.
- **Tests added/updated:**
  - `src/plumber/template.rs`: 14 grammar tests covering every token and
    placeholder rule above, plus `field_values_cannot_reshape_argv`.
  - `src/plumber/exec.rs`: requirement grammar (valid forms, every rejected
    suffix, extra commas, empty program, `Display` round-trip).
  - `src/plumber/mod.rs`: 9 new load-time rejection cases (`service_typ`, bare
    `txt`, empty name/command, whitespace-only command, unknown placeholder,
    unterminated quote, malformed requirement) — each now also asserting the
    error names its source; `type` alias + arbitrary `txt.<key>` accepted;
    `supported_fields_resolve_against_a_populated_record` pins the vocabulary
    against `Entry::field`; missing templated field/TXT key yields no candidate;
    `run` hand-off/fork/requirement-block; injection barrier and quoted-empty
    argv through the real loading interface; strict-vs-lenient with source paths
    and "every invalid file warned".
  - `src/ui/app.rs`: removed `BAD_TEMPLATE` — `echo {nonexistent_field}` can no
    longer be loaded at all, which is the point of the task; the picker-closing
    regression now uses the missing-binary rule, which still fails at run time.
- **Documentation updated:** `docs/actions.md` — new Requirements section
  (mandatory vs optional, PATH/executable lookup incl. `PATHEXT`, blocking
  behaviour, checked per run not at load); Quoting/Escaping and Placeholders
  sections stating the exact grammar; "Interpolation Is Safe"; an accurate
  `list-commands` validator contract; lenient-startup wording.
  - **Contradiction resolved** (evidence `docs/actions.md:83-85`): the docs said
    requirements "are not installed or checked for you" while the runtime
    blocked missing mandatory executables. Resolved in favour of the actual
    intended behaviour — mandatory requirements *are* checked via PATH lookup
    and block the action; optional ones never do; kinjo never installs anything.
    Documented explicitly rather than left implicit.
- **Validation evidence:**
  - `cargo fmt -- --check`: clean.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`: clean.
  - `cargo test --locked --all-targets`: 256 passed (from 245).
  - `cargo test --locked --all-targets --all-features`: 262 passed.
  - `cargo run --locked -- list-commands --config-dir actions`: all 5 bundled
    rules still validate and list.
  - End-to-end strict rejection confirmed by hand: `service_typ` →
    "unsupported match field `service_typ`; supported fields are …",
    `{hostnam}` → "unknown service field `hostnam` in `ssh {hostnam}`",
    `"browser, optinal"` → "unsupported suffix `optinal`" — each prefixed with
    the offending file path, exit code 1.
  - Fuzz smoke runs (nightly, 25s each), no crashes: `parse_command` 423,961
    runs; `command_roundtrip` 83,070 runs; `prepare_command` 372,057 runs.
- **Follow-ups:**
  - **Task 015 — `RuleEngine` deletion test applied, seam confirmed shallow.**
    Evidence: exactly one implementation (`impl RuleEngine for Matcher`,
    `src/plumber/mod.rs:355`), whose three methods are pure pass-throughs to
    `Matcher`'s inherent methods. Its only cost is indirection: `Box<dyn
    RuleEngine>` in `App::matcher` and `ConfigLoader` (`src/ui/app.rs:32,58`),
    plus four `as Box<dyn RuleEngine>` casts in tests and one in `src/lib.rs:49`.
    Deleting the trait makes that complexity vanish with no behaviour lost —
    a hypothetical seam by CONTEXT's definition. Not removed here: task 015 owns
    it and this task's blast radius was already wide.
  - **Pre-existing, out of scope:** `fuzz/fuzz_targets/discovery_entry.rs` does
    not compile on `main` — it imports `kinjo::discovery::group_entries`, which
    the merged discovery work renamed to `browse_groups`. Confirmed present
    before this change (`git stash` check). The fuzz crate is not in the default
    workspace, so the completion gate never catches it. Belongs to the discovery
    workstream (task 003 owns `src/discovery/**`); worth its own small task so
    CI's fuzz soak is not silently broken.
  - `src/ui/render.rs` needed three unavoidable lines (a `Requirement` render
    helper and two test-fixture constructions) because `requirements` changed
    type and `CommandAction` gained a private field. Kept minimal and far from
    the keybinding-hint code task 013 is editing.
