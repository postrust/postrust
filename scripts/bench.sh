#!/usr/bin/env bash
# Postrust Benchmark Harness
#
# Measures the three numbers the project advertises -- release binary size,
# request latency/throughput, and resident memory -- against a throwaway
# PostgreSQL container, and prints them as a table.
#
# Usage:
#   scripts/bench.sh                    # run everything, tear down afterwards
#   REQUESTS=10000 CONCURRENCY=100 scripts/bench.sh
#   KEEP=1 scripts/bench.sh             # leave the database and server running
#   SKIP_BUILD=1 scripts/bench.sh       # reuse an existing release build
#
# Requirements: docker, cargo, curl, and a load generator -- either oha
# (preferred; also needs jq) or ab, which ships with macOS.

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

PG_IMAGE="${PG_IMAGE:-postgres:16}"
PG_PORT="${PG_PORT:-55432}"
PG_CONTAINER="${PG_CONTAINER:-postrust-bench-pg}"
PG_DB="${PG_DB:-postrust_bench}"

BENCH_HOST="${BENCH_HOST:-127.0.0.1}"
BENCH_PORT="${BENCH_PORT:-3999}"

REQUESTS="${REQUESTS:-3000}"
CONCURRENCY="${CONCURRENCY:-50}"
WARMUP="${WARMUP:-200}"

KEEP="${KEEP:-0}"
SKIP_BUILD="${SKIP_BUILD:-0}"

# Feature set to build. Defaults to the set the published Docker image and
# release binaries are built with, so the reported size matches what users get.
# Set to an empty string to measure a minimal build.
BENCH_FEATURES="${BENCH_FEATURES-admin-ui}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/lib/bench-lib.sh
source "$REPO_ROOT/scripts/lib/bench-lib.sh"
BINARY="$REPO_ROOT/target/release/postrust"
BASE_URL="http://$BENCH_HOST:$BENCH_PORT"
RESULTS_DIR="${RESULTS_DIR:-$(mktemp -d)}"
SERVER_LOG="$RESULTS_DIR/server.log"
SERVER_PID=""

