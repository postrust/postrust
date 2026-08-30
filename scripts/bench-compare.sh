#!/usr/bin/env bash
# Postrust Comparison Benchmark
#
# Measures Postrust against the other tools that generate an API from a
# PostgreSQL schema -- PostgREST, Hasura and PostGraphile -- under conditions
# that are the same for every tool:
#
#   * one PostgreSQL container, one dataset, loaded once
#   * every server runs as a container on the same docker network, so none of
#     them skips container overhead
#   * memory is read the same way for all four (docker stats)
#   * the same request count and concurrency, from the same load generator
#   * each tool keeps its own default pool and worker settings
#
# That last point is deliberate. Tuning one tool and not the others produces a
# number that says more about the tuning than the tool.
#
# Two matrices are produced, because Hasura and PostGraphile expose no REST
# surface and PostgREST exposes no GraphQL:
#
#   REST     -- postrust vs postgrest
#   GraphQL  -- postrust vs hasura vs postgraphile
#
# Usage:
#   scripts/bench-compare.sh                  # run everything, tear down after
#   REQUESTS=10000 CONCURRENCY=100 scripts/bench-compare.sh
#   KEEP=1 scripts/bench-compare.sh           # leave containers running
#   ONLY=postrust,postgrest scripts/bench-compare.sh
#
# Results are written as a table to stdout and as results.json in $RESULTS_DIR,
# so published figures can be copied from a file rather than retyped from a
# terminal.
#
# Requirements: docker, curl, jq, and oha as the load generator. ab cannot
# drive the GraphQL targets usefully here, so oha is required rather than
# preferred.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/lib/bench-lib.sh
source "$REPO_ROOT/scripts/lib/bench-lib.sh"

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

# Base-image variant. Alpine images are smaller, which is the number the
# website quotes, so both are measured rather than one being assumed to stand
# in for the other.
#
# Note: PostgREST and Hasura publish no Alpine images, so those two targets run
# the same image in both variants. That is reported rather than papered over.
VARIANT="${VARIANT:-debian}"

case "$VARIANT" in
    debian)
        PG_IMAGE_DEFAULT="postgres:16"
        POSTRUST_DOCKERFILE="Dockerfile"
        POSTGRAPHILE_NODE_DEFAULT="node:22"
        ;;
    alpine)
        # Not `postgres:16-alpine`: its arm64 build ships an empty /etc/passwd
        # and exits before starting.
        # https://github.com/docker-library/postgres/issues/1418
        PG_IMAGE_DEFAULT="postgres:16.11-alpine"
        POSTRUST_DOCKERFILE="Dockerfile.alpine"
        POSTGRAPHILE_NODE_DEFAULT="node:22-alpine"
        ;;
    *)
        echo "VARIANT must be 'debian' or 'alpine', got '$VARIANT'" >&2
        exit 1
        ;;
esac

PG_IMAGE="${PG_IMAGE:-$PG_IMAGE_DEFAULT}"
PG_DB="${PG_DB:-postrust_bench}"
PG_META_DB="${PG_META_DB:-hasura_metadata}"

NETWORK="${NETWORK:-postrust-bench-net}"
PG_CONTAINER="postrust-cmp-pg"

# Pinned so a re-run measures the same thing. Bump deliberately, not by drift.
# Each is the current release: benchmarking an old version of someone else's
# tool produces a number that is not worth publishing.
#
# The Hasura pin must match the one scripts/hasura-conformance/conformance.sh
# measures against. It did not for a while -- this benchmarked v2.44.0 while
# conformance reported v2.50.1 -- which put two numbers about "Hasura" on the
# same website that were not about the same Hasura.
POSTGREST_IMAGE="${POSTGREST_IMAGE:-postgrest/postgrest:v16.1}"
HASURA_IMAGE="${HASURA_IMAGE:-hasura/graphql-engine:v2.50.1}"
POSTGRAPHILE_NODE_IMAGE="${POSTGRAPHILE_NODE_IMAGE:-$POSTGRAPHILE_NODE_DEFAULT}"
POSTGRAPHILE_VERSION="${POSTGRAPHILE_VERSION:-5}"

POSTRUST_IMAGE="${POSTRUST_IMAGE:-postrust:bench-$VARIANT}"
POSTGRAPHILE_IMAGE="${POSTGRAPHILE_IMAGE:-postgraphile:bench-$VARIANT}"

