#!/usr/bin/env bash
# Every Dockerfile that builds the workspace must build it on the toolchain
# rust-toolchain.toml pins -- the one CI lints and tests with.
#
# The 3.1.0 Debian image was never published: its Dockerfile built on an
# unpinned nightly, which hit a compiler regression in code stable builds.
# Bumping rust-toolchain.toml without bumping these is the same drift, so CI
# refuses it.
set -euo pipefail
cd "$(dirname "$0")/.."

want=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
if [ -z "$want" ]; then
    echo "error: no channel in rust-toolchain.toml" >&2
    exit 1
fi

status=0
for file in Dockerfile Dockerfile.alpine Dockerfile.test; do
    while read -r image; do
        tag=${image#rust:}
        case "$tag" in
            "$want"-*) ;;
            *)
                echo "error: $file builds on $image, but rust-toolchain.toml pins $want" >&2
                status=1
                ;;
        esac
    done < <(awk 'toupper($1) == "FROM" && $2 ~ /^(rust|rustlang\/rust):/ { print $2 }' "$file")
    # A nightly image is never the pinned toolchain.
    if grep -qiE '^FROM +rustlang/rust:' "$file"; then
        echo "error: $file builds on a rustlang/rust nightly image" >&2
        status=1
    fi
    if ! grep -qiE '^FROM +rust:' "$file"; then
        echo "error: $file has no rust: build stage to check" >&2
        status=1
    fi
done

[ "$status" -eq 0 ] && echo "Dockerfiles build on $want, as rust-toolchain.toml pins."
exit "$status"
