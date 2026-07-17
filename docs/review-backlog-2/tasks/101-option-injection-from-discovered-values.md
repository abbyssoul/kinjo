# Task 101 — Leading-dash option injection from discovered values

- **Priority**: P0 (trust model)
- **Status**: ready — but **blocked on an owner decision**; see "Decision needed".
- **Depends on**: none
- **Likely conflicts**: 102 (both touch `plumber`)

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