# Scenarios: name|path|extra curl/load-generator header (may be empty)
SCENARIOS=(
    "point lookup (id=eq.N)|/api/bench_items?id=eq.42|"
    "25-row page|/api/bench_items?select=id,name,price&limit=25|"
    "filtered + ordered page|/api/bench_items?category=eq.cat-5&order=id.desc&select=id,name&limit=25|"
    "page with exact count|/api/bench_items?select=id,name&limit=25|Prefer: count=exact"
    "range filter on numeric|/api/bench_items?price=gt.50&select=id,price&limit=25|"
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

cleanup() {
    local exit_code=$?

    if [[ "$KEEP" == "1" ]]; then
        log "KEEP=1: leaving server (pid ${SERVER_PID:-none}) and container $PG_CONTAINER running"
        log "results and server log in $RESULTS_DIR"
        return $exit_code
    fi

    if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true

    return $exit_code
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

require docker
require cargo
require curl
LOAD_TOOL="$(pick_load_generator)"
[[ "$LOAD_TOOL" == "oha" ]] && require jq

log "load generator: $LOAD_TOOL ($REQUESTS requests, concurrency $CONCURRENCY)"
log "results directory: $RESULTS_DIR"

# --- Build -----------------------------------------------------------------

if [[ "$SKIP_BUILD" == "1" ]]; then
    [[ -x "$BINARY" ]] || die "SKIP_BUILD=1 but $BINARY does not exist"
    warn "SKIP_BUILD=1: reusing $BINARY -- its feature set may not match BENCH_FEATURES"
    BUILD_FEATURES="unknown (SKIP_BUILD=1)"
else
    if [[ -n "$BENCH_FEATURES" ]]; then
        log "building release binary (features: $BENCH_FEATURES)..."
        (cd "$REPO_ROOT" && cargo build --release --package postrust-server --features "$BENCH_FEATURES")
        BUILD_FEATURES="$BENCH_FEATURES"
    else
        log "building release binary (default features)..."
        (cd "$REPO_ROOT" && cargo build --release --package postrust-server)
        BUILD_FEATURES="default"
    fi
fi

BINARY_BYTES="$(wc -c < "$BINARY" | tr -d ' ')"
BINARY_MIB="$(awk -v b="$BINARY_BYTES" 'BEGIN { printf "%.2f", b / 1048576 }')"
BINARY_STRIPPED="no"
if command -v file >/dev/null 2>&1 && ! file "$BINARY" | grep -qi 'not stripped'; then
    BINARY_STRIPPED="yes"
fi

# --- Database --------------------------------------------------------------

log "starting PostgreSQL ($PG_IMAGE) on port $PG_PORT..."
docker rm -f "$PG_CONTAINER" >/dev/null 2>&1 || true
docker run -d --rm \
    --name "$PG_CONTAINER" \
    -e POSTGRES_PASSWORD=postgres \
    -e POSTGRES_DB="$PG_DB" \
    -p "$PG_PORT:5432" \
    "$PG_IMAGE" >/dev/null

# `-h 127.0.0.1` forces the check over TCP. The official image runs a
# temporary socket-only server while it initialises the data directory, so a
# socket-based pg_isready can report ready and then the connection fails with
# "the database system is shutting down" as that server is replaced.
if ! wait_until 90 docker exec "$PG_CONTAINER" \
    pg_isready -h 127.0.0.1 -U postgres -d "$PG_DB"; then
    docker logs "$PG_CONTAINER" 2>&1 | tail -20 >&2
    die "PostgreSQL did not become ready within 90s"
fi

log "loading benchmark fixtures (100k rows)..."
docker exec -i "$PG_CONTAINER" \
    psql -q -v ON_ERROR_STOP=1 -U postgres -d "$PG_DB" \
    < "$REPO_ROOT/scripts/bench-fixtures.sql" >/dev/null

# --- Server ----------------------------------------------------------------

log "starting postrust on port $BENCH_PORT..."
DATABASE_URL="postgres://postgres:postgres@127.0.0.1:$PG_PORT/$PG_DB" \
PGRST_DB_ANON_ROLE=bench_anon \
PGRST_SERVER_PORT="$BENCH_PORT" \
PGRST_LOG_LEVEL=warn \
    "$BINARY" > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

if ! wait_until 45 curl -fsS -o /dev/null "$BASE_URL/_/health"; then
    cat "$SERVER_LOG" >&2
    if kill -0 "$SERVER_PID" 2>/dev/null; then
        die "server did not become healthy within 45s"
    else
        die "server exited during startup"
    fi
fi

# Memory before any request has been served.
RSS_IDLE="$(rss_kb "$SERVER_PID")"

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

RESULTS_FILE="$RESULTS_DIR/results.txt"
: > "$RESULTS_FILE"

for scenario in "${SCENARIOS[@]}"; do
    IFS='|' read -r name path header <<<"$scenario"
    url="$BASE_URL$path"

    # Verify the scenario actually succeeds before measuring it -- a benchmark
    # of an error path looks fast and means nothing.
    if [[ -n "$header" ]]; then
        status="$(curl -s -o /dev/null -w '%{http_code}' -H "$header" "$url")"
    else
        status="$(curl -s -o /dev/null -w '%{http_code}' "$url")"
    fi
    if [[ "$status" != "200" && "$status" != "206" ]]; then
        warn "skipping '$name': returned HTTP $status"
        printf '%s\tSKIPPED (HTTP %s)\n' "$name" "$status" >> "$RESULTS_FILE"
        continue
    fi

    log "benchmarking: $name"

    # Warm up so the first measured request is not paying for pool setup.
    for _ in $(seq 1 "$((WARMUP / 50 + 1))"); do
        if [[ -n "$header" ]]; then
            curl -s -o /dev/null -H "$header" "$url" || true
        else
            curl -s -o /dev/null "$url" || true
        fi
    done

    measured="$(run_load "$url" "$header")"
    rss_after="$(rss_kb "$SERVER_PID")"

    printf '%s\t%s\t%s\n' "$name" "$measured" "$rss_after" >> "$RESULTS_FILE"
done

RSS_FINAL="$(rss_kb "$SERVER_PID")"

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------

echo
echo "==========================================================================="
echo " Postrust benchmark"
echo "==========================================================================="
printf ' host           : %s\n' "$(uname -srm)"
printf ' postgres       : %s\n' "$PG_IMAGE"
printf ' load generator : %s (n=%s, c=%s)\n' "$LOAD_TOOL" "$REQUESTS" "$CONCURRENCY"
printf ' dataset        : bench_items, 100000 rows\n'
echo
printf ' binary         : %s bytes (%s MiB), stripped: %s\n' \
    "$BINARY_BYTES" "$BINARY_MIB" "$BINARY_STRIPPED"
printf ' features       : %s\n' "$BUILD_FEATURES"
printf ' memory (idle)  : %s\n' "$(human_kb "$RSS_IDLE")"
printf ' memory (final) : %s\n' "$(human_kb "$RSS_FINAL")"
echo
printf ' %-28s %9s %8s %8s %8s %10s\n' "scenario" "req/s" "p50 ms" "p95 ms" "p99 ms" "RSS"
printf ' %-28s %9s %8s %8s %8s %10s\n' \
    "----------------------------" "---------" "--------" "--------" "--------" "----------"

while IFS=$'\t' read -r name measured rss; do
    if [[ "$measured" == SKIPPED* || "$measured" == ERROR* ]]; then
        printf ' %-28s %9s\n' "$name" "$measured"
        continue
    fi
    read -r rps p50 p95 p99 <<<"$measured"
    printf ' %-28s %9s %8s %8s %8s %10s\n' \
        "$name" "$rps" "$p50" "$p95" "$p99" "$(human_kb "$rss")"
done < "$RESULTS_FILE"

echo
echo " Numbers are from a single machine over loopback: PostgreSQL, the server"
echo " and the load generator all compete for the same cores, so treat these as"
echo " relative measurements, not absolute capacity."
echo "==========================================================================="
