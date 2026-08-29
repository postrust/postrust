#!/bin/bash
# Differential PostgREST conformance run.
#
# PostgREST's own spec suite (test/spec) cannot be pointed at another server:
# it drives the WAI Application in-process via hspec-wai and imports
# PostgREST.Config directly. What is reusable is (a) the fixture database and
# (b) the request literals inside the examples.
#
# So we replay those requests against stock PostgREST and against Postrust,
# both on an identical, freshly loaded fixture database, and diff the live
# responses. The reference implementation is the oracle -- we never have to
# interpret an hspec expectation, and a mistake in the extractor shows up as a
# case both servers answer the same way rather than as a false failure.
#
# Usage:  scripts/conformance/conformance.sh [postgrest-version]
set -euo pipefail

PGRST_VERSION="${1:-v16.1}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${CONFORMANCE_WORK:-$HERE/.work}"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"

DB=pgrst-conf-db
REF=pgrst-conf-ref
NET=pgrst-conf-net
DB_PORT="${CONFORMANCE_DB_PORT:-55432}"
REF_PORT="${CONFORMANCE_REF_PORT:-55001}"
CAND_PORT="${CONFORMANCE_CAND_PORT:-55002}"

# The fixtures need PostGIS; the stock postgres image cannot load schema.sql.
DB_IMAGE="${CONFORMANCE_DB_IMAGE:-postgis/postgis:16-3.4}"

# Matches PostgREST's own test configuration (test/spec/SpecHelper.hs).
SCHEMA=test
ANON_ROLE=postgrest_test_anonymous
JWT_SECRET=reallyreallyreallyreallyverysafe
# PostgREST's own suite runs each spec under its own configuration; this
# harness runs one server for all of them, so the setting has to be the union
# of what the specs ask for. `extensions` is where the fixtures install PostGIS
# and isn, and without it on the path *both* servers answer 42883 to every
# request that touches them -- which reads as agreement while measuring
# nothing. The third entry is a schema whose name is mostly punctuation, and it
# is here because ExtraSearchPathSpec puts it there.
EXTRA_SEARCH_PATH='public, extensions, EXTRA "@/\#~_-'


mkdir -p "$WORK"
say() { printf '\n==> %s\n' "$*"; }

