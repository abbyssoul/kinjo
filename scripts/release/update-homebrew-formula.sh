#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$script_dir/lib.sh"

if [[ $# -ne 5 ]]; then
    echo "usage: $0 FORMULA VERSION ARM64_SHA INTEL_SHA SOURCE_SHA" >&2
    exit 2
fi

formula="$1"
version="$2"
arm64_sha="$3"
intel_sha="$4"
source_sha="$5"

release_validate_version "$version"
[[ -f "$formula" ]] || { release_error "formula not found: $formula"; exit 1; }

for checksum in "$arm64_sha" "$intel_sha" "$source_sha"; do
    if [[ ! "$checksum" =~ ^[a-f0-9]{64}$ ]]; then
        release_error "invalid SHA-256: $checksum"
        exit 1
    fi
done

current="$(
    sed -nE \
        -e 's|.*(/archive/refs/tags/)v([0-9]+\.[0-9]+\.[0-9]+)(\.tar\.gz).*|\2|p' \
        -e 's|.*(/releases/download/)v([0-9]+\.[0-9]+\.[0-9]+)/kinjo-[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz.*|\2|p' \
        "$formula" | head -1
)"
if [[ -z "$current" ]]; then
    release_error "could not determine the current source version in $formula"
    exit 1
fi
if [[ "$current" != "$version" ]]; then
    release_require_newer "$version" "$current"
fi

# Migrate the old GitHub-generated source archive to the uploaded immutable
# source asset, then update both that source URL and the two native archives.
sed -i -E \
    "s|(/archive/refs/tags/)v[0-9]+\.[0-9]+\.[0-9]+(\.tar\.gz)|/releases/download/v${version}/kinjo-${version}\2|" \
    "$formula"
sed -i -E \
    "s|(/releases/download/)v[0-9]+\.[0-9]+\.[0-9]+/kinjo-[0-9]+\.[0-9]+\.[0-9]+(\.tar\.gz)|\1v${version}/kinjo-${version}\2|" \
    "$formula"
sed -i -E \
    "s|(/releases/download/)v[0-9]+\.[0-9]+\.[0-9]+/kinjo-[0-9]+\.[0-9]+\.[0-9]+-|\1v${version}/kinjo-${version}-|" \
    "$formula"
sed -i \
    "s/sha256 \"[a-f0-9]*\" # kinjo-macos-arm64-sha256/sha256 \"${arm64_sha}\" # kinjo-macos-arm64-sha256/" \
    "$formula"
sed -i \
    "s/sha256 \"[a-f0-9]*\" # kinjo-macos-intel-sha256/sha256 \"${intel_sha}\" # kinjo-macos-intel-sha256/" \
    "$formula"
sed -i \
    "s/sha256 \"[a-f0-9]*\" # kinjo-source-sha256/sha256 \"${source_sha}\" # kinjo-source-sha256/" \
    "$formula"

expected=(
    "/releases/download/v${version}/kinjo-${version}.tar.gz"
    "/releases/download/v${version}/kinjo-${version}-aarch64-apple-darwin.tar.gz"
    "/releases/download/v${version}/kinjo-${version}-x86_64-apple-darwin.tar.gz"
    "sha256 \"${arm64_sha}\" # kinjo-macos-arm64-sha256"
    "sha256 \"${intel_sha}\" # kinjo-macos-intel-sha256"
    "sha256 \"${source_sha}\" # kinjo-source-sha256"
)

for want in "${expected[@]}"; do
    count="$(grep -Fc -- "$want" "$formula" || true)"
    if [[ "$count" -ne 1 ]]; then
        release_error "expected exactly one formula entry, found $count: $want"
        exit 1
    fi
done

mapfile -t release_urls < <(
    grep -Eo '/releases/download/v[0-9]+\.[0-9]+\.[0-9]+/kinjo-[^"[:space:]]+\.tar\.gz' \
        "$formula" || true
)
if [[ "${#release_urls[@]}" -ne 3 ]]; then
    release_error "expected exactly three Kinjo release-asset URLs, found ${#release_urls[@]}"
    exit 1
fi

if grep -Eq '/archive/refs/tags/v[0-9]+\.[0-9]+\.[0-9]+\.tar\.gz' "$formula"; then
    release_error "formula still references a generated tag archive"
    exit 1
fi
