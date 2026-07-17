#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

version=0.3.0

stage() {
    local dist="$1"
    mkdir -p "$dist"
    ( cd "$dist"
      touch \
          "kinjo-${version}.crate" \
          "kinjo-${version}.cdx.json" \
          "kinjo-${version}.tar.gz" \
          "kinjo-${version}-aarch64-apple-darwin.tar.gz" \
          "kinjo-${version}-x86_64-apple-darwin.tar.gz" \
          "kinjo_${version}-1_amd64.deb" \
          "kinjo_${version}-1_arm64.deb" )
}

fail() {
    printf 'release-artifacts-test: %s\n' "$*" >&2
    exit 1
}

complete="$tmp/complete"
stage "$complete"
scripts/release/check-artifacts.sh "$complete" "$version" >/dev/null
[[ "$(wc -l < "$complete/SHA256SUMS")" -eq 7 ]] || fail "expected seven digest lines"

# Rerunning after a resumed publication must not trip over its own digest file.
scripts/release/check-artifacts.sh "$complete" "$version" >/dev/null

missing="$tmp/missing"
stage "$missing"
rm "$missing/kinjo-${version}.crate"
if scripts/release/check-artifacts.sh "$missing" "$version" >/dev/null 2>&1; then
    fail "missing crate was accepted"
fi

# upload-artifact roots an artifact at the least common ancestor of its paths, so
# uploading target/package/*.crate alongside repository-root files buries the
# crate one directory down. That must fail here rather than after the release
# environment has already been approved.
nested="$tmp/nested"
stage "$nested"
mkdir -p "$nested/target/package"
mv "$nested/kinjo-${version}.crate" "$nested/target/package/"
if scripts/release/check-artifacts.sh "$nested" "$version" >/dev/null 2>&1; then
    fail "crate nested under target/package was accepted"
fi

extra="$tmp/extra"
stage "$extra"
touch "$extra/kinjo-${version}-unexpected.tar.gz"
if scripts/release/check-artifacts.sh "$extra" "$version" >/dev/null 2>&1; then
    fail "unexpected staged file was accepted"
fi

if scripts/release/check-artifacts.sh "$tmp/absent" "$version" >/dev/null 2>&1; then
    fail "absent staging directory was accepted"
fi

if scripts/release/check-artifacts.sh "$complete" 1.2.3-rc.1 >/dev/null 2>&1; then
    fail "prerelease version was accepted"
fi

printf 'release-artifacts-test: PASS\n'
