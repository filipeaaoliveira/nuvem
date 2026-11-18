#!/usr/bin/env bash
# Render the Homebrew formula for one release.
#
#   packaging/homebrew/render-formula.sh <version> <dist-dir> > nuvem.rb
#
# <version> is the release version without the leading `v` (0.1.0).
# <dist-dir> holds the release archives and their .sha256 sidecars:
#
#   nuvem-aarch64-apple-darwin.tar.gz        + .sha256
#   nuvem-x86_64-apple-darwin.tar.gz         + .sha256
#   nuvem-aarch64-unknown-linux-gnu.tar.gz   + .sha256
#   nuvem-x86_64-unknown-linux-gnu.tar.gz    + .sha256
#
# Each archive is checked against its sidecar, and the digest written into the
# formula is computed from the archive itself, so a formula cannot be rendered
# for bytes that disagree with what the release publishes. Any missing file is
# an error: a formula with one platform's checksum left blank installs nothing
# on that platform, and the failure would land on users, not here.
set -euo pipefail

usage() {
    echo "usage: $0 <version> <dist-dir>" >&2
    exit 2
}

[ "$#" -eq 2 ] || usage
version="$1"
dist="$2"

if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: version must be MAJOR.MINOR.PATCH without a leading v, a pre-release suffix or build metadata, got '$version'" >&2
    exit 2
fi
[ -d "$dist" ] || { echo "error: $dist is not a directory" >&2; exit 2; }

template="$(dirname "$0")/nuvem.rb.in"
[ -f "$template" ] || { echo "error: template $template not found" >&2; exit 1; }

# sha256sum on Linux, shasum on macOS; both print "<digest>  <file>".
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1"
    else
        shasum -a 256 "$1"
    fi
}

digest_of() {
    local archive="$dist/nuvem-$1.tar.gz"
    local sidecar="$archive.sha256"
    [ -f "$archive" ] || { echo "error: missing $archive" >&2; exit 1; }
    [ -f "$sidecar" ] || { echo "error: missing $sidecar" >&2; exit 1; }

    local actual expected
    actual="$(sha256 "$archive" | awk '{print $1}')"
    expected="$(awk '{print $1; exit}' "$sidecar")"
    if [ "$actual" != "$expected" ]; then
        echo "error: $archive hashes to $actual, but its sidecar says $expected" >&2
        exit 1
    fi
    echo "$actual"
}

aarch64_darwin="$(digest_of aarch64-apple-darwin)"
x86_64_darwin="$(digest_of x86_64-apple-darwin)"
aarch64_linux="$(digest_of aarch64-unknown-linux-gnu)"
x86_64_linux="$(digest_of x86_64-unknown-linux-gnu)"

formula="$(sed \
    -e "s/@VERSION@/$version/g" \
    -e "s/@SHA256_AARCH64_APPLE_DARWIN@/$aarch64_darwin/g" \
    -e "s/@SHA256_X86_64_APPLE_DARWIN@/$x86_64_darwin/g" \
    -e "s/@SHA256_AARCH64_UNKNOWN_LINUX_GNU@/$aarch64_linux/g" \
    -e "s/@SHA256_X86_64_UNKNOWN_LINUX_GNU@/$x86_64_linux/g" \
    "$template")"

# A placeholder the template gained without a substitution here would reach
# the tap as a literal and break the formula for that platform.
if leftover="$(printf '%s\n' "$formula" | grep -o '@[A-Z0-9_]*@')"; then
    echo "error: the template has placeholders this script does not fill: $leftover" >&2
    exit 1
fi
printf '%s\n' "$formula"