# Host ports. Each server also listens inside the network on its own port.
PORT_POSTRUST="${PORT_POSTRUST:-3991}"
PORT_POSTGREST="${PORT_POSTGREST:-3992}"
PORT_HASURA="${PORT_HASURA:-3993}"
PORT_POSTGRAPHILE="${PORT_POSTGRAPHILE:-3994}"

# 3000 requests completes in roughly a third of a second at the throughputs
# these tools reach, and a window that short measures scheduling and cache
# warmth as much as steady-state throughput. Repeating it does not help: five
# medians of a biased sample share the bias. Measured directly, going from
# 3000 to 30000 pulled the run-to-run spread on one scenario from 1.33x down
# to 1.25x for this server and 1.39x to 1.17x for PostGraphile.
REQUESTS="${REQUESTS:-30000}"
CONCURRENCY="${CONCURRENCY:-50}"
# Requests issued against every target before anything is measured. This is a
# real number of requests, not a token few: a cold connection pool, an unplanned
# statement and an empty buffer cache all show up as a slow first scenario
# otherwise.
WARMUP="${WARMUP:-500}"

# Each measurement is repeated and the median taken. A single run cannot tell a
# 20% change from the noise of a laptop, which is enough to draw a wrong
# conclusion from.
REPEATS="${REPEATS:-3}"

KEEP="${KEEP:-0}"
SKIP_BUILD="${SKIP_BUILD:-0}"
ONLY="${ONLY:-postrust,postgrest,hasura,postgraphile}"

RESULTS_DIR="${RESULTS_DIR:-$(mktemp -d)}"
mkdir -p "$RESULTS_DIR"
RESULTS_JSON="$RESULTS_DIR/results.json"
RESULTS_TSV="$RESULTS_DIR/results.tsv"

HASURA_SECRET="benchsecret"
PG_INTERNAL_URI="postgres://postgres:postgres@$PG_CONTAINER:5432/$PG_DB"

TARGETS=()
IFS=',' read -r -a TARGETS <<<"$ONLY"

wants() {
    local t
    for t in "${TARGETS[@]}"; do [[ "$t" == "$1" ]] && return 0; done
    return 1
}

# ---------------------------------------------------------------------------
# Scenarios
#
# Each tool expresses the same request in its own dialect: PostgREST serves
# tables at the root where Postrust mounts them under /api, and PostGraphile
# inflects field names its own way (allBenchItems, benchItemByRowId). The
# request being measured is the same; only the spelling differs. That is a
# property of the tools, and is stated on the published comparison rather than
# smoothed over.
#
# Postrust and Hasura are now the exception: they are sent the *same query
# text*, byte for byte, because Postrust answers Hasura's dialect. That is the
# claim the conformance harness measures, and sending one string to both is
# the cheapest possible restatement of it -- if the dialects ever drift, this
# benchmark stops running rather than quietly measuring two different
# questions.
# ---------------------------------------------------------------------------

# name|postrust path|postgrest path
REST_SCENARIOS=(
    "point lookup|/api/bench_items?id=eq.42|/bench_items?id=eq.42"
    "25-row page|/api/bench_items?select=id,name,price&limit=25|/bench_items?select=id,name,price&limit=25"
    "filtered + ordered page|/api/bench_items?category=eq.cat-5&order=id.desc&select=id,name&limit=25|/bench_items?category=eq.cat-5&order=id.desc&select=id,name&limit=25"
    "range filter on numeric|/api/bench_items?price=gt.50&select=id,price&limit=25|/bench_items?price=gt.50&select=id,price&limit=25"
    "25-row page + embed|/api/bench_items?select=id,name,bench_reviews(id,rating)&limit=25|/bench_items?select=id,name,bench_reviews(id,rating)&limit=25"
)

# GraphQL queries, one per tool per scenario. Kept as literal strings so what
# was measured is visible in the diff.
gql_query() {
    local target="$1" scenario="$2"

    case "$target:$scenario" in
    postrust:row|hasura:row)
                      echo '{ bench_items(where: {id: {_eq: 42}}) { id name price } }' ;;
    postgraphile:row) echo '{ benchItemByRowId(rowId: 42) { rowId name price } }' ;;

    postrust:page|hasura:page)
                      echo '{ bench_items(limit: 25) { id name price } }' ;;
    postgraphile:page) echo '{ allBenchItems(first: 25) { nodes { rowId name price } } }' ;;

    postrust:embed|hasura:embed)
                      echo '{ bench_items(limit: 25) { id name bench_reviews { id rating } } }' ;;
    postgraphile:embed) echo '{ allBenchItems(first: 25) { nodes { rowId name benchReviewsByItemId { nodes { rowId rating } } } } }' ;;

    *) return 1 ;;
    esac
}

