#!/usr/bin/env bash

# Shared release validation. This file is sourced by the small command wrappers
# and their tests so release invariants do not live only inside workflow YAML.

release_error() {
    printf 'release: %s\n' "$*" >&2
}

release_validate_version() {
    local version="${1:-}"
    local part
    local -a parts

    if [[ ! "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
        release_error "'$version' is not stable MAJOR.MINOR.PATCH SemVer"
        return 1
    fi

    IFS=. read -r -a parts <<<"$version"
    for part in "${parts[@]}"; do
        if ((10#$part > 2147483647)); then
            release_error "version component '$part' is too large"
            return 1
        fi
    done
}

# Prints -1, 0, or 1 when the first validated version is lower, equal, or
# higher than the second one.
release_compare_versions() {
    local left="$1"
    local right="$2"
    local -a lhs rhs
    local i

    release_validate_version "$left" || return 1
    release_validate_version "$right" || return 1
    IFS=. read -r -a lhs <<<"$left"
    IFS=. read -r -a rhs <<<"$right"

    for i in 0 1 2; do
        if ((10#${lhs[$i]} < 10#${rhs[$i]})); then
            printf '%s\n' -1
            return
        fi
        if ((10#${lhs[$i]} > 10#${rhs[$i]})); then
            printf '%s\n' 1
            return
        fi
    done
    printf '%s\n' 0
}

release_require_newer() {
    local requested="$1"
    local current="$2"
    local comparison

    comparison="$(release_compare_versions "$requested" "$current")" || return 1
    if [[ "$comparison" != 1 ]]; then
        release_error "requested $requested must be newer than current $current"
        return 1
    fi
}

release_manifest_version() {
    cargo metadata --locked --no-deps --format-version 1 |
        jq -er '.packages[] | select(.name == "kinjo") | .version'
}

release_require_manifest_version() {
    local expected="$1"
    local actual

    release_validate_version "$expected" || return 1
    actual="$(release_manifest_version)" || return 1
    if [[ "$actual" != "$expected" ]]; then
        release_error "Cargo.toml version $actual does not match requested $expected"
        return 1
    fi
}

release_require_main_ref() {
    local ref="${1:-${GITHUB_REF:-}}"

    if [[ "$ref" != refs/heads/main ]]; then
        release_error "release workflows must run from refs/heads/main (got '$ref')"
        return 1
    fi
}

# A pinned release commit must be a full, lowercase 40-hex SHA. Abbreviated or
# uppercase forms are rejected so the tag, crate, and assets all resolve to one
# unambiguous object.
release_validate_sha() {
    local sha="${1:-}"

    if [[ ! "$sha" =~ ^[0-9a-f]{40}$ ]]; then
        release_error "'$sha' is not a full lowercase 40-hex commit SHA"
        return 1
    fi
}