cleanup() {
    pkill -f 'target/release/postrust' 2>/dev/null || true
    docker rm -f "$REF" "$DB" >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Built here rather than required to exist, because which features it was built
# with is part of what is being measured and cannot be read off the file.
#
# `compat-key-order` is one of them: PostgREST returns object keys in select
# order and this server returns them alphabetically without it. Invisible in
# JSON, since bodies are compared as parsed JSON -- and the whole answer in
# CSV, which puts its columns in key order.
#
# The trap this closes: `cargo test --workspace` rebuilds this same binary with
# default features and overwrites it, so a build, then a test run, then a
# conformance run silently measured a binary with neither feature. Nothing said
# so; the binary was simply 1.1 MB smaller.
say "Building the candidate"
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml" \
    -p postrust-server --features admin-ui,compat-key-order >&2

# The server says so itself when compatibility mode is on without the feature,
# so the log is checked once it starts. Belt and braces: the build above should
# make it impossible, and this catches it if anything else replaces the binary
# between here and there.
assert_key_order() {
    if grep -q "features compat-key-order" "$WORK/postrust.log" 2>/dev/null; then
        echo "error: the candidate was built without compat-key-order." >&2
        echo "       Its object keys will be alphabetical and every CSV column" >&2
        echo "       order case will diverge for a reason that is not a bug." >&2
        exit 1
    fi
}

say "Fetching PostgREST $PGRST_VERSION fixtures and specs"
if [ ! -d "$WORK/pgrst" ]; then
    curl -sL "https://github.com/PostgREST/postgrest/archive/refs/tags/$PGRST_VERSION.tar.gz" \
        -o "$WORK/pgrst.tar.gz"
    tar xzf "$WORK/pgrst.tar.gz" -C "$WORK"
    mv "$WORK/postgrest-${PGRST_VERSION#v}" "$WORK/pgrst"
    rm "$WORK/pgrst.tar.gz"
fi

say "Starting fixture database"
cleanup
docker network create "$NET" >/dev/null
docker run -d --name "$DB" --network "$NET" \
    -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=pgrst_conf \
    -p "$DB_PORT:5432" "$DB_IMAGE" >/dev/null
for _ in $(seq 1 60); do
    docker exec "$DB" pg_isready -h 127.0.0.1 -U postgres >/dev/null 2>&1 && break
    sleep 1
done
docker cp "$WORK/pgrst/test" "$DB:/pgrst-test" >/dev/null

psql_do() {
    docker exec -e PGPASSWORD=postgres "$DB" \
        psql -q -U postgres -d "$1" -v ON_ERROR_STOP=1 "${@:2}"
}

# Full reload, schema included. This drops and recreates the schemas, which
# changes OIDs, so any running server must be restarted afterwards.
full_load() {
    psql_do postgres -c "DROP DATABASE IF EXISTS pgrst_conf WITH (FORCE);" \
                     -c "CREATE DATABASE pgrst_conf;" >/dev/null
    psql_do pgrst_conf -v DBNAME=pgrst_conf -v PGUSER=postgres \
        -f /pgrst-test/spec/fixtures/load.sql >/dev/null
}

# Data-only restore, used between mutating cases. No DDL runs, so OIDs are
# stable and neither server needs restarting.
#
# We reload a snapshot taken after the first full load rather than re-running
# data.sql, for two reasons: some fixture rows come from schema.sql and are
# absent from data.sql, and data.sql is not idempotent (a few tables are
# inserted into with no preceding TRUNCATE, so a second run duplicates rows).
write_reset_assets() {
    cat > "$WORK/truncate-all.sql" <<'SQL'
DO $$
DECLARE tables text;
BEGIN
    -- Found by exclusion, not listed: the fixtures include schemas named
    -- `تست` and `SPECIAL "@/\#~_-|`, and a missed schema surfaces later as a
    -- duplicate-key error on reload.
    SELECT string_agg(format('%I.%I', schemaname, tablename), ', ')
      INTO tables FROM pg_tables
     WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
       AND schemaname NOT LIKE 'pg\_%';
    IF tables IS NOT NULL THEN
        EXECUTE 'TRUNCATE TABLE ' || tables || ' CASCADE';
    END IF;
END $$;
SQL
    docker exec -e PGPASSWORD=postgres "$DB" pg_dump -U postgres -d pgrst_conf \
        --data-only --disable-triggers --no-owner > "$WORK/snapshot.sql" 2>/dev/null
    docker cp "$WORK/truncate-all.sql" "$DB:/truncate-all.sql" >/dev/null
    docker cp "$WORK/snapshot.sql" "$DB:/snapshot.sql" >/dev/null
}

RESET_CMD="docker exec -e PGPASSWORD=postgres $DB psql -q -U postgres -d pgrst_conf \
    -v ON_ERROR_STOP=1 -f /truncate-all.sql -f /snapshot.sql"

say "Loading fixtures"
full_load
write_reset_assets

say "Extracting request cases from the spec suite"
python3 "$HERE/extract.py" "$WORK/pgrst/test/spec/Feature" "$WORK/cases.json"

# A probe run: only the specs named, so a change can be measured in minutes
# rather than in an hour. Point CONFORMANCE_WORK somewhere of its own when
# using this -- a filtered `ref.json` must not be mistaken for a full one.
if [ -n "${CONFORMANCE_SPECS:-}" ]; then
    python3 - "$WORK/cases.json" "$CONFORMANCE_SPECS" <<'FILTER'
import json, re, sys
path, pattern = sys.argv[1], sys.argv[2]
cases = [c for c in json.load(open(path)) if re.search(pattern, c["spec"])]
json.dump(cases, open(path, "w"))
print("  filtered to %d cases matching %s" % (len(cases), pattern))
FILTER
fi

# The reference run is the expensive half and its answers only change when
# PostgREST's version or the fixtures do, so a previous one can be reused
# while iterating on the candidate. Set CONFORMANCE_REUSE_REF=1 to do that.
if [ "${CONFORMANCE_REUSE_REF:-}" = "1" ] && [ -s "$WORK/ref.json" ]; then
    say "Reusing the recorded PostgREST $PGRST_VERSION responses"
else
    say "Replaying against PostgREST $PGRST_VERSION (reference)"
    docker rm -f "$REF" >/dev/null 2>&1 || true
    docker run -d --name "$REF" --network "$NET" -p "$REF_PORT:3000" \
        -e PGRST_DB_URI="postgres://postgres:postgres@$DB:5432/pgrst_conf" \
        -e PGRST_DB_SCHEMAS="$SCHEMA" -e PGRST_DB_ANON_ROLE="$ANON_ROLE" \
        -e PGRST_DB_EXTRA_SEARCH_PATH="$EXTRA_SEARCH_PATH" \
        -e PGRST_JWT_SECRET="$JWT_SECRET" -e PGRST_SERVER_PORT=3000 \
        "postgrest/postgrest:$PGRST_VERSION" >/dev/null
    sleep 9
    python3 "$HERE/run.py" "$WORK/cases.json" "http://localhost:$REF_PORT" \
        "$WORK/ref.json" "$RESET_CMD >/dev/null 2>&1"
fi

say "Replaying against Postrust (candidate)"
full_load
write_reset_assets
pkill -f 'target/release/postrust' 2>/dev/null || true
sleep 2
DATABASE_URL="postgres://postgres:postgres@localhost:$DB_PORT/pgrst_conf" \
PGRST_DB_SCHEMAS="$SCHEMA" PGRST_DB_ANON_ROLE="$ANON_ROLE" PGRST_JWT_SECRET="$JWT_SECRET" \
PGRST_DB_EXTRA_SEARCH_PATH="$EXTRA_SEARCH_PATH" \
PGRST_SERVER_PORT="$CAND_PORT" PGRST_SERVER_HOST=127.0.0.1 \
PGRST_COMPAT_MODE=true PGRST_LOG_LEVEL=warn \
    "$REPO_ROOT/target/release/postrust" >"$WORK/postrust.log" 2>&1 &
sleep 9
assert_key_order
python3 "$HERE/run.py" "$WORK/cases.json" "http://localhost:$CAND_PORT" \
    "$WORK/cand.json" "$RESET_CMD >/dev/null 2>&1"

# What was actually measured, written beside the diff so that anything
# publishing these numbers reads it from the run rather than from whatever was
# typed on a command line. The features are the ones this script built with a
# few lines up, and `assert_key_order` has already confirmed the running binary
# agrees; the commit is what produced it.
#
# Run 4 is why: it was measured with a binary `cargo test --workspace` had
# quietly rebuilt without `compat-key-order`, and nothing in its output said so.
cat > "$WORK/run-meta.json" <<META
{
  "postgrest": "$PGRST_VERSION",
  "features": "admin-ui,compat-key-order",
  "compatMode": true,
  "commit": "$(git -C "$REPO_ROOT" rev-parse HEAD)",
  "measured": "$(date -u +%Y-%m-%d)"
}
META

say "Report"
python3 "$HERE/report.py" "$WORK/ref.json" "$WORK/cand.json" "$WORK/diff.json" \
    | tee "$WORK/report.txt"
