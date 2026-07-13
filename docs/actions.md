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

`metadata.name` is required and is the stable command identity used for overlay
replacement. `metadata.description` and `metadata.requirements` are optional.
Requirements are shown in the UI; they are not installed or checked for you.

`action.command` is required. It is the shell command template to run for the
selected service. `action.description` is optional and is shown in the action
picker. `action.mode` is required and must be one of:

- `fork`: spawn the command and return to the TUI.
- `execute`: restore the terminal and replace the TUI process with the command.
- `exec`: alias for `execute`.

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

If a command uses instance-specific fields such as `{address}` or `{port}`, and
the selected row contains multiple service instances, `kinjo` asks which
instance to use before running the command.

Commands are split into an argument vector by `kinjo`; they are not passed
through a shell. Quote arguments that may contain spaces:

```toml
command = "ssh '{hostname}'"
```

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

When the TUI starts normally, a malformed command file is skipped with a
warning (shown on the status line and printed on exit) so one bad file cannot
prevent the app from starting. `kinjo list-commands` loads strictly and
fails on the first invalid file — use it to validate your configuration.
