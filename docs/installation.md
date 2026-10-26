# Installation options

For the recommended Homebrew and Ubuntu PPA commands, see
[Installation in the README](../README.md#installation). This guide covers
release downloads, Cargo, source builds, Snap, Nix / NixOS, and Docker.

On Linux, discovery requires Avahi and system D-Bus. Complete the
[Linux setup](../README.md#linux-setup) before starting Kinjo.

## Download a release (macOS and Linux)

Open the [latest GitHub release](https://github.com/abbyssoul/kinjo/releases/latest)
and expand **Assets**. Download the `.tar.gz` matching your operating system and
CPU; no Rust toolchain is needed. Run `uname -m` if you are unsure of your CPU.

| Platform / CPU | Archive name (`VERSION` is the release version) |
|---|---|
| macOS, Apple silicon (`arm64`) | `kinjo-VERSION-aarch64-apple-darwin.tar.gz` |
| macOS, Intel (`x86_64`) | `kinjo-VERSION-x86_64-apple-darwin.tar.gz` |
| Linux, x86_64 / amd64 | `kinjo-VERSION-x86_64-unknown-linux-musl.tar.gz` |
| Linux, ARM64 / aarch64 | `kinjo-VERSION-aarch64-unknown-linux-musl.tar.gz` |

In the directory where you downloaded the archive, extract it and run Kinjo.
Replace the example filename and directory below with the ones you downloaded:

```sh
tar -xzf kinjo-VERSION-aarch64-apple-darwin.tar.gz
cd kinjo-VERSION-aarch64-apple-darwin
./kinjo
```

Keep the `commands/` directory alongside the binary to use the bundled actions.
To install the binary and those actions for all users on macOS or Linux:

```sh
sudo install -d /usr/local/bin /usr/local/share/kinjo/commands
sudo install -m 0755 kinjo /usr/local/bin/kinjo
sudo install -m 0644 commands/*.toml /usr/local/share/kinjo/commands/
```

Ensure `/usr/local/bin` is on your `PATH`, then run `kinjo` from any directory.
For a later release, download the new archive and repeat these steps.

## Debian / Ubuntu

Download the `.deb` for your architecture (`amd64` or `arm64`) from the
[latest GitHub release](https://github.com/abbyssoul/kinjo/releases/latest).
From the download directory, install it with `apt`:

```sh
sudo apt install ./kinjo_*.deb
```

Download only the package for your architecture into that directory. The package
includes the `kinjo` binary and default commands; Linux also needs the
[Avahi setup](../README.md#linux-setup). On Ubuntu, the [PPA](../README.md#ubuntu-ppa) is the
easier option for receiving updates.

## Cargo (advanced)

With the [Rust toolchain](https://rustup.rs/) installed, build and install Kinjo
from crates.io:

```sh
cargo install kinjo --locked
```

This works on macOS, Linux, and Windows. Ensure Cargo's bin directory (usually
`~/.cargo/bin`, or `%USERPROFILE%\.cargo\bin` on Windows) is on your `PATH`, then
run `kinjo`. Windows is tested in CI; prebuilt release downloads are currently
provided for macOS and Linux.

Cargo installs the binary only. For example actions, download and extract a
[release archive](#download-a-release-macos-and-linux), then run
`kinjo --config-dir /path/to/extracted/commands`, replacing the path with the
actual directory. You can also [write your own actions](actions.md).

Optional discovery backends are selected at compile time. For example, the
Avahi-backed `zeroconf` backend on Linux requires the Avahi client development
headers (see [Discovery backends](usage.md#discovery-backends)):

```sh
cargo install kinjo --locked --features zeroconf
```

## Build from source

Install [Rust via rustup](https://rustup.rs/), Git, and your platform's C/C++
build tools, then clone and install Kinjo:

```sh
git clone https://github.com/abbyssoul/kinjo.git
cd kinjo
cargo install --path . --locked
kinjo --config-dir ./actions
```

The checkout's `rust-toolchain.toml` selects the Rust version used by the project.
Cargo installs the executable into its bin directory, which must be on your
`PATH`. The `--config-dir ./actions` argument loads the example actions from the
checkout; use an absolute path when launching from another directory, or copy
the TOML files into your [user command directory](../README.md#configuration).

To build without installing, run `cargo build --release --locked`, then
`./target/release/kinjo --config-dir ./actions` (`target\release\kinjo.exe` on
Windows). Linux discovery still requires Avahi, but the default build does not
need Avahi development headers.

To try sample services without network discovery, run from the checkout:

```sh
cargo run --features fake -- --backend fake --config-dir actions
```

Contributors can find the full development setup and verification commands in
[CONTRIBUTING.md](../CONTRIBUTING.md).

## Snap (Linux)

Each release attaches a snap for `amd64` and `arm64`. Download the one for your
architecture from the
[latest GitHub release](https://github.com/abbyssoul/kinjo/releases/latest)
and install it:

```sh
sudo snap install --dangerous --classic ./kinjo_*.snap
```

Once Kinjo is in the Snap Store, `sudo snap install kinjo --classic` installs it
from there and keeps it updated. `--dangerous` is needed only for a downloaded
file, which the Store has not signed.

The snap uses classic confinement because Kinjo runs the commands you
configure, such as `ssh` or a browser, and a confined snap could run only
programs it ships. It shares `~/.config/kinjo` with other installations and
includes the default commands. Like the `.deb`, it browses through the host's
`avahi-daemon`.

## Nix / NixOS

Run Kinjo without installing it:

```sh
nix run github:abbyssoul/kinjo
```

The flake provides packages for `x86_64-linux` and `aarch64-linux`. To install
Kinjo declaratively, add the flake as an input and use its overlay:

```nix
# flake inputs:
kinjo.url = "github:abbyssoul/kinjo";

# in your NixOS module:
nixpkgs.overlays = [ inputs.kinjo.overlays.default ];
environment.systemPackages = [ pkgs.kinjo ];
```

Enable Avahi on NixOS for service discovery:

```nix
services.avahi.enable = true;
```

## Docker (Linux)

A multi-arch image (`linux/amd64`, `linux/arm64`) is published with each
release. It contains both discovery backends and the default commands.

```sh
docker run --rm -it \
  --network host \
  -v /run/dbus/system_bus_socket:/run/dbus/system_bus_socket \
  -v /run/avahi-daemon/socket:/run/avahi-daemon/socket \
  ghcr.io/abbyssoul/kinjo
```

The container does not browse the network itself: on Linux both backends ask
the host's `avahi-daemon` over the system D-Bus, so the host must run
`avahi-daemon`. Each argument does a separate job:

- `-it` gives the TUI a terminal.
- `/run/dbus/system_bus_socket` is required: discovery fails to start without
  it.
- `/run/avahi-daemon/socket` lets commands resolve the `.local` host names in
  `{hostname}`, such as the default `ssh` command. Without it discovery still
  works, but those names do not resolve.
- `--network host` lets commands reach what was discovered, including
  link-local IPv6 addresses that a bridged container cannot route to. Discovery
  alone does not need it.

With rootless Podman, also pass `--userns=keep-id --user "$(id -u):$(id -g)"`.
Otherwise the container user is mapped to a subordinate UID that does not match
the UID it presents to D-Bus, and the bus rejects the connection.

Add your own commands by mounting them over the defaults, for example
`-v ~/.config/kinjo/commands:/etc/kinjo/commands:ro`. Commands run inside the
container, so the `xdg-open` defaults cannot open a browser on the host.
