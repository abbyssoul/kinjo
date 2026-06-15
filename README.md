
# Avahi-TUI

<div align="center">

**TUI browser for service discovery and hackable fuzzy finder.**

![GitHub Release](https://img.shields.io/github/v/release/abbyssoul/avahi-tui?display_name=tag&color=%23a6a)
[![Crates.io](https://img.shields.io/crates/v/avahi-tui.svg)](https://crates.io/crates/avahi-tui)
[![docs.rs](https://img.shields.io/docsrs/avahi-tui)](https://docs.rs/avahi-tui)
[![GitHub branch check runs](https://img.shields.io/github/check-runs/abbyssoul/avahi-tui/main)](https://github.com/abbyssoul/avahi-tui/actions/workflows/ci-test.yml)
[![License: MIT](https://img.shields.io/crates/l/avahi-tui.svg)](LICENSE)

<img width="1430" height="609" alt="avahi-tui-screenshot" src="https://github.com/user-attachments/assets/319acde6-a3d0-4cb1-aefc-12088bc67328" />


</div>

## What's it for?

Avahi, the common Linux implementation of Bonjour / mDNS / DNS-SD, allows services
to be published and discovered on a local network. This TUI lets users browse
discovered services, filter and group them, and launch configured actions for a
selected service.

Launch the TUI without arguments to browse the default `local` domain:

```sh
avahi-tui
```

Browse another DNS-SD domain by passing it as the positional argument:

```sh
avahi-tui example.local
```

For development without a running Avahi setup:

```sh
avahi-tui --fake-discovery
```

The app discovers services over mDNS/DNS-SD using the `zeroconf-tokio` crate
(which talks to the system Avahi daemon on Linux), so no external CLI tools are
required. When a `--service-type` is given, only that type is browsed; otherwise
a curated set of common service types is swept in parallel. If mDNS discovery is
unavailable, it falls back to sample records so the UI remains usable.

## UI

Default keys follow Vim-style conventions:

- `j` / `down`: move down
- `k` / `up`: move up
- `enter`: show or run matching actions
- `/`: fuzzy text filter
- `t`: service type checklist filter
- `g`: grouping selector
- `?`: help
- `q`: quit

The service list supports fuzzy text search, service type filtering, and runtime
grouping by logical service, host, service type, port, or address.

## Configuration

Command files follow the XDG Base Directory Specification. User command files are
loaded from:

```sh
$XDG_CONFIG_HOME/avahi-tui/commands/*.toml
```

If `XDG_CONFIG_HOME` is not set, the fallback path is:

```sh
~/.config/avahi-tui/commands/*.toml
```

Additional command directories can be provided with:

```sh
avahi-tui --config-dir ./commands
```

Validate and list the registered commands with:

```sh
avahi-tui list-commands
```

To validate and list only the commands from a specific directory:

```sh
avahi-tui list-commands --config-dir ./commands
```

Keybindings can be overridden at:

```sh
$XDG_CONFIG_HOME/avahi-tui/keybindings.toml
```

## Command Files

Each command file defines one action and structured match predicates. Example SSH
opener:

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

Supported action modes:

- `fork`: spawn the command and return to the TUI.
- `execute`: restore the terminal and replace the TUI process with the command.

Supported match predicates:

- `equals`
- `contains`
- `regex`

Supported service fields:

- `name`
- `service_type` or `type`
- `domain`
- `hostname`
- `address`
- `port`
- `txt.<key>`

The same fields can be used in action command interpolation, for example
`{hostname}`, `{address}`, and `{port}`.

Multiple configured actions can match the same service. In that case, the TUI
shows an action picker. If an action needs instance-specific fields such as
`address` or `port` and the selected row contains multiple instances, the TUI
asks which exact instance to use.