GQL_SCENARIOS=(
    "row|single row by primary key"
    "page|25-row page"
    "embed|25-row page + embed"
)

gql_endpoint() {
    case "$1" in
        # `/v1/graphql`, not `/api/graphql`: same handler either way, but this
        # is the address a Hasura client is pointed at, so the benchmark
        # exercises the migration path rather than one beside it.
        postrust)     echo "http://127.0.0.1:$PORT_POSTRUST/v1/graphql" ;;
        hasura)       echo "http://127.0.0.1:$PORT_HASURA/v1/graphql" ;;
        postgraphile) echo "http://127.0.0.1:$PORT_POSTGRAPHILE/graphql" ;;
    esac
}

container_of() {
    case "$1" in
        postrust)     echo "postrust-cmp-postrust" ;;
        postgrest)    echo "postrust-cmp-postgrest" ;;
        hasura)       echo "postrust-cmp-hasura" ;;
        postgraphile) echo "postrust-cmp-postgraphile" ;;
    esac
}

# Only the server being measured should be running.
#
# Every candidate used to stay up for the whole run, so each measurement was
# taken with the other three idling beside it on the same host. That is not a
# fixed overhead shared equally: it moved this server's GraphQL throughput from
# ~9000 rps measured alone to 3938-5111 measured alongside, while PostGraphile
# barely shifted -- so the comparison was reporting a difference in how the
# tools respond to a busy host, on top of the difference being asked about.
#
# Paused rather than stopped: `docker pause` freezes the processes through the
# cgroup freezer without touching their state, so nothing pays a cold start or
# loses a warm connection pool between scenarios.
isolate() {
    local keep="$1" t c
    for t in postrust postgrest hasura postgraphile; do
        wants "$t" || continue
        c="$(container_of "$t")"
        if [ "$t" = "$keep" ]; then
            docker unpause "$c" >/dev/null 2>&1 || true
        else
            docker pause "$c" >/dev/null 2>&1 || true
        fi
    done
}

# Everything running again, for warmup and for the memory reading at the end.
unisolate() {
    local t
    for t in postrust postgrest hasura postgraphile; do
        wants "$t" || continue
        docker unpause "$(container_of "$t")" >/dev/null 2>&1 || true
    done
}

image_of() {
    case "$1" in
        postrust)     echo "$POSTRUST_IMAGE" ;;
        postgrest)    echo "$POSTGREST_IMAGE" ;;
        hasura)       echo "$HASURA_IMAGE" ;;
        postgraphile) echo "$POSTGRAPHILE_IMAGE" ;;
    esac
}

# ---------------------------------------------------------------------------
# Lifecycle
# ---------------------------------------------------------------------------

ALL_CONTAINERS=(
    "$PG_CONTAINER"
    postrust-cmp-postrust
    postrust-cmp-postgrest
    postrust-cmp-hasura
    postrust-cmp-postgraphile
)

cleanup() {
    local exit_code=$?

    # A run that dies mid-measurement leaves one server frozen by `isolate`.
    # Unfreezing first means neither KEEP=1 nor the next run inherits a
    # container that is up but answers nothing.
    unisolate 2>/dev/null || true

    if [[ "$KEEP" == "1" ]]; then
        log "KEEP=1: leaving containers and network $NETWORK running"
        log "results in $RESULTS_DIR"
        return $exit_code
    fi

    local c
    for c in "${ALL_CONTAINERS[@]}"; do
        docker rm -f "$c" >/dev/null 2>&1 || true
    done
    docker network rm "$NETWORK" >/dev/null 2>&1 || true

    return $exit_code
}
trap cleanup EXIT

# A GraphQL error is returned with HTTP 200 and an "errors" key. Checking only
# the status code would benchmark the error path, which looks fast and means
# nothing.
gql_ok() {
    local url="$1" query="$2" header="${3:-}" body response
    body="$(jq -nc --arg q "$query" '{query: $q}')"

    if [[ -n "$header" ]]; then
        response="$(curl -s -H 'Content-Type: application/json' -H "$header" -d "$body" "$url" 2>&1)"
    else
        response="$(curl -s -H 'Content-Type: application/json' -d "$body" "$url" 2>&1)"
    fi

    if jq -e '.errors' >/dev/null 2>&1 <<<"$response"; then
        printf '%s' "$(jq -r '.errors[0].message // "unknown error"' <<<"$response" 2>/dev/null)"
        return 1
    fi
    if ! jq -e '.data' >/dev/null 2>&1 <<<"$response"; then
        printf '%s' "no data key in response: ${response:0:160}"
        return 1
    fi
    return 0
}

