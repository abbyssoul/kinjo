
# Avahi-TUI

<div align="center">

**Browse local DNS-SD services, filter them, and run custom actions from a terminal UI.**

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

Browse another DNS-SD domain with the `--domain` (`-d`) flag:

```sh
avahi-tui --domain example.local
```

For development without a running Avahi setup:

```sh
avahi-tui --fake-discovery
```

The app discovers services over mDNS/DNS-SD so no external CLI tools are
required. Two discovery backends are available, selectable with `--backend`:

- `mdns-sd` (default): the `mdns-sd-discovery` crate. A single browser
  enumerates every service type on the link via the native DNS-SD meta-query.
- `zeroconf`: the `zeroconf-tokio` crate, which talks to the system Avahi daemon
  on Linux. It browses one service type at a time, so a curated set of common
  types is swept in parallel when no `--service-type` is given.

```sh
avahi-tui --backend zeroconf
```

When a `--service-type` is given, only that type is browsed. If mDNS discovery is
unavailable, it falls back to sample records so the UI remains usable.

## Installation

### Debian / Ubuntu

Download the latest `.deb` package from the project's
[GitHub Releases](https://github.com/abbyssoul/avahi-tui/releases) page, then
install it with `apt`:

```sh
sudo apt install ./avahi-tui_*_amd64.deb
```

The package installs `avahi-tui` and the bundled system command files under
`/etc/avahi-tui/commands`.

For real local-network discovery, make sure Avahi is installed and running:

```sh
sudo apt-get update
sudo apt-get install -y avahi-daemon
sudo systemctl enable --now avahi-daemon
```

### Cargo

Install from crates.io with Cargo:

```sh
cargo install avahi-tui
```

On Debian or Ubuntu, install native build dependencies first:

```sh
sudo apt-get update
sudo apt-get install -y clang libavahi-client-dev libxcb-shape0-dev libxcb-xfixes0-dev xorg-dev
```

### Build From Source

Clone the repository and build locally:

```sh
git clone https://github.com/abbyssoul/avahi-tui.git
cd avahi-tui
cargo build --locked
```

Run from the source tree:

```sh
cargo run -- --fake-discovery
```

Install the built binary into Cargo's bin directory:

```sh
cargo install --path .
```

### Smoke Test

You can verify the UI without a running Avahi daemon:

```sh
avahi-tui --fake-discovery
```

To browse real services on the default `local` domain:

```sh
avahi-tui
```

## How It Works

`avahi-tui` has five moving parts:

1. Discovery finds DNS-SD service records on the network. Each record can carry
   fields such as service name, service type, domain, hostname, address, port,
   and TXT values.
2. Filtering and view tabs organize those records in the UI. You can fuzzy-search
   the visible services, limit by service type, and switch the top-panel tab to
   view discovery by service, host, service type, or matching command.
3. Actions decide what can be done with a selected service. Action command files
   define match predicates such as "service type equals `_ssh._tcp`" or "TXT
   field contains a URL".
4. Command templates turn service fields into executable commands. For example,
   `ssh {hostname}` uses the selected service hostname, while
   `xdg-open http://{hostname}:{port}` builds a URL from the selected instance.
5. Keybindings control the TUI. The defaults use Vim-style navigation, and every
   built-in UI command can be rebound in `keybindings.toml`.

The result is a small local service browser that behaves like a configurable
launcher: discover services, narrow the list, choose a matching action, and run
the command built from that service's fields.

### Architecture

Internally those moving parts live in three deliberately decoupled modules, so
the project is easy to extend and hack on. Each is designed to be swapped or
reused independently:

1. **Discovery** (`src/discovery/`) — the producer of *entries*. An entry is a
   discovered record described entirely by its attributes (name, type, host,
   address, port, TXT, …). A backend implements the `Discovery` trait and emits
   `Entry` values; `Entry` is the only contract the rest of the program depends
   on. The mDNS/Avahi backend is the default, with a built-in sample backend for
   `--fake-discovery` — but you could drop in a different DNS-SD source, a static
   file, or an SSDP/UPnP browser without touching anything else.
2. **Plumber** (`src/plumber/`) — the rules engine. A serializable collection of
   command rules (the TOML command files) is matched against entries by their
   attributes; multiple rules can match one entry, and a matching rule can be
   executed. It depends only on `Entry`, never on the UI, and sits behind a
   `RuleEngine` trait so an alternative matching strategy can be substituted.
3. **UI** (`src/ui/`) — ties discovery and the rules engine together for a person
   at the terminal: CLI parsing, config and keymap loading, the application
   state machine, and rendering. It depends on the other two; they do not depend
   on it.

The dependency flow is one-directional — `discovery ← plumber ← ui` — wired
together in `main.rs`. The two trait seams (`Discovery` and `RuleEngine`) are the
intended extension points: implement a trait, swap it in at the composition root,
and experiment. See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup.

## UI

Default keys follow Vim-style conventions:

- `j` / `down`: move down
- `k` / `up`: move up
- `enter`: show or run matching actions
- `/`: fuzzy text filter
- `t`: service type checklist filter
- `tab` / `shift+tab` (or `←` / `→`): switch view tab
- `?`: help
- `q`: quit

The top panel exposes four tabs — services, hosts, types, and commands. Each tab
swaps the list and details panes to view discovery from that angle: individual
services, hosts and the services they offer, discovered service types, or
configured commands and the services they match. The list also supports fuzzy
text search and service type filtering.

Keybindings are fully customizable: all built-in UI commands can be rebound with
a keybindings config file. See [docs/keybindings.md](docs/keybindings.md) for
the full keybinding reference and examples.

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

See [docs/keybindings.md](docs/keybindings.md) for examples and the complete
list of bindable commands.

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

For the full command file format, examples, and overlay rules, see
[docs/actions.md](docs/actions.md).

## Contributing

Development setup, required system packages, and local verification commands are
documented in [CONTRIBUTING.md](CONTRIBUTING.md).
