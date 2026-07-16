# 005: Compile and Validate Command Rules at Load Time

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `in-progress` |
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

- [ ] `list-commands` rejects every unsupported field/placeholder and malformed
      template/requirement with source file and actionable context.
- [ ] Lenient startup skips each invalid file and retains valid rules.
- [ ] Valid quoted empty arguments are preserved exactly in prepared argv.
- [ ] A discovered value containing spaces, quotes, separators, or braces cannot
      reshape argv.
- [ ] UI no longer owns requirement/template preparation ordering.
- [ ] `docs/actions.md` documents validation and quoting accurately.
- [ ] Parser/prepare fuzz targets remain meaningful and pass their smoke runs.
- [ ] Full validation passes.

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
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
