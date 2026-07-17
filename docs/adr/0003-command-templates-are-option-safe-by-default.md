# ADR 0003: Command templates are option-safe by default

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-07-17 |
| Decider | Ivan Ryabov (project owner) |
| Context | [review-backlog-2 task 101](../review-backlog-2/tasks/101-option-injection-from-discovered-values.md) |

## Context

Kinjo compiles command templates before discovery data exists. That fixes the
argv shape and prevents a discovered value from adding, removing, or splitting
arguments. It does not stop a launched program interpreting one field-led token
such as `-oProxyCommand=...` as an option.

Automatically inserting `--` is not sound for arbitrary programs: some do not
support it, and a placeholder may intentionally be the value consumed by a
preceding option. Documentation alone would preserve the unsafe default and
leave the shipped SSH rules exposed.

The compiler also previously allowed a field fragment in the program token.
That gave network data authority to select the executable even though command
files are the trusted local configuration that is supposed to request programs.

## Decision

Command templates are option-safe by default:

- The program token (`argv[0]`) must be entirely literal. No discovered field
  may select or partially construct the executable.
- Before a literal `--`, an argument whose first character can come from an
  untrusted text field is rejected. Address and port are exempt because they are
  rendered from typed values and cannot begin with `-`.
- Kinjo never inserts `--`; the trusted rule author writes the terminator in the
  grammar understood by that program.
- A rule whose program has no terminator, or that intentionally places a field
  as an option value, must set `action.allow_option_like_values = true`. The
  name makes the authority being granted explicit. It never relaxes the literal
  executable requirement.
- The shipped SSH rules use `ssh -- {hostname}`.

Validation happens when the command rule is loaded, through the same Interface
used by strict listing, lenient startup, and transactional reload.

## Consequences

Positive:

- Shell injection, argv reshaping, option injection, and executable selection
  now have separate, explicit guarantees.
- Unsafe older command files fail with an actionable remedy before the action
  can be selected.
- Programs with unusual argument grammars remain usable through a narrow,
  auditable opt-out.

Negative:

- Some previously accepted command files require `--`, a literal prefix, or the
  explicit opt-out.
- Kinjo cannot prove that a program treats `--` as a terminator; trusted local
  configuration remains responsible for choosing syntax that its program
  supports.

Implementation note: `CommandAction::prepare` remains fallible because it is a
public Interface and a caller may ask an action to render against an `Entry`
that lacks a referenced field. The matcher proves fields resolve before it
constructs candidates, so its candidate-preparation failure path is defensive
rather than normally reachable; it remains in place to avoid ever dropping one
candidate and silently running another if that invariant regresses.

## Rejected alternatives

- **Documentation only:** rejected because it adds no default safety property.
- **Automatic `--`:** rejected because program grammars differ and silent argv
  rewriting can change correct commands.
- **Prefix or strip leading dashes:** rejected because it mutates discovered
  data and can target the wrong resource.