rest_ok() {
    local url="$1" status
    status="$(curl -s -o /dev/null -w '%{http_code}' "$url")"
    [[ "$status" == "200" || "$status" == "206" ]] && return 0
    printf 'HTTP %s' "$status"
    return 1
}

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------

require docker
require curl
require jq
require oha
LOAD_TOOL="oha"

docker info >/dev/null 2>&1 || die "docker daemon is not responding"

# A process already holding one of these ports produces a confusing failure:
# docker cannot publish the port, the health check reaches whatever is already
# listening, and the target is reported unhealthy after a 90s wait. Checking up
# front turns that into one clear line.
check_port_free() {
    local port="$1" label="$2"
    if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
        die "port $port ($label) is already in use -- stop whatever is listening, or set PORT_${label}"
    fi
}

if wants postrust;     then check_port_free "$PORT_POSTRUST" POSTRUST; fi
if wants postgrest;    then check_port_free "$PORT_POSTGREST" POSTGREST; fi
if wants hasura;       then check_port_free "$PORT_HASURA" HASURA; fi
if wants postgraphile; then check_port_free "$PORT_POSTGRAPHILE" POSTGRAPHILE; fi

log "results directory: $RESULTS_DIR"
log "targets: ${TARGETS[*]}"

# --- Images ----------------------------------------------------------------

if [[ "$SKIP_BUILD" != "1" ]] && wants postrust; then
    log "building $POSTRUST_IMAGE from ./$POSTRUST_DOCKERFILE (this is the slow step)..."
    docker build -q -t "$POSTRUST_IMAGE" -f "$REPO_ROOT/$POSTRUST_DOCKERFILE" "$REPO_ROOT" >/dev/null \
        || die "failed to build the postrust image"
fi

if [[ "$SKIP_BUILD" != "1" ]] && wants postgraphile; then
    log "building $POSTGRAPHILE_IMAGE (postgraphile v$POSTGRAPHILE_VERSION)..."
    # There is no official V5 image, so one is built here. Pinned to a major so
    # the comparison is against current PostGraphile rather than V4.
    docker build -q -t "$POSTGRAPHILE_IMAGE" -f - "$REPO_ROOT" >/dev/null <<EOF || die "failed to build the postgraphile image"
FROM $POSTGRAPHILE_NODE_IMAGE
RUN npm install -g postgraphile@$POSTGRAPHILE_VERSION @graphile/simplify-inflection
EXPOSE 5000
EOF
fi

for t in "${TARGETS[@]}"; do
    case "$t" in
        postgrest) docker pull -q "$POSTGREST_IMAGE" >/dev/null || die "cannot pull $POSTGREST_IMAGE" ;;
        hasura)    docker pull -q "$HASURA_IMAGE" >/dev/null    || die "cannot pull $HASURA_IMAGE" ;;
    esac
done

# ---------------------------------------------------------------------------
# Database
# ---------------------------------------------------------------------------

cleanup_quiet() {
    local c
    for c in "${ALL_CONTAINERS[@]}"; do
        docker unpause "$c" >/dev/null 2>&1 || true
        docker rm -f "$c" >/dev/null 2>&1 || true
    done
}
cleanup_quiet
docker network rm "$NETWORK" >/dev/null 2>&1 || true
docker network create "$NETWORK" >/dev/null

log "starting postgres ($PG_IMAGE)..."
docker run -d --name "$PG_CONTAINER" --network "$NETWORK" \
    -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB="$PG_DB" \
    "$PG_IMAGE" >/dev/null

# `-h 127.0.0.1` forces a TCP check. The official image runs a temporary
# socket-only server while it initialises, so a socket check can pass against
# that server and the next connection fails as it is replaced.
wait_until 90 docker exec "$PG_CONTAINER" pg_isready -h 127.0.0.1 -U postgres -d "$PG_DB" \
    || { docker logs "$PG_CONTAINER" 2>&1 | tail -20 >&2; die "postgres did not become ready"; }

