#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."
source scripts/release/lib.sh

usage() {
    echo "usage: $0 stable VERSION | newer REQUESTED CURRENT | manifest VERSION | sha SHA | main [REF]" >&2
    exit 2
}

case "${1:-}" in
    stable)
        [[ $# -eq 2 ]] || usage
        release_validate_version "$2"
        ;;
    newer)
        [[ $# -eq 3 ]] || usage
        release_require_newer "$2" "$3"
        ;;
    manifest)
        [[ $# -eq 2 ]] || usage
        release_require_manifest_version "$2"
        ;;
    sha)
        [[ $# -eq 2 ]] || usage
        release_validate_sha "$2"
        ;;
    main)
        [[ $# -le 2 ]] || usage
        release_require_main_ref "${2:-${GITHUB_REF:-}}"
        ;;
    *)
        usage
        ;;
esac

