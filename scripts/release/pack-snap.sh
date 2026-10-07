#!/usr/bin/env bash
set -euo pipefail

# Pack the static Linux binary into kinjo_VERSION_ARCH.snap in the current
# directory. The snap wraps the exact binary the Linux archive ships, so packing
# a prepared directory is enough and no snapcraft build provider is needed.
#
# Confinement is classic: Kinjo exists to launch the commands a user configures
# (ssh, a browser, anything), and a strict snap could run only binaries it ships
# itself. The default commands sit beside the binary, where Kinjo looks for them.

script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$script_dir/lib.sh"

if [[ $# -ne 3 ]]; then
    echo "usage: $0 VERSION ARCH BINARY" >&2
    exit 2
fi

version="$1"
arch="$2"
binary="$3"
root="$script_dir/../.."

release_validate_version "$version"
case "$arch" in
    amd64 | arm64) ;;
    *) release_error "unsupported snap architecture: $arch"; exit 1 ;;
esac
[[ -x "$binary" ]] || { release_error "binary not found or not executable: $binary"; exit 1; }

umask 022
prime="$(mktemp -d)"
trap 'rm -rf "$prime"' EXIT
# mktemp creates the directory 0700; the snap root must be world-readable.
chmod 0755 "$prime"

install -D -m 0755 "$binary" "$prime/kinjo"
install -D -m 0644 -t "$prime/commands" "$root"/actions/*.toml
install -m 0644 "$root/LICENSE" "$root/README.md" "$prime/"
mkdir "$prime/meta"
cat > "$prime/meta/snap.yaml" <<EOF
name: kinjo
version: '$version'
summary: Browse mDNS/DNS-SD services and launch commands against them
description: |
  Kinjo is a terminal UI that discovers services on the local network over
  mDNS/DNS-SD and runs configured commands, such as ssh or a browser, against
  the selected one. On Linux it browses through the host's avahi-daemon.
license: MIT
base: core24
grade: stable
confinement: classic
architectures: [$arch]
apps:
  kinjo:
    command: kinjo
EOF

snapcraft pack "$prime" --output "kinjo_${version}_${arch}.snap"
