# Task 101 — Leading-dash option injection from discovered values

- **Priority**: P0 (trust model)
- **Status**: done
- **Depends on**: none
- **Likely conflicts**: 102 (both touch `plumber`)

> **Reconciliation (2026-07-17).** The owner decision this task asked for has
> been made and recorded in
> [ADR 0003](../../adr/0003-command-templates-are-option-safe-by-default.md).
> **The ADR supersedes the "Decision needed", "Scope", "Tests", and "Definition
> of Done" sections below**, which are retained only as the record of how the
> question was posed. Read the ADR first, then the
> "Implementing ADR 0003" section at the foot of this file. The earlier text
> recommends option 3 (author-written `--`); the owner chose something stronger
> than any option listed — default rejection with an explicit opt-out.

## Problem

The interpolation barrier is doing exactly what round 1 built it to do: a
discovered value can fill one argument token but can never add, remove, or
split arguments (`src/plumber/template.rs`, and the fuzz oracle
`fuzz/fuzz_targets/prepare_command.rs`). That defeats *shell* injection.

It does **not** defeat *option* injection. A value that lands in one argument
is still passed to the launched program, which may interpret an argument
beginning with `-` as an option rather than data. DNS-SD names, hostnames, and
TXT values all originate from untrusted devices on the link
(`CONTEXT.md` → Trust and Safety Model), and a hostile device controls them.

Concrete case, using the `ssh {hostname}` rule shipped in `docs/actions.md`:

- A device advertises an `_ssh._tcp` service whose resolved hostname is
  `-oProxyCommand=calc`.
- The rule matches, `CommandAction::prepare` renders
  `argv = ["ssh", "-oProxyCommand=calc"]` — one token, exactly as designed.
- `plumber::exec::exec` execs `ssh` with that argv. `ssh` parses
  `-oProxyCommand=…` as an option and runs the attacker's command.

The barrier held (no extra argument was created) and the outcome is still code
execution chosen by a device on the network. The same shape threatens any rule
whose program takes options: `curl {txt.url}` with a `-o`/`--config` value,
`ping {address}` is safe only because addresses are validated, etc.

This is a genuinely subtle, defensible gap — the current design is *correct
about what it claims*. What it needs is an explicit decision about whether
"one token" is a strong enough guarantee, recorded where the next reader will
find it, rather than being left implicit.

## Decision needed (owner)

Pick one; the choice determines the implementation and should land as an ADR
under `docs/adr/`:

1. **Accept and document.** Declare that kinjo runs the programs a user's
   rules name with the values a device advertises, and that rule authors are
   responsible for `--` or equivalent. Cheapest; changes only docs and adds a
   test pinning the current behaviour. Weakest for a tool whose entire input
   is untrusted-by-definition.
2. **Per-field option-safety, opt-out.** Prefix a rendered field value with a
   configurable guard when it would otherwise start with `-` — but only
   `argv` positions the rule did not intend as options. This cannot be done
   safely field-by-field without knowing the program's option syntax, so in
   practice it means (3).
3. **Rule-level `--` support (recommended).** Let a rule declare an
   options-terminator so an author writes `ssh -- {hostname}` (or the template
   grammar treats a bare `--` token as "everything after me is data"). This is
   the standard POSIX remedy, keeps the decision with the rule author who
   knows the program, and is a small, testable template change. It does *not*
   silently rewrite anyone's argv.

The recommendation is (3): it is the mechanism every affected program already
documents, it is opt-in per rule, and it does not pretend kinjo can know an
arbitrary program's flag grammar.

## Scope (assuming option 3)

- Teach the template/`prepare` path that a literal `--` token is a real
  options terminator when the author writes one — i.e. document and test that
  `ssh -- {hostname}` renders `["ssh", "--", "-oProxyCommand=…"]`, which it
  already does, so the code change may be *only* documentation + tests + a
  worked example in `docs/actions.md`'s security notes. Confirm this by test
  before adding code.
- If the owner wants kinjo to be safe *by default* rather than by author
  discipline, that is a larger change (auto-insert `--` before the first
  interpolated token unless the rule opts out); record that as a follow-up
  task rather than expanding this one silently.

## Out of scope

- Address predicates/values (already validated to be IP addresses).
- Any change to the shell-injection barrier, which is correct.

## Tests

- Regression through the matcher's real loading interface: a rule
  `ssh -- {hostname}` against a hostname `-oProxyCommand=x` renders
  `["ssh", "--", "-oProxyCommand=x"]`; the same rule *without* `--` renders
  `["ssh", "-oProxyCommand=x"]` (pinning the documented, author-responsible
  behaviour under option 3 / option 1).
- Extend `fuzz/fuzz_targets/prepare_command.rs`'s oracle only if the code path
  changes what a prepared argv may contain.

## Definition of Done

- An ADR under `docs/adr/` records the decision and its rationale, and is
  linked from `CONTEXT.md`'s Trust and Safety Model.
- `docs/actions.md` gains a "leading-dash values" security note with a worked
  example.
- Regression tests as above.
- Completion gate green.

## Follow-up validation note (2026-07-17)

**Finding confirmed, but this task is not ready to implement unchanged.** A
literal `--` is already an ordinary template token, so option 3 as scoped above
adds documentation and tests but does not change default behaviour or the
shipped SSH rules. An owner must therefore choose one of these coherent
outcomes:

- keep P0 and require a default-safe implementation plus updates to shipped
  rules; or
- explicitly accept author responsibility, document it in an ADR, and lower
  the priority because no code-level safety property was added.

The illustrative hostile *hostname* may depend on what the discovery adapter
accepts as a hostname; the general issue remains because service names and TXT
values are network-controlled and can occupy option-sensitive argv positions.

