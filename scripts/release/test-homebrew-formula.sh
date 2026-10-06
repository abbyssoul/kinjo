#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

macos_arm64="$(printf 'a%.0s' {1..64})"
macos_intel="$(printf 'b%.0s' {1..64})"
linux_arm64="$(printf 'c%.0s' {1..64})"
linux_x86_64="$(printf 'd%.0s' {1..64})"
shas=("$macos_arm64" "$macos_intel" "$linux_arm64" "$linux_x86_64")

fail() {
    printf 'homebrew-formula-test: %s\n' "$*" >&2
    exit 1
}

render() {
    scripts/release/update-homebrew-formula.sh "$1" "$2" "${shas[@]}"
}

# The tap's formula before this renderer: macOS archives plus a Rust source
# build on Linux. The first release rendered over it must replace all of it.
write_legacy() {
    printf '%s\n' \
        'class Kinjo < Formula' \
        '  url "https://github.com/abbyssoul/kinjo/releases/download/v0.3.9/kinjo-0.3.9.tar.gz"' \
        '  sha256 "0000" # kinjo-source-sha256' \
        '  on_macos do' \
        '    on_arm do' \
        '      url "https://github.com/abbyssoul/kinjo/releases/download/v0.3.9/kinjo-0.3.9-aarch64-apple-darwin.tar.gz"' \
        '      sha256 "1111" # kinjo-macos-arm64-sha256' \
        '    end' \
        '  end' \
        '  on_linux do' \
        '    depends_on "rust" => :build' \
        '  end' \
        '  def install' \
        '    system "cargo", "install", *std_cargo_args' \
        '  end' \
        'end' > "$1"
}

# Prints the url/sha256 pair following the given platform blocks, e.g.
# `block on_linux on_arm`.
block() {
    awk -v os="$1" -v arch="$2" '
        $1 == os { in_os = 1 }
        in_os && $1 == arch { in_arch = 1 }
        in_arch && ($1 == "url" || $1 == "sha256") { print $2 }
        in_arch && $1 == "end" { exit }
    ' "$formula"
}

formula="$tmp/Formula/kinjo.rb"
mkdir -p "$(dirname "$formula")"
write_legacy "$formula"
render "$formula" 0.10.0

base='"https://github.com/abbyssoul/kinjo/releases/download/v0.10.0/kinjo-0.10.0'
[[ "$(block on_macos on_arm)" == "$base-aarch64-apple-darwin.tar.gz\""$'\n'"\"$macos_arm64\"" ]] ||
    fail "macOS arm64 block does not install its own archive and digest"
[[ "$(block on_macos on_intel)" == "$base-x86_64-apple-darwin.tar.gz\""$'\n'"\"$macos_intel\"" ]] ||
    fail "macOS Intel block does not install its own archive and digest"
[[ "$(block on_linux on_arm)" == "$base-aarch64-unknown-linux-musl.tar.gz\""$'\n'"\"$linux_arm64\"" ]] ||
    fail "Linux arm64 block does not install its own archive and digest"
[[ "$(block on_linux on_intel)" == "$base-x86_64-unknown-linux-musl.tar.gz\""$'\n'"\"$linux_x86_64\"" ]] ||
    fail "Linux x86_64 block does not install its own archive and digest"

# No platform may build from source: that is what required a Rust toolchain.
for forbidden in cargo rust kinjo-0.10.0.tar.gz kinjo-source-sha256 '/archive/refs/tags/'; do
    if grep -Fq -- "$forbidden" "$formula"; then
        fail "rendered formula still mentions $forbidden"
    fi
done
[[ "$(grep -c '^ *url ' "$formula")" -eq 4 ]] || fail "expected exactly four archive URLs"
# Homebrew scans the version from the URLs; its audit rejects a redundant one.
if grep -Eq '^ *version ' "$formula"; then
    fail "rendered formula declares a redundant version"
fi

# A rerun for the same release is a no-op.
cp "$formula" "$tmp/first.rb"
render "$formula" 0.10.0
cmp -s "$formula" "$tmp/first.rb" || fail "re-rendering the same release changed the formula"

# Numeric, not lexical: 0.9.0 sorts after 0.10.0 as text.
if render "$formula" 0.9.0 >/dev/null 2>&1; then
    fail "downgrade was accepted"
fi
cmp -s "$formula" "$tmp/first.rb" || fail "a refused downgrade changed the formula"

render "$formula" 0.11.0
grep -Fq '/releases/download/v0.11.0/kinjo-0.11.0-x86_64-unknown-linux-musl.tar.gz' "$formula" ||
    fail "newer release was not rendered"

# A formula naming two releases has no single current version to compare with.
mixed="$tmp/mixed.rb"
sed '0,/v0\.11\.0/s//v9.0.0/' "$formula" > "$mixed"
if render "$mixed" 0.12.0 >/dev/null 2>&1; then
    fail "formula mixing releases was accepted"
fi

fresh="$tmp/fresh/Formula/kinjo.rb"
render "$fresh" 0.10.0
[[ -f "$fresh" ]] || fail "absent formula was not created"

if scripts/release/update-homebrew-formula.sh "$formula" 0.12.0 invalid "$macos_intel" "$linux_arm64" "$linux_x86_64" >/dev/null 2>&1; then
    fail "invalid checksum was accepted"
fi
if scripts/release/update-homebrew-formula.sh "$formula" 0.12.0 "$macos_arm64" "$macos_intel" "$linux_arm64" >/dev/null 2>&1; then
    fail "a missing Linux checksum was accepted"
fi
if render "$formula" 0.12.0-rc.1 >/dev/null 2>&1; then
    fail "prerelease version was accepted"
fi

printf 'homebrew-formula-test: PASS\n'
