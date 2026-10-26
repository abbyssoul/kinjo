# Kinjo

<div align="center">

**Browse local DNS-SD services, filter them, and run custom actions from a terminal UI.**

[![GitHub Release](https://img.shields.io/github/v/release/abbyssoul/kinjo?display_name=tag&color=%23a6a)](https://github.com/abbyssoul/kinjo/releases/latest)
[![Crates.io](https://img.shields.io/crates/v/kinjo.svg)](https://crates.io/crates/kinjo)
[![docs.rs](https://img.shields.io/docsrs/kinjo)](https://docs.rs/kinjo)
[![GitHub branch check runs](https://img.shields.io/github/check-runs/abbyssoul/kinjo/main)](https://github.com/abbyssoul/kinjo/actions/workflows/ci-test.yml)
[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=abbyssoul_kinjo&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=abbyssoul_kinjo)
[![License: MIT](https://img.shields.io/github/license/abbyssoul/kinjo)](LICENSE)

</div>

<img width="1430" height="609" alt="kinjo-screenshot" src="https://github.com/user-attachments/assets/319acde6-a3d0-4cb1-aefc-12088bc67328" />

## Why Kinjo?

Your network already advertises SSH servers, printers, device web interfaces,
and development services over mDNS/DNS-SD. Finding those services and turning
their hostnames and ports into a useful command can still take several steps.

Kinjo brings discovery and actions together in a terminal: find a service,
narrow the list, and open a connection or web interface without memorizing
addresses or leaving your keyboard.

- Browse services by name, host, type, or matching command.
- Narrow a busy network with fuzzy search and service-type filters.
- Launch configurable actions such as SSH or opening a web interface.
- Discover through your operating system's DNS-SD stack.
- Keep everything local: no telemetry, account, or cloud service.

Kinjo supports **macOS, Linux, and Windows**.

[Installation](#installation) · [Getting started](#getting-started) ·
[Configuration](#configuration) · [Documentation](#documentation) ·
[Contributing](#contributing)

## Installation

For most users, choose **Homebrew on macOS or Linux**, or the **PPA on Ubuntu**.
Both install Kinjo and its default action commands without a Rust toolchain.

| Platform | Installation |
|---|---|
| macOS | [Homebrew](#homebrew-macos-and-linux) or [release archive](docs/installation.md#download-a-release-macos-and-linux) |
| Ubuntu | [PPA](#ubuntu-ppa) or [`.deb` package](docs/installation.md#debian--ubuntu) |
| Debian | [`.deb` package](docs/installation.md#debian--ubuntu) |
| Other Linux | [Homebrew](#homebrew-macos-and-linux) or [release archive](docs/installation.md#download-a-release-macos-and-linux) |
| Windows | [Cargo](docs/installation.md#cargo-advanced) or [build from source](docs/installation.md#build-from-source) |

### Homebrew (macOS and Linux)

With [Homebrew](https://brew.sh/) installed, run:

```sh
brew install abbyssoul/abyss/kinjo
```

This adds the [`abbyssoul/abyss`](https://github.com/abbyssoul/homebrew-abyss)
tap automatically and installs a prebuilt binary for Apple silicon or Intel
macOS, or x86_64 or ARM64 Linux. To update later, run `brew update` followed by
`brew upgrade kinjo`.

### Ubuntu (PPA)

Add the [Kinjo PPA](https://launchpad.net/~abbyssoul/+archive/ubuntu/ppa), then
install the package:

```sh
sudo add-apt-repository ppa:abbyssoul/ppa
sudo apt update
sudo apt install kinjo
```

If `add-apt-repository` is unavailable, install it first with
`sudo apt install software-properties-common`. Future Kinjo releases arrive
through your normal APT updates. For other Debian-based distributions, use the
[downloadable `.deb` package](docs/installation.md#debian--ubuntu).

For release downloads, source builds, Snap, Nix / NixOS, or Docker, see the
[full installation guide](docs/installation.md).

## Getting started

### Linux setup

**On Linux, discovery requires a running Avahi daemon and system D-Bus**, whichever
installation method you choose. On Ubuntu or Debian, install and start Avahi:

```sh
sudo apt install avahi-daemon
sudo systemctl enable --now avahi-daemon
```

On other Linux distributions, install `avahi` using your system package manager
and enable its service (see also [NixOS](docs/installation.md#nix--nixos) and
[Docker](docs/installation.md#docker-linux)).
macOS and Windows use their built-in DNS-SD services.

### Start browsing

Start Kinjo in your terminal:

```sh
kinjo
```

Kinjo browses the default `local` domain immediately; no configuration is needed
to discover services. Homebrew, PPA, and release packages also include default
actions such as SSH and opening a web interface. Those actions need the programs
they launch, such as `ssh` or a browser, installed on your machine.

- Move with the arrow keys or `j` / `k`.
- Press `/` to search, or `tab` to switch views.
- Press `enter` to choose or run an action for the selected service.
- Press `?` for all shortcuts, or `q` to quit.

If the list is empty, make sure a device on the same local network advertises
services over mDNS/DNS-SD. Check the status line for discovery errors; on Linux,
check that Avahi is running. Press `r` to restart discovery after fixing an error.
On macOS, allow Local Network access for your terminal app if prompted.

Browse another DNS-SD domain with `--domain` (`-d`):

```sh
kinjo --domain example.local
```

See the [usage reference](docs/usage.md) for all shortcuts, filters, views, and
discovery options.

## Configuration

Homebrew, PPA, and release packages include default actions, so you can start
using Kinjo before writing any configuration. To add or customize an action,
save a TOML file under `~/.config/kinjo/commands/` (or
`$XDG_CONFIG_HOME/kinjo/commands/` if set). A user action with the same
`metadata.name` overrides the bundled one.

For example, save this as `~/.config/kinjo/commands/ssh.toml` to connect to SSH
services:

```toml
[metadata]
name = "ssh"
description = "SSH into a service"
requirements = ["ssh"]

[match.service_type]
equals = "_ssh._tcp"

[action]
description = "SSH into the selected service"
command = "ssh -- {hostname}"
mode = "execute"
```

Validate your commands before starting Kinjo:

```sh
kinjo list-commands
```

To load an additional directory, run `kinjo --config-dir ./commands`. Restart
Kinjo after editing actions, or reload a running instance with `pkill -HUP kinjo`
on Unix systems. An invalid reload leaves the current actions in place.

See the [custom commands guide](docs/actions.md) for matching rules, templates,
action modes, directory precedence, and reloading. To change shortcuts, create
`~/.config/kinjo/keybindings.toml` (or `$XDG_CONFIG_HOME/kinjo/keybindings.toml`)
and follow the [keybindings guide](docs/keybindings.md).

## Documentation

| What you want to do | Guide |
|---|---|
| Install from a release archive, Cargo, source, Snap, Nix, or Docker | [Installation options](docs/installation.md) |
| Explore views, shortcuts, filtering, and discovery backends | [Usage reference](docs/usage.md) |
| Add an action or change how services are matched | [Custom commands](docs/actions.md) |
| Customize keyboard shortcuts | [Keybindings](docs/keybindings.md) |
| Understand the code and extension points | [Architecture](docs/architecture.md) |
| Check changes between releases | [Release notes](docs/release-notes.md) |

## Contributing

Questions, bug reports, feature ideas, documentation fixes, and pull requests
are welcome. Use [GitHub Issues](https://github.com/abbyssoul/kinjo/issues) to
ask for help or suggest an improvement. For a discovery problem, include your
operating system, Kinjo version (`kinjo --version`), and the status-line error.

You can contribute by improving instructions, sharing useful action examples,
reporting a reproducible bug, or working on the code. Start with
[CONTRIBUTING.md](CONTRIBUTING.md) for development setup, sample services, and
local verification commands.

Kinjo is maintained by [Ivan Ryabov (@abbyssoul)](https://github.com/abbyssoul).

## Privacy

`kinjo` browses your local network, so it's worth being explicit about what it
does and does not do with that access:

- **No telemetry.** `kinjo` does not phone home, collect analytics, or send
  usage data anywhere. There is no update checker and no crash reporter.
- **No proactive scanning.** `kinjo` never port-scans or probes hosts on its
  own initiative. All discovery is delegated to a pluggable backend behind a
  discovery session (see [Architecture](docs/architecture.md#architecture)), and
  every backend speaks only the standard mDNS/DNS-SD protocol — it surfaces services that
  are already being announced on the link, nothing more.
- **Discovered data stays local.** Services found on the network are shown in
  the terminal and used only to fill in the commands you configure. Nothing is
  uploaded or shared with anyone but you.

The two network discovery backends differ in how they reach the network:

- `mdns-sd` (default) delegates discovery to the operating system: Avahi over
  system D-Bus on Linux, or the native DNS-SD APIs on macOS and Windows. The
  system stack handles mDNS queries and responses.
- `zeroconf` (opt-in, behind the `zeroconf` cargo feature) delegates all
  network I/O to the system `avahi-daemon` over D-Bus. `kinjo` itself opens no
  sockets in this mode — it only reads the records the daemon already
  maintains.

Either way, the traffic involved is the same kind of local multicast query
your OS already performs for Bonjour/AirPlay/network-printer discovery — not
a general network scan.

## License

Kinjo is available under the [MIT License](LICENSE).