Also include interpolation into `argv[0]` in the trust-model decision.
`CommandTemplate` deliberately permits a field fragment in the program token,
which lets discovered data choose the executable. Either prohibit that shape
or document it as authority intentionally granted by trusted local config.

Finally, normalise the task status: the file says both `ready` and "blocked on
an owner decision", while the index says `ready`. Use `blocked` until that
decision is recorded.

## Implementing ADR 0003 (2026-07-17)

The decision is recorded; this section is what to build. It supersedes
"Decision needed", "Scope", "Tests", and "Definition of Done" above.

### What the ADR decided

Templates are option-safe **by default**, enforced at load time:

- `argv[0]` must be entirely literal — no field fragment may select or partly
  construct the executable.
- Before a literal `--`, a token whose **first character can come from an
  untrusted text field** is rejected. `address` and `port` are exempt: they
  render from typed values (`IpAddr`/`u16`) that cannot begin with `-`.
- Kinjo never inserts `--`; the rule author writes it.
- `action.allow_option_like_values = true` is the explicit, narrow opt-out. It
  never relaxes the literal-`argv[0]` rule.
- Shipped rules use `ssh -- {hostname}`.

Note the check is **static**: "first character can come from a field" is
decidable at compile time by asking whether the token's first `Fragment` is a
`Field`. `user@{hostname}` stays legal (first char is `u`); a bare
`{hostname}` before any `--` does not. Do not implement this as a runtime scan
of rendered values — that would move a load-time guarantee onto the hot path
and lose the "reject before it can be selected" property.

### Existing tests this decision obsoletes

Three tests encode the shape ADR 0003 now prohibits. They must be updated or
removed as part of this task, not left to fail:

- `template.rs::a_placeholder_program_name_compiles` — asserts
  `{hostname} --flag` compiles. Now rejected; invert it.
- `lib.rs::failed_placeholder_program_is_raw_for_exec_but_safe_in_final_stderr`
  — builds a rule with `command = "{hostname} --flag"`. Needs a different
  vehicle for its real subject (exec failure text stays raw for exec but is
  escaped in the final report). Pick a literal program that does not exist.
- `plumber/mod.rs::a_candidate_that_cannot_prepare_is_offered_rather_than_skipped`
  — uses `{name} --version` so an empty name renders an empty program. That
  rule no longer loads. See below before rewriting it.

Also update `CommandTemplate::compile`'s doc comment, which currently states
the opposite policy ("A placeholder program name is allowed: it is only
knowable per record, and `CommandAction::prepare` checks the rendered
result").

### Watch for: `prepare` may become infallible

With `argv[0]` literal and `CommandConfig::template_fields_resolve` already
gating candidates on every non-address field resolving, there may be no
remaining way for `CommandAction::prepare` to fail. If so, three things become
dead:

- `prepare`'s `if argv[0].is_empty()` check (`plumber/mod.rs`),
- `distinct_targets`' `Vec<Option<PreparedCommand>>` and its `.ok()`,
- the "a candidate that cannot prepare is kept as a target rather than
  dropped" rule and its test.

That last one is deliberate round-1 work (task 006: "a failing candidate must
not hand its turn to another service"). **If ADR 0003 subsumes it, say so
explicitly** — in this task's completion record and in a note on ADR 0003 —
rather than deleting it quietly. If a genuine remaining failure mode exists,
keep the machinery and re-point the test at that mode. Either outcome is fine;
an unexamined deletion is not.

### Compatibility

This rejects input that was **valid, supported, and possible** — merely
unsafe. `CONTEXT.md`'s compatibility constraint currently permits rejection
only of input that was "malformed, unsupported, or semantically impossible",
so this task must widen that wording to cover a deliberate safety rejection,
citing ADR 0003.

Consequence to handle, not just note: startup loads leniently, so an upgrading
user's now-invalid rules are **skipped with a warning printed on exit** — their
commands silently stop appearing. Decide and record whether that is acceptable
or whether this rejection deserves louder treatment (it is the one case where a
skipped file is the user's own working config breaking under an upgrade).
`docs/release-notes.md` needs a migration entry either way.

### Definition of Done (supersedes the one above)

- Load-time enforcement of the four ADR 0003 rules, with actionable errors
  naming the remedy (`--`, a literal prefix, or the opt-out).
- `action.allow_option_like_values` parsed, validated, and documented in
  `docs/actions.md` alongside a "leading-dash values" security note.
- Shipped `actions/*.toml` updated; `ssh -- {hostname}` verified end to end.
- The three obsoleted tests resolved, and the `prepare`-infallibility question
  answered in the completion record.
- `CONTEXT.md` compatibility wording widened; `docs/release-notes.md` migration
  entry added.
- `fuzz/fuzz_targets/prepare_command.rs`'s oracle extended: it now has a real
  invariant to assert — no rendered `argv[0]` contains discovered text, and no
  pre-`--` argument begins with `-` unless the rule opted out.
- Completion gate green.

## Completion Record (2026-07-17)

- Implemented ADR 0003 in `CommandTemplate`: `argv[0]` is literal, field-led
  pre-terminator arguments are rejected by default, typed address/port fields
  remain safe, and `allow_option_like_values = true` is the explicit opt-out.
- Updated shipped SSH rules, integration fixtures, action documentation,
  compatibility language, release migration notes, and the fuzz oracle. The
  fake-backend smoke run produced `ssh -- <hostname>` end to end.
- `prepare` remains fallible through its public Interface because callers may
  provide an entry missing a referenced field. Matcher candidates prove fields
  resolve; their failure-preserving path remains defensive, as recorded in ADR
  0003.
- Completion gate and `cargo check --manifest-path fuzz/Cargo.toml --bin
  prepare_command` passed.
