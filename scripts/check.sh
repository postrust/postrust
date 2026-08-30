#!/bin/bash
# Everything that can be checked without Hasura.
#
# Exists because a compile error is not a test failure and does not look like
# one: `cargo test | grep 'test result: FAILED'` finds nothing when the test
# build breaks, and reads as success. That happened -- the postrust-core unit
# tests stopped compiling and stayed broken for a while, because a signature
# changed and `cargo build` does not compile `#[cfg(test)]` code. This script
# fails on the exit status and says which stage.
#
# Usage:
#   scripts/check.sh                    # unit tests only
#   DATABASE_URL=... scripts/check.sh   # and the ones that need a database
#
# A database is prepared with:
#   psql "$DATABASE_URL" -f scripts/init-db.sql -f scripts/test-fixtures.sql
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

failed=0
stage() {
    printf '\n==> %s\n' "$1"
}
check() {
    local what="$1"; shift
    if "$@"; then
        printf '    %s: ok\n' "$what"
    else
        printf '    %s: FAILED\n' "$what"
        failed=1
    fi
}

stage "Formatting and lints"
check "cargo fmt --check" cargo fmt --all -- --check
# Not `-D warnings`: the workspace carries a few lints that predate this
# script and that fixing means changing a shared error type. Clippy still
# fails the stage on a real error, which is what this is here to catch.
check "cargo clippy" cargo clippy --workspace --all-targets

stage "Tests that need nothing"
check "cargo test" cargo test --workspace

if [ -n "${DATABASE_URL:-}" ]; then
    stage "Tests that need a database"
    # Serialised more than the default: each one creates and drops a schema of
    # its own, and the fixture database is small.
    check "cargo test -- --ignored" cargo test --workspace -- --ignored --test-threads=4
else
    stage "Tests that need a database"
    printf '    skipped -- set DATABASE_URL to run them\n'
fi

printf '\n'
if [ "$failed" -ne 0 ]; then
    printf 'FAILED\n'
    exit 1
fi
printf 'ok\n'