log "loading fixtures (100k items, 300k reviews)..."
docker exec -i "$PG_CONTAINER" psql -q -v ON_ERROR_STOP=1 -U postgres -d "$PG_DB" \
    < "$REPO_ROOT/scripts/bench-fixtures.sql" >/dev/null

# Pull both tables and their indexes into the buffer cache before any tool is
# measured. Without this the first target to run each scenario pays to populate
# the cache and every later one reads it warm, which is a difference between
# tools that has nothing to do with the tools.
log "warming the database cache..."
docker exec "$PG_CONTAINER" psql -q -U postgres -d "$PG_DB" -c "
    SELECT count(*) FROM public.bench_items;
    SELECT count(*) FROM public.bench_reviews;
    SELECT count(*) FROM public.bench_items WHERE category = 'cat-5';
    SELECT count(*) FROM public.bench_reviews WHERE item_id < 1000;
" >/dev/null 2>&1 || warn "database cache warm-up query failed"

if wants hasura; then
    # Hasura writes its own catalog into whatever database it is pointed at.
    # Giving it a separate metadata database keeps hdb_catalog out of the
    # benchmark database, so every tool queries the same schema.
    docker exec "$PG_CONTAINER" psql -q -U postgres -d postgres \
        -c "CREATE DATABASE $PG_META_DB" >/dev/null 2>&1 || true
fi

# ---------------------------------------------------------------------------
# Servers
# ---------------------------------------------------------------------------

start_postrust() {
    log "starting postrust..."
    docker run -d --name postrust-cmp-postrust --network "$NETWORK" \
        -p "$PORT_POSTRUST:3000" \
        -e DATABASE_URL="$PG_INTERNAL_URI" \
        -e PGRST_DB_ANON_ROLE=bench_anon \
        -e PGRST_SERVER_PORT=3000 \
        -e PGRST_SERVER_HOST=0.0.0.0 \
        -e PGRST_LOG_LEVEL=warn \
        "$POSTRUST_IMAGE" >/dev/null

    wait_until 60 curl -fsS -o /dev/null "http://127.0.0.1:$PORT_POSTRUST/_/health" \
        || { docker logs postrust-cmp-postrust 2>&1 | tail -20 >&2; die "postrust did not become healthy"; }
}

start_postgrest() {
    log "starting postgrest..."
    docker run -d --name postrust-cmp-postgrest --network "$NETWORK" \
        -p "$PORT_POSTGREST:3000" \
        -e PGRST_DB_URI="$PG_INTERNAL_URI" \
        -e PGRST_DB_SCHEMAS=public \
        -e PGRST_DB_ANON_ROLE=bench_anon \
        -e PGRST_SERVER_PORT=3000 \
        -e PGRST_LOG_LEVEL=error \
        "$POSTGREST_IMAGE" >/dev/null

    wait_until 60 curl -fsS -o /dev/null "http://127.0.0.1:$PORT_POSTGREST/bench_items?limit=1" \
        || { docker logs postrust-cmp-postgrest 2>&1 | tail -20 >&2; die "postgrest did not become healthy"; }
}

