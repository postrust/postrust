#!/usr/bin/env bash
# vendor/async-graphql-parser must be exactly the pristine crate from crates.io
# plus patches/async-graphql-parser-enum-value.patch -- nothing more, nothing
# less -- so the copy can be audited and dropped once upstream carries the fix.
set -euo pipefail
cd "$(dirname "$0")/.."

version=7.0.17
sha256=60b7607e59424a35dadbc085b0d513aa54ec28160ee640cf79ec3b634eba66d3

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -sSfL --retry 3 -o "$tmp/crate.tgz" \
    "https://static.crates.io/crates/async-graphql-parser/async-graphql-parser-$version.crate"
echo "$sha256  $tmp/crate.tgz" | sha256sum -c - >/dev/null
tar xzf "$tmp/crate.tgz" -C "$tmp"

pristine="$tmp/async-graphql-parser-$version"
(cd "$pristine" && patch -p1 --no-backup-if-mismatch -s) < patches/async-graphql-parser-enum-value.patch

if ! diff -ru -x VENDORED.md "$pristine" vendor/async-graphql-parser; then
    echo "error: vendor/async-graphql-parser differs from the pristine $version crate plus the patch" >&2
    exit 1
fi
echo "vendor/async-graphql-parser matches async-graphql-parser $version plus the patch."
