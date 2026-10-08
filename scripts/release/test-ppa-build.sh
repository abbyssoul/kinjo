#!/usr/bin/env bash
set -euo pipefail

# Build a source package from build-ppa-source.sh the way Launchpad will: in a
# clean container of the target Ubuntu series, with build dependencies from that
# series' archive only, no network during the build, and HOME pointing nowhere.
# Then install the resulting .deb and run it. A failure here would otherwise
# surface only as an emailed build failure after the upload.
#
# CONTAINER_RUNTIME selects docker (default) or podman.

script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$script_dir/lib.sh"

if [[ $# -ne 3 ]]; then
    echo "usage: $0 SOURCE_DIR VERSION SERIES" >&2
    exit 2
fi

source_dir="$(cd "$1" && pwd)"
version="$2"
series="$3"
runtime="${CONTAINER_RUNTIME:-docker}"

release_validate_version "$version"
dsc="kinjo_${version}-1~${series}1.dsc"
[[ -f "$source_dir/$dsc" ]] || { release_error "$dsc not found in $source_dir"; exit 1; }

name="kinjo-ppa-${series}-$$"
image="localhost/kinjo-ppa-build:${series}-$$"
cleanup() {
    "$runtime" rm -f "$name" >/dev/null 2>&1 || true
    "$runtime" rmi -f "$image" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Stage 1, online: unpack the source and install its build dependencies.
"$runtime" run --name "$name" -v "$source_dir:/in:ro" -e DEBIAN_FRONTEND=noninteractive \
    "docker.io/library/ubuntu:${series}" bash -euxc "
        apt-get update
        apt-get install -y --no-install-recommends dpkg-dev
        useradd --create-home builder
        install -d -o builder /build
        runuser -u builder -- dpkg-source -x '/in/$dsc' /build/src
        apt-get build-dep -y --no-install-recommends /build/src
    "
"$runtime" commit "$name" "$image" >/dev/null

# Stage 2, offline: build as an unprivileged user, then install and run.
"$runtime" run --rm --network none "$image" bash -euxc "
    cd /build/src
    runuser -u builder -- env HOME=/sbuild-nonexistent dpkg-buildpackage -b -us -uc
    dpkg -i /build/kinjo_*.deb
    kinjo --version | grep -F '$version'
    test -f /etc/kinjo/commands/ssh.toml
    test -f /usr/share/doc/kinjo/copyright
"