start_hasura() {
    log "starting hasura..."
    docker run -d --name postrust-cmp-hasura --network "$NETWORK" \
        -p "$PORT_HASURA:8080" \
        -e HASURA_GRAPHQL_DATABASE_URL="$PG_INTERNAL_URI" \
        -e HASURA_GRAPHQL_METADATA_DATABASE_URL="postgres://postgres:postgres@$PG_CONTAINER:5432/$PG_META_DB" \
        -e HASURA_GRAPHQL_ADMIN_SECRET="$HASURA_SECRET" \
        -e HASURA_GRAPHQL_UNAUTHORIZED_ROLE=bench_anon \
        -e HASURA_GRAPHQL_ENABLE_CONSOLE=false \
        -e HASURA_GRAPHQL_ENABLE_TELEMETRY=false \
        "$HASURA_IMAGE" >/dev/null

    wait_until 120 curl -fsS -o /dev/null "http://127.0.0.1:$PORT_HASURA/healthz" \
        || { docker logs postrust-cmp-hasura 2>&1 | tail -30 >&2; die "hasura did not become healthy"; }

    # Hasura answers nothing until tables are tracked and the anonymous role has
    # select permission. Both are metadata API calls, not schema introspection.
    log "tracking tables in hasura..."
    local meta="http://127.0.0.1:$PORT_HASURA/v1/metadata"

    curl -fsS -o /dev/null -X POST "$meta" \
        -H "x-hasura-admin-secret: $HASURA_SECRET" \
        -H 'Content-Type: application/json' \
        -d '{"type":"pg_track_tables","args":{"allow_warnings":true,"tables":[
              {"table":{"schema":"public","name":"bench_items"}},
              {"table":{"schema":"public","name":"bench_reviews"}}]}}' \
        || die "hasura: failed to track tables"

    # Tracking a table does not create its relationships. Without these the
    # embed field simply is not in the schema, which reads like Hasura cannot
    # embed rather than like the benchmark forgot a setup step.
    curl -fsS -o /dev/null -X POST "$meta" \
        -H "x-hasura-admin-secret: $HASURA_SECRET" \
        -H 'Content-Type: application/json' \
        -d '{"type":"pg_create_array_relationship","args":{
              "table":{"schema":"public","name":"bench_items"},
              "name":"bench_reviews",
              "using":{"foreign_key_constraint_on":
                {"table":{"schema":"public","name":"bench_reviews"},"column":"item_id"}}}}' \
        || die "hasura: failed to create the bench_reviews relationship"

    curl -fsS -o /dev/null -X POST "$meta" \
        -H "x-hasura-admin-secret: $HASURA_SECRET" \
        -H 'Content-Type: application/json' \
        -d '{"type":"pg_create_object_relationship","args":{
              "table":{"schema":"public","name":"bench_reviews"},
              "name":"bench_item",
              "using":{"foreign_key_constraint_on":"item_id"}}}' \
        || die "hasura: failed to create the bench_item relationship"

    local tbl
    for tbl in bench_items bench_reviews; do
        curl -fsS -o /dev/null -X POST "$meta" \
            -H "x-hasura-admin-secret: $HASURA_SECRET" \
            -H 'Content-Type: application/json' \
            -d "{\"type\":\"pg_create_select_permission\",\"args\":{
                  \"table\":{\"schema\":\"public\",\"name\":\"$tbl\"},
                  \"role\":\"bench_anon\",
                  \"permission\":{\"columns\":\"*\",\"filter\":{},\"allow_aggregations\":true}}}" \
            || die "hasura: failed to grant select on $tbl"
    done
}

start_postgraphile() {
    log "starting postgraphile..."
    # --simple-inflection is deliberately NOT used: the default inflection is
    # what a default install serves, and the point is to measure defaults.
    docker run -d --name postrust-cmp-postgraphile --network "$NETWORK" \
        -p "$PORT_POSTGRAPHILE:5000" \
        "$POSTGRAPHILE_IMAGE" \
        sh -c "postgraphile --preset postgraphile/presets/amber -c '$PG_INTERNAL_URI' -n 0.0.0.0 -p 5000 --schema public" >/dev/null

    wait_until 90 curl -fsS -o /dev/null \
        -H 'Content-Type: application/json' \
        -d '{"query":"{__typename}"}' \
        "http://127.0.0.1:$PORT_POSTGRAPHILE/graphql" \
        || { docker logs postrust-cmp-postgraphile 2>&1 | tail -30 >&2; die "postgraphile did not become healthy"; }
}

if wants postrust;     then start_postrust;     fi
if wants postgrest;    then start_postgrest;    fi
if wants hasura;       then start_hasura;       fi
if wants postgraphile; then start_postgraphile; fi

# Memory before any request is served, same method for every tool.
# Indexed parallel to TARGETS: associative arrays would need bash 4, and macOS
# still ships 3.2.
RSS_IDLE=()
for i in "${!TARGETS[@]}"; do
    RSS_IDLE[$i]="$(container_rss_kb "$(container_of "${TARGETS[$i]}")")"
done

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

: > "$RESULTS_TSV"

# surface|scenario|target|rps|p50|p95|p99|status
record() {
    printf '%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" >> "$RESULTS_TSV"
}

# Run one measurement REPEATS times and print the median by throughput.
#
# The median rather than the best: a best-of-N flatters whichever tool got the
# quietest moment on the machine.
measure_median() {
    local url="$1" header="$2" body="${3:-}"
    local results=() line
    local i

    for ((i = 0; i < REPEATS; i++)); do
        line="$(run_load "$url" "$header" "$body")"
        # Errors are not averaged with successes -- report the first one.
        case "$line" in
            ERROR*) echo "$line"; return 0 ;;
        esac
        results+=("$line")
    done

    printf '%s\n' "${results[@]}" | sort -k1,1n | awk -v n="${#results[@]}" 'NR == int((n + 1) / 2)'
}

