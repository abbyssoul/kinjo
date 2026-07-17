#!/usr/bin/env bash
set -euo pipefail

# Assert that a staging directory holds exactly the release artifact set for
# VERSION, then write SHA256SUMS over it. Both the dry run and the publisher call
# this, so `publish=false` proves the same naming contract the publisher relies
# on rather than discovering a mismatch after the release approval.

script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$script_dir/lib.sh"

if [[ $# -ne 2 ]]; then
    echo "usage: $0 DIST VERSION" >&2
    exit 2
fi

dist="$1"
version="$2"

release_validate_version "$version"
[[ -d "$dist" ]] || { release_error "staging directory not found: $dist"; exit 1; }

expected=(
    "kinjo-${version}.crate"
    "kinjo-${version}.cdx.json"
    "kinjo-${version}.tar.gz"
    "kinjo-${version}-aarch64-apple-darwin.tar.gz"
    "kinjo-${version}-x86_64-apple-darwin.tar.gz"
    "kinjo_${version}-1_amd64.deb"
    "kinjo_${version}-1_arm64.deb"
)

for name in "${expected[@]}"; do
    [[ -f "$dist/$name" ]] || { release_error "missing $dist/$name"; exit 1; }
done

# Rerunning must be a no-op, so a SHA256SUMS this script wrote earlier is not an
# unexpected entry.
declare -A permitted=([SHA256SUMS]=1)
for name in "${expected[@]}"; do
    permitted["$name"]=1
done

shopt -s nullglob dotglob
for path in "$dist"/*; do
    name="${path##*/}"
    if [[ -z "${permitted[$name]:-}" ]]; then
        release_error "unexpected staged entry: $name"
        exit 1
    fi
    if [[ ! -f "$path" ]]; then
        release_error "staged entry is not a regular file: $name"
        exit 1
    fi
done
shopt -u nullglob dotglob

# Listing `expected` explicitly fixes the order without depending on the locale's
# collation, so the digest file is byte-identical across runs and runners.
(
    cd "$dist"
    sha256sum "${expected[@]}" > SHA256SUMS.tmp
    mv SHA256SUMS.tmp SHA256SUMS
)

printf 'release: staged %d artifacts for %s\n' "${#expected[@]}" "$version"
