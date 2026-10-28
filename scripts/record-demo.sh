#!/usr/bin/env bash
#
# Record the README demo GIF with VHS (https://github.com/charmbracelet/vhs).
#
# The recording runs against the built-in sample backend (`--backend fake`), so
# it shows no real device on the recording machine's network. It also runs
# with an empty, throwaway config home that holds only the bundled commands, so
# the recording machine's own keybindings and command rules cannot leak in.
#
# Usage: scripts/record-demo.sh   # writes docs/demo/kinjo-demo.gif
#
# KINJO_TAPE overrides the tape to play (default: docs/demo/demo.tape).
set -euo pipefail

cd "$(dirname "$0")/.."

command -v vhs >/dev/null || {
    echo "record-demo: vhs not found; see https://github.com/charmbracelet/vhs" >&2
    exit 1
}

cargo build --release --features fake

config_home=$(mktemp -d)
trap 'rm -rf "$config_home"' EXIT
mkdir -p "$config_home/kinjo/commands"
cp actions/*.toml "$config_home/kinjo/commands/"

XDG_CONFIG_HOME="$config_home" PATH="$PWD/target/release:$PATH" vhs "${KINJO_TAPE:-docs/demo/demo.tape}"