# Issue WARMUP requests without recording anything.
warm_target() {
    local url="$1" header="$2" body="${3:-}"
    local saved="$REQUESTS"
    REQUESTS="$WARMUP"
    run_load "$url" "$header" "$body" >/dev/null 2>&1 || true
    REQUESTS="$saved"
}

bench_rest() {
    local scenario name postrust_path postgrest_path target path url reason measured

    local urls=() entry
    for scenario in "${REST_SCENARIOS[@]}"; do
        IFS='|' read -r name postrust_path postgrest_path <<<"$scenario"
        urls=()

        for target in postrust postgrest; do
            wants "$target" || continue

            case "$target" in
                postrust)  path="$postrust_path";  url="http://127.0.0.1:$PORT_POSTRUST$path" ;;
                postgrest) path="$postgrest_path"; url="http://127.0.0.1:$PORT_POSTGREST$path" ;;
            esac

            if ! reason="$(rest_ok "$url")"; then
                warn "$target / $name: $reason -- not measured"
                record rest "$name" "$target" "UNSUPPORTED" "$reason"
                continue
            fi

            urls+=("$target|$url")
        done

        # Warm every target for this scenario before measuring any of them.
        # Measuring one tool while the cache is still cold and the next once it
        # is warm compares the cache, not the tools.
        for entry in "${urls[@]}"; do
            warm_target "${entry#*|}" ""
        done

        for entry in "${urls[@]}"; do
            target="${entry%%|*}"
            url="${entry#*|}"
            log "rest: $name -- $target"
            isolate "$target"
            measured="$(measure_median "$url" "")"
            record rest "$name" "$target" "$measured" ok
        done
        unisolate
    done
}

bench_gql() {
    local gql_entry entry key label target url query body reason measured header

    local posts=() _t _u _b
    for gql_entry in "${GQL_SCENARIOS[@]}"; do
        IFS='|' read -r key label <<<"$gql_entry"
        posts=()

        for target in postrust hasura postgraphile; do
            wants "$target" || continue

            url="$(gql_endpoint "$target")"
            query="$(gql_query "$target" "$key")" || continue
            body="$(jq -nc --arg q "$query" '{query: $q}')"
            header=""

            if ! reason="$(gql_ok "$url" "$query" "$header")"; then
                warn "$target / $label: $reason -- not measured"
                record graphql "$label" "$target" "UNSUPPORTED" "${reason:0:80}"
                continue
            fi

            # The body differs per target, so it travels with the endpoint.
            posts+=("$target|$url|$body")
        done

        for entry in "${posts[@]}"; do
            IFS='|' read -r _t _u _b <<<"$entry"
            warm_target "$_u" "" "$_b"
        done

        for entry in "${posts[@]}"; do
            IFS='|' read -r target url body <<<"$entry"
            log "graphql: $label -- $target"
            isolate "$target"
            measured="$(measure_median "$url" "" "$body")"
            record graphql "$label" "$target" "$measured" ok
        done
        unisolate
    done
}

bench_rest
bench_gql

RSS_FINAL=()
for i in "${!TARGETS[@]}"; do
    RSS_FINAL[$i]="$(container_rss_kb "$(container_of "${TARGETS[$i]}")")"
done

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------

echo
echo "==========================================================================="
echo " Postrust vs PostgREST, Hasura, PostGraphile"
echo "==========================================================================="
printf ' host           : %s\n' "$(uname -srm)"
printf ' variant        : %s\n' "$VARIANT"
printf ' postgres       : %s\n' "$PG_IMAGE"
printf ' load generator : %s (n=%s, c=%s)\n' "$LOAD_TOOL" "$REQUESTS" "$CONCURRENCY"
printf ' per measurement: median of %s runs, after %s warm-up requests\n' "$REPEATS" "$WARMUP"
printf ' dataset        : bench_items 100000 rows, bench_reviews 300000 rows\n'
printf ' all servers    : containers on %s, default settings\n' "$NETWORK"
echo

