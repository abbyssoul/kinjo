#!/usr/bin/env bash
set -euo pipefail

# Build unsigned Debian source packages of the checked-out commit for Launchpad,
# one per Ubuntu series, into OUTDIR. Signing happens later, in the job that
# holds the key, so this script never sees it.
#
# Launchpad builds offline, so every crate pinned by Cargo.lock is vendored into
# the orig tarball. All series share that one orig tarball, and Launchpad
# rejects a second, different file under the same name, so it is built
# reproducibly from the commit: a rerun yields identical bytes.

script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$script_dir/lib.sh"

if [[ $# -lt 3 ]]; then
    echo "usage: $0 VERSION OUTDIR SERIES..." >&2
    exit 2
fi

version="$1"
outdir="$2"
shift 2
root="$(cd "$script_dir/../.." && pwd)"
cd "$root"

release_validate_version "$version"
for series in "$@"; do
    [[ "$series" =~ ^[a-z]+$ ]] || { release_error "invalid Ubuntu series name: $series"; exit 1; }
done
release_require_manifest_version "$version"

umask 022
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
src="$work/kinjo-$version"
orig="kinjo_${version}.orig.tar.xz"

epoch="$(git -C "$root" show -s --format=%ct HEAD)"
maintainer="$(sed -n 's/^Maintainer: //p' "$root/packaging/ppa/debian/control")"
[[ -n "$maintainer" ]] || { release_error "no Maintainer in packaging/ppa/debian/control"; exit 1; }

git -C "$root" archive --format=tar --prefix="kinjo-$version/" HEAD | tar -xf - -C "$work"
cargo vendor --locked --manifest-path "$src/Cargo.toml" "$src/vendor" >/dev/null

tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner \
    --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
    -cf - -C "$work" "kinjo-$version" | xz -6 -T1 > "$work/$orig"

changelog_date="$(LC_ALL=C date -u -R -d "@$epoch")"
for series in "$@"; do
    rm -rf "$src/debian"
    cp -R "$root/packaging/ppa/debian" "$src/debian"
    cat > "$src/debian/changelog" <<EOF
kinjo (${version}-1~${series}1) ${series}; urgency=medium

  * Release ${version}: https://github.com/abbyssoul/kinjo/releases/tag/v${version}

 -- ${maintainer}  ${changelog_date}
EOF
    # -sa puts the orig tarball in every series' upload; Launchpad accepts the
    # repeats because the bytes are identical. -d and --no-pre-clean: the build
    # dependencies (cargo-1.91 and debhelper) are needed only on the builder.
    (cd "$src" && SOURCE_DATE_EPOCH="$epoch" dpkg-buildpackage -S -sa -us -uc -d --no-pre-clean)
done

mkdir -p "$outdir"
cp "$work/$orig" "$work"/*.debian.tar.xz "$work"/*.dsc "$work"/*_source.changes "$work"/*_source.buildinfo "$outdir/"
ls -l "$outdir"
