# Custom Commands

Custom commands let `kinjo` turn a discovered service into an action, such as
opening a URL or starting an SSH session. Each command lives in one `.toml` file.

## Locations

User commands are loaded from:

```sh
$XDG_CONFIG_HOME/kinjo/commands/*.toml
```

If `XDG_CONFIG_HOME` is not set, the fallback path is:

```sh
~/.config/kinjo/commands/*.toml
```

System-wide commands are loaded from:

```sh
/etc/kinjo/commands/*.toml
```

Commands bundled with a relocatable install (a release tarball run in place,
or a prefix install such as Homebrew) are loaded relative to the binary:

```sh
<exe_dir>/commands/*.toml
<prefix>/share/kinjo/commands/*.toml   # binary at <prefix>/bin/kinjo
```

Extra directories can be added with `--config-dir`. The flag is repeatable:

```sh
kinjo --config-dir ./team-commands --config-dir ./local-commands
```

Directories are loaded as overlay layers in this order:

1. `<exe_dir>/commands`, then `<prefix>/share/kinjo/commands`
2. `/etc/kinjo/commands`
3. `$XDG_CONFIG_HOME/kinjo/commands` or `~/.config/kinjo/commands`
4. each `--config-dir` in the order given

If a later layer defines a command with the same `metadata.name`, it overrides
the earlier command. Duplicate names within the same directory layer are errors.

## Validate Commands

Use `list-commands` to validate and list the registered commands:

```sh
kinjo list-commands
```

To validate only specific directories:

```sh
kinjo list-commands --config-dir ./commands
```

`list-commands` is a real validator, not just a TOML syntax check. A command
file is compiled the same way it is when the TUI loads it, so everything that
could make a rule impossible to run is reported here rather than when someone
selects the action:

- an empty `metadata.name`, or an empty or whitespace-only `action.command`;
- an unsupported `[match.<field>]` field or predicate kind, and an invalid regex;
- an unknown, empty, nested, or unterminated `{placeholder}`;
- an unterminated quote or a dangling backslash in `action.command`;
- a malformed `requirements` entry.

It fails on the first invalid file and names it. Anything `list-commands`
accepts can be matched and prepared.

Requirements are the one thing it does not check: whether a program is installed
is a property of the machine, not of the file, and it is re-checked each time an
action runs.

## File Format

A command file has three parts:

```toml
[metadata]
name = "open-http"
description = "Open HTTP service in a browser"
requirements = ["xdg-open"]

[match.service_type]
regex = "^_http\\._tcp$"

[action]
description = "Open in browser"
command = "xdg-open http://{hostname}:{port}"
mode = "fork"
```