print_matrix() {
    local surface="$1" title="$2"
    grep -q "^$surface	" "$RESULTS_TSV" || return 0

    echo "$title"
    printf ' %-26s %-14s %10s %8s %8s %8s\n' scenario target "req/s" "p50 ms" "p95 ms" "p99 ms"
    printf ' %s\n' "-------------------------------------------------------------------------------"

    local sfc name target rest status rps p50 p95 p99
    while IFS=$'\t' read -r sfc name target rest status; do
        [[ "$sfc" == "$surface" ]] || continue
        if [[ "$rest" == "UNSUPPORTED" ]]; then
            printf ' %-26s %-14s %10s   (%s)\n' "$name" "$target" "n/a" "$status"
            continue
        fi
        read -r rps p50 p95 p99 <<<"$rest"
        printf ' %-26s %-14s %10s %8s %8s %8s\n' "$name" "$target" "$rps" "$p50" "$p95" "$p99"
    done < "$RESULTS_TSV"
    echo
}

print_matrix rest    "REST"
print_matrix graphql "GraphQL"

echo "Image size and memory"
printf ' %-14s %12s %12s %12s %12s\n' target "on disk" "layers" "idle RSS" "after load"
printf ' %s\n' "-----------------------------------------------------------------------"
for i in "${!TARGETS[@]}"; do
    printf ' %-14s %12s %12s %12s %12s\n' \
        "${TARGETS[$i]}" \
        "$(image_disk_size "$(image_of "${TARGETS[$i]}")")" \
        "$(human_bytes "$(image_layer_bytes "$(image_of "${TARGETS[$i]}")")")" \
        "$(human_kb "${RSS_IDLE[$i]:-0}")" \
        "$(human_kb "${RSS_FINAL[$i]:-0}")"
done
echo

# --- Machine-readable ------------------------------------------------------

{
    printf '{\n'
    printf '  "host": %s,\n' "$(jq -Rn --arg v "$(uname -srm)" '$v')"
    printf '  "variant": %s,\n' "$(jq -Rn --arg v "$VARIANT" '$v')"
    printf '  "postgres": %s,\n' "$(jq -Rn --arg v "$PG_IMAGE" '$v')"
    printf '  "requests": %s,\n' "$REQUESTS"
    printf '  "concurrency": %s,\n' "$CONCURRENCY"
    printf '  "repeats": %s,\n' "$REPEATS"
    printf '  "warmup": %s,\n' "$WARMUP"
    printf '  "dataset": "bench_items 100000 rows, bench_reviews 300000 rows",\n'
    printf '  "images": {\n'
    first=1
    for i in "${!TARGETS[@]}"; do
        [[ $first -eq 0 ]] && printf ',\n'
        first=0
        printf '    %s: {"image": %s, "size_layers_bytes": %s, "size_on_disk": %s, "rss_idle_kb": %s, "rss_after_kb": %s}' \
            "$(jq -Rn --arg v "${TARGETS[$i]}" '$v')" \
            "$(jq -Rn --arg v "$(image_of "${TARGETS[$i]}")" '$v')" \
            "$(image_layer_bytes "$(image_of "${TARGETS[$i]}")")" \
            "$(jq -Rn --arg v "$(image_disk_size "$(image_of "${TARGETS[$i]}")")" '$v')" \
            "${RSS_IDLE[$i]:-0}" "${RSS_FINAL[$i]:-0}"
    done
    printf '\n  },\n'
    printf '  "measurements": [\n'
    first=1
    while IFS=$'\t' read -r sfc name target rest status; do
        [[ $first -eq 0 ]] && printf ',\n'
        first=0
        if [[ "$rest" == "UNSUPPORTED" ]]; then
            printf '    {"surface": %s, "scenario": %s, "target": %s, "supported": false, "reason": %s}' \
                "$(jq -Rn --arg v "$sfc" '$v')" "$(jq -Rn --arg v "$name" '$v')" \
                "$(jq -Rn --arg v "$target" '$v')" "$(jq -Rn --arg v "$status" '$v')"
        else
            read -r rps p50 p95 p99 <<<"$rest"
            printf '    {"surface": %s, "scenario": %s, "target": %s, "supported": true, "rps": %s, "p50_ms": %s, "p95_ms": %s, "p99_ms": %s}' \
                "$(jq -Rn --arg v "$sfc" '$v')" "$(jq -Rn --arg v "$name" '$v')" \
                "$(jq -Rn --arg v "$target" '$v')" "$rps" "$p50" "$p95" "$p99"
        fi
    done < "$RESULTS_TSV"
    printf '\n  ]\n'
    printf '}\n'
} > "$RESULTS_JSON"

jq -e . "$RESULTS_JSON" >/dev/null || die "results.json is not valid JSON"

log "wrote $RESULTS_JSON"
echo "Numbers published on the website are copied from this file."
