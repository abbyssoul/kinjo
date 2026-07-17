#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

arm64="$(printf 'a%.0s' {1..64})"
intel="$(printf 'b%.0s' {1..64})"
source_sha="$(printf 'c%.0s' {1..64})"

write_fixture() {
    local destination="$1"
    printf '%s\n' \
        'class Kinjo < Formula' \
        '  url "https://github.com/abbyssoul/kinjo/archive/refs/tags/v0.2.0.tar.gz"' \
        '  sha256 "0000" # kinjo-source-sha256' \
        '  on_macos do' \
        '    on_arm do' \
        '      url "https://github.com/abbyssoul/kinjo/releases/download/v0.2.0/kinjo-0.2.0-aarch64-apple-darwin.tar.gz"' \
        '      sha256 "1111" # kinjo-macos-arm64-sha256' \
        '    end' \
        '    on_intel do' \
        '      url "https://github.com/abbyssoul/kinjo/releases/download/v0.2.0/kinjo-0.2.0-x86_64-apple-darwin.tar.gz"' \
        '      sha256 "2222" # kinjo-macos-intel-sha256' \
        '    end' \
        '  end' \
        'end' > "$destination"
}

formula="$tmp/kinjo.rb"
write_fixture "$formula"
scripts/release/update-homebrew-formula.sh \
    "$formula" 0.10.0 "$arm64" "$intel" "$source_sha"
grep -Fq '/releases/download/v0.10.0/kinjo-0.10.0.tar.gz' "$formula"
scripts/release/update-homebrew-formula.sh \
    "$formula" 0.11.0 "$arm64" "$intel" "$source_sha"
grep -Fq '/releases/download/v0.11.0/kinjo-0.11.0.tar.gz' "$formula"

downgrade="$tmp/downgrade.rb"
write_fixture "$downgrade"
if scripts/release/update-homebrew-formula.sh \
    "$downgrade" 0.1.9 "$arm64" "$intel" "$source_sha" >/dev/null 2>&1; then
    echo "homebrew-formula-test: downgrade was accepted" >&2
    exit 1
fi

missing="$tmp/missing-marker.rb"
write_fixture "$missing"
sed -i '/kinjo-source-sha256/d' "$missing"
if scripts/release/update-homebrew-formula.sh \
    "$missing" 0.10.0 "$arm64" "$intel" "$source_sha" >/dev/null 2>&1; then
    echo "homebrew-formula-test: missing checksum marker was accepted" >&2
    exit 1
fi

if scripts/release/update-homebrew-formula.sh \
    "$formula" 0.11.0 invalid "$intel" "$source_sha" >/dev/null 2>&1; then
    echo "homebrew-formula-test: invalid checksum was accepted" >&2
    exit 1
fi

printf 'homebrew-formula-test: PASS\n'
