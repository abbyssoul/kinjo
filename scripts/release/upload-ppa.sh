#!/usr/bin/env bash
set -euo pipefail

# Sign the source packages from build-ppa-source.sh and upload them to a
# Launchpad PPA, then wait until Launchpad has accepted each one.
#
# The key comes from PPA_GPG_PRIVATE_KEY (ASCII-armoured secret key) and
# PPA_GPG_PASSPHRASE. It must be registered with the Launchpad account that
# owns the PPA. It is imported into a throwaway GNUPGHOME removed on exit.
#
# Safe to rerun: a series that already has this version in the PPA, in any
# state, is skipped. Launchpad would reject a second upload of it anyway.
#
# Set PPA_DRY_RUN=1 to sign and simulate the upload without sending anything.

script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$script_dir/lib.sh"

if [[ $# -ne 3 ]]; then
    echo "usage: $0 SOURCE_DIR VERSION LAUNCHPAD_USER/PPA_NAME" >&2
    exit 2
fi

source_dir="$(cd "$1" && pwd)"
version="$2"
ppa="$3"
dry_run="${PPA_DRY_RUN:-0}"
accept_timeout="${PPA_ACCEPT_TIMEOUT:-1800}"

release_validate_version "$version"
if [[ ! "$ppa" =~ ^[a-z0-9][a-z0-9.+-]*/[a-z0-9][a-z0-9.+-]*$ ]]; then
    release_error "'$ppa' is not LAUNCHPAD_USER/PPA_NAME"
    exit 1
fi
: "${PPA_GPG_PRIVATE_KEY:?PPA_GPG_PRIVATE_KEY is not set}"
: "${PPA_GPG_PASSPHRASE:?PPA_GPG_PASSPHRASE is not set}"

owner="${ppa%%/*}"
name="${ppa#*/}"
ppa_api="https://api.launchpad.net/1.0/~${owner}/+archive/ubuntu/${name}"
ppa_web="https://launchpad.net/~${owner}/+archive/ubuntu/${name}"

# Prints the number of kinjo VERSION-REVISION sources in the PPA, in any status.
published_count() {
    curl --proto '=https' -fsS --retry 3 --get "$ppa_api" \
        --data-urlencode ws.op=getPublishedSources \
        --data-urlencode source_name=kinjo \
        --data-urlencode exact_match=true \
        --data-urlencode "version=$1" |
        jq '.entries | length'
}

# Launchpad answers a missing PPA with a redirect, not a 404.
status="$(curl --proto '=https' -sS --retry 3 -o /dev/null -w '%{http_code}' "$ppa_api")"
if [[ "$status" != 200 ]]; then
    release_error "PPA $ppa not found at $ppa_web (HTTP $status)"
    release_error "Create it on Launchpad, or fix the PPA_USERNAME / PPA_NAME repository variables."
    exit 1
fi

shopt -s nullglob
all_changes=("$source_dir"/kinjo_"$version"-*_source.changes)
((${#all_changes[@]})) || { release_error "no kinjo_${version}-*_source.changes in $source_dir"; exit 1; }

pending=()
for changes in "${all_changes[@]}"; do
    pkg_version="$(sed -n 's/^Version: //p' "$changes")"
    count="$(published_count "$pkg_version")"
    if ((count > 0)); then
        echo "kinjo $pkg_version is already in $ppa; skipping"
    else
        pending+=("$changes")
    fi
done
if ((${#pending[@]} == 0)); then
    echo "Every series is already uploaded."
    exit 0
fi

GNUPGHOME="$(mktemp -d)"
export GNUPGHOME
trap 'gpgconf --kill gpg-agent >/dev/null 2>&1 || true; rm -rf "$GNUPGHOME"' EXIT
gpg --batch --quiet --import <<<"$PPA_GPG_PRIVATE_KEY"
fingerprint="$(gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" { print $10; exit }')"
[[ -n "$fingerprint" ]] || { release_error "PPA_GPG_PRIVATE_KEY holds no secret key"; exit 1; }
# dput verifies the signatures before uploading and rejects an untrusted key.
echo "$fingerprint:6:" | gpg --batch --quiet --import-ownertrust
echo "Signing with key $fingerprint"

# debsign takes a program name, not a command line, so the loopback pinentry
# and passphrase go through a wrapper.
printf '%s' "$PPA_GPG_PASSPHRASE" > "$GNUPGHOME/passphrase"
cat > "$GNUPGHOME/gpg-sign" <<EOF
#!/bin/sh
exec gpg --batch --pinentry-mode loopback --passphrase-file '$GNUPGHOME/passphrase' "\$@"
EOF
chmod 0700 "$GNUPGHOME/gpg-sign"

dput_flags=()
[[ "$dry_run" == 1 ]] && dput_flags+=(--simulate)

uploaded=()
for changes in "${pending[@]}"; do
    debsign --no-conf -p"$GNUPGHOME/gpg-sign" -k"$fingerprint" "$changes"
    dput "${dput_flags[@]}" "ppa:$ppa" "$changes"
    uploaded+=("$(sed -n 's/^Version: //p' "$changes")")
done

if [[ "$dry_run" == 1 ]]; then
    echo "Dry run: signed and simulated ${uploaded[*]}; nothing was sent."
    exit 0
fi

# dput succeeds once the files are on the FTP server. Launchpad checks them
# minutes later and reports a rejection only by email, so wait for each upload
# to appear in the PPA.
deadline=$((SECONDS + accept_timeout))
for pkg_version in "${uploaded[@]}"; do
    until (($(published_count "$pkg_version") > 0)); do
        if ((SECONDS >= deadline)); then
            echo "::error title=Launchpad did not accept kinjo ${pkg_version}::Not in ${ppa} after ${accept_timeout}s."
            {
                echo "Check:    every signed upload must appear in $ppa_web"
                echo "Expected: kinjo $pkg_version listed in the PPA (any status)."
                echo "Got:      not listed after ${accept_timeout}s."
                echo "Launchpad emails the reason for a rejection to the signing key's address."
                echo "Possible causes:"
                echo "  - Key $fingerprint is not registered with Launchpad user '$owner':"
                echo "    https://launchpad.net/~${owner}/+editpgpkeys"
                echo "  - That account cannot upload to $ppa."
                echo "  - The PPA already has a different kinjo_${version}.orig.tar.xz."
                echo "  - Launchpad's upload queue is slow; check the PPA page before retrying."
                echo "Next step: read the rejection email, fix the cause, and rerun the release."
                echo "Series already accepted are skipped on rerun."
            } >&2
            exit 1
        fi
        sleep 30
    done
    echo "Launchpad accepted kinjo $pkg_version"
done
echo "Builds are queued on Launchpad: $ppa_web/+packages"