`metadata.name` is required, must not be empty, and is the stable command
identity used for overlay replacement. `metadata.description` and
`metadata.requirements` are optional; see [Requirements](#requirements).

`action.command` is required and must not be empty or whitespace only. It is the
command template to run for the selected service — a template, not a shell
command line; see [Command Templates](#command-templates). `action.description`
is optional and is shown in the action picker. `action.mode` is required and
must be one of:

- `fork`: spawn the command and return to the TUI.
- `execute`: restore the terminal and replace the TUI process with the command.
- `exec`: alias for `execute`.

## Requirements

`metadata.requirements` lists the programs a command needs. Each entry is
written in exactly one of two forms, after surrounding whitespace is trimmed:

```toml
requirements = ["xdg-open", "browser, optional"]
```

- `<program>` — **mandatory**. If it cannot be found, `kinjo` refuses to run the
  action and reports it on the status line. Nothing is launched.
- `<program>, optional` — **optional**. It is shown in the UI and never blocks
  the action. Use it for a dependency the command can do without.

The `, optional` marker is case-insensitive. Any other suffix, an extra comma,
or an empty program name is a configuration error and the file is rejected. This
is deliberately strict: a typo such as `"browser, optinal"` would otherwise be
read as a *mandatory* requirement named `browser`, silently blocking the action
it was meant to make optional.

A program is looked up the same way the operating system would when starting it:

- a name containing a path separator (`/usr/local/bin/tool`) is used as-is;
- a bare name (`ssh`) is searched for in each `PATH` directory, and must resolve
  to a file with an execute bit set. On Windows, `PATHEXT` extensions are tried
  too, so `cmd` finds `cmd.exe`.

Mandatory requirements are checked immediately before the action runs, not when
the file is loaded, so installing a missing tool takes effect without restarting
`kinjo`.

`kinjo` never installs anything. Requirements describe what a command needs so
that a missing dependency is reported clearly instead of surfacing as a failed
launch; installing it is up to you.

## Matching Services

Each `[match.<field>]` section adds predicates for a service field. All
predicates must match for the command to be offered.

Supported predicates:

- `equals`: exact string match.
- `contains`: substring match.
- `regex`: Rust regular expression.

Supported fields:

- `name`
- `service_type` or `type`
- `domain`
- `hostname`
- `address`
- `port`
- `txt.<key>`

A service may advertise several addresses. Every `[match.address]` predicate is
applied to the *same* single address, so a command matches only if one concrete
address satisfies all of them at once, and it can then only run against such an
address. Predicates that no single address can satisfy together — for example
`contains = "10."` and `regex = ":"` on a dual-stack host — match nothing:

```toml
[match.address]
contains = "10."
regex = "\\.99$"    # only an address matching BOTH is offered
```

Example matching a service type and a TXT record:

```toml
[metadata]
name = "open-printer-admin"
description = "Open printer admin page"
requirements = ["xdg-open"]

[match.service_type]
equals = "_ipp._tcp"

[match.txt.adminurl]
contains = "http"

[action]
description = "Open printer admin"
command = "xdg-open {txt.adminurl}"
mode = "fork"
```

## Command Templates

The same service fields can be used as placeholders in `action.command`:

```toml
command = "ssh {hostname}"
```

Common placeholders:

- `{name}`
- `{service_type}`
- `{type}`
- `{domain}`
- `{hostname}`
- `{address}`
- `{port}`
- `{txt.<key>}`

### Choosing a target

A row may cover several services — a host row collects everything on one host, a
service-type row everything advertising one type — and a service may advertise
several addresses. When you invoke a command there, `kinjo` decides whether to
ask you which one it should act on by looking at *what the command would
actually run*:

- It builds the command for every service and address the rule matches.
- Candidates producing the **identical** command collapse into one, and it runs
  without asking. There is nothing to choose between two identical commands.
- If two candidates produce **different** commands, `kinjo` asks which target to
  use, and runs only the one you pick.

This follows from the command line itself, so **any** placeholder that varies
across a row causes the question — `{hostname}`, `{name}`, `{service_type}`,
`{domain}`, `{port}`, `{address}`, or a `{txt.<key>}`. A command that
interpolates nothing, such as `echo hello`, runs once however many services the
row holds.

For example, `ssh {hostname}` on a service-type row covering two hosts asks
which host, because it would otherwise pick one for you. The same rule on a row
whose services all share one host does not ask, because both candidates would
run the very same command.

A command using `{address}` needs one concrete address. With no
`[match.address]` predicate, every advertised address is offered for selection;
with predicates, only the addresses satisfying all of them are. A service whose
addresses are not (yet) resolved offers no such command at all, rather than
failing once it is run. If the rule constrains the address but does not
interpolate it, every satisfying address builds the same command, so `kinjo`
does not ask.

### Quoting and Escaping

`action.command` is split into an argument vector by `kinjo` and handed straight
to the operating system. It is **never passed through a shell**, so there is no
expansion, no environment substitution, no pipelines (`|`), no redirection
(`>`), and no command chaining (`&&`, `;`). Those characters have no special
meaning; they are ordinary text.

The full grammar:

- Unquoted whitespace separates arguments.
- Single (`'`) and double (`"`) quotes remove their delimiters and preserve
  their contents. The other quote style is literal text inside them, so
  `"it's"` is one argument: `it's`.
- Adjacent quoted and unquoted fragments form **one** argument:
  `user@"{hostname}":22` is a single argument.
- A backslash escapes exactly the next character, inside or outside quotes:
  `one\ arg` is one argument, and `\{` is a literal `{`.
- A quoted empty string is a real, preserved argument: `cmd "" next` passes
  three arguments, the middle one empty.
- A dangling backslash at the end, or an unclosed quote, is an error.

```toml
command = "ssh '{hostname}'"
```

### Placeholders

- `{field}` interpolates a supported service field.
- `{{` emits a literal `{`.
- A lone `}` is literal text, so `echo {hostname}}` ends with a `}`.
- An empty (`{}`), nested (`{a{b}}`), unknown (`{nonexistent}`), or unterminated
  (`{hostname`) placeholder is an error.

### Interpolation Is Safe

Argument boundaries are decided when the command file is loaded, before any
service exists. A discovered value only ever fills in an argument, so it cannot
add, remove, or split one — whatever it contains.

Service names, hostnames, and TXT values come from devices on the network and
are not trusted. A service advertising itself as
`evil" && rm -rf / #` is passed through as one ordinary argument containing
those exact characters. This is why quoting a placeholder is a readability
choice rather than a safety one: `{hostname}` and `'{hostname}'` are equally
safe.

## Examples

SSH into discovered SSH services:

```toml
[metadata]
name = "ssh"
description = "SSH into a service"
requirements = ["ssh"]

[match.service_type]
equals = "_ssh._tcp"

[action]
description = "SSH into the selected service"
command = "ssh {hostname}"
mode = "execute"
```

Open an alternate HTTP port:

```toml
[metadata]
name = "open-http-alt"
description = "Open alternate HTTP service in a browser"
requirements = ["xdg-open"]

[match.service_type]
regex = "^_http-alt\\._tcp$"

[action]
description = "Open in browser"
command = "xdg-open http://{hostname}:{port}"
mode = "fork"
```

Open a service by IP address instead of hostname:

```toml
[metadata]
name = "open-by-address"
description = "Open service by address"
requirements = ["xdg-open"]

[match.service_type]
contains = "_http."

[action]
description = "Open by address"
command = "xdg-open http://{address}:{port}"
mode = "fork"
```

## Parser Notes

Command files are standard TOML. Values must be quoted strings unless the
field expects an array of strings, such as `requirements`. Unknown sections,
unknown metadata/action keys, and unknown predicate kinds are rejected with an
error naming the offending file.

When the TUI starts normally, an invalid command file is skipped with a warning
naming it (shown on the status line and printed on exit) so one bad file — for
example in the shared system directory — cannot prevent the app from starting.
Every invalid file is reported, not just the first, and the valid ones still
load. `kinjo list-commands` loads strictly and fails on the first invalid file;
use it to validate your configuration.
