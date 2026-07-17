#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."
source scripts/release/lib.sh

fail() {
    printf 'release-version-test: %s\n' "$*" >&2
    exit 1
}

expect_ok() {
    "$@" >/dev/null 2>&1 || fail "expected success: $*"
}

expect_fail() {
    if "$@" >/dev/null 2>&1; then
        fail "expected failure: $*"
    fi
}

for version in 0.0.0 0.2.1 1.0.0 10.20.300; do
    expect_ok release_validate_version "$version"
done

for version in '' v1.2.3 01.2.3 1.02.3 1.2.03 1.2 1.2.3.4 1.2.3-rc.1 1.2.x 2147483648.0.0; do
    expect_fail release_validate_version "$version"
done

[[ "$(release_compare_versions 1.10.0 1.9.99)" == 1 ]] || fail "multi-digit comparison"
[[ "$(release_compare_versions 2.0.0 10.0.0)" == -1 ]] || fail "major comparison"
[[ "$(release_compare_versions 3.4.5 3.4.5)" == 0 ]] || fail "equality comparison"

expect_ok release_require_newer 0.10.0 0.9.99
expect_fail release_require_newer 0.2.0 0.2.0
expect_fail release_require_newer 0.1.9 0.2.0
expect_ok release_require_main_ref refs/heads/main
expect_fail release_require_main_ref refs/heads/release

printf 'release-version-test: PASS\n'
