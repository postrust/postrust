#!/bin/bash
# The Cloudflare Workers adapter, in the runtime it runs in.
#
# Builds the Worker with worker-build, serves it with `wrangler dev` -- which
# runs workerd, the Workers runtime itself -- and points its Hyperdrive binding
# at a PostgreSQL with row-level security through `localConnectionString`. The
# Worker logs in as the same least-privilege, NOINHERIT `authenticator` as the
# Lambda test (fixture.sql), and is asked the same questions.
#
# Usage:
#   scripts/serverless/worker.sh                  # build, then test
#   KEEP=1 scripts/serverless/worker.sh           # leave it running afterwards
#   BUILD=path/to/build scripts/serverless/worker.sh   # test a worker-build output
#   PREFIX=mine-wk PORT=59021 scripts/serverless/worker.sh   # beside another run
#
# Needs docker, python3 and curl. Containers, the network and volumes are all
# named `$PREFIX-*` (`postrust-wk-*` unless set) and removed on exit, except
# the build caches.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PORT="${PORT:-59020}"
PREFIX="${PREFIX:-postrust-wk}"
NET=$PREFIX-net
PG=$PREFIX-pg
WK=$PREFIX-worker
# Tokens are signed with SECRET; the Worker is given it base64-encoded, with
# PGRST_JWT_SECRET_IS_BASE64 as an unquoted TOML boolean -- which wrangler
# passes as a boolean, not a string, and which the Worker once dropped.
SECRET=reallyreallyreallyreallyverysafe
SECRET_B64=$(printf %s "$SECRET" | base64)
RELOAD=a-reload-secret
RUST_IMAGE="${RUST_IMAGE:-rust:1.98.1}"
NODE_IMAGE="${NODE_IMAGE:-node:22}"
# Pinned, like every tool a check depends on: worker-build must match the
# `worker` crate, and wrangler decides which workerd runs the test.
WORKER_BUILD_VERSION=0.8.6
WRANGLER_VERSION="${WRANGLER_VERSION:-4.40.0}"
OUT="$(mktemp -d)"

cleanup() {
    if [ "${KEEP:-0}" = 1 ]; then
        echo "KEEP=1: leaving $PG and $WK running on http://127.0.0.1:$PORT"
        return
    fi
    docker rm -f "$PG" "$WK" >/dev/null 2>&1
    docker network rm "$NET" >/dev/null 2>&1
    # wrangler runs as root in its container, and leaves .wrangler/ behind
    # owned by root; hand it back before removing it.
    if [ -d "$OUT/.wrangler" ]; then
        docker run --rm -v "$OUT":/out "$NODE_IMAGE" chown -R "$(id -u):$(id -g)" /out \
            >/dev/null 2>&1
    fi
    rm -rf "$OUT"
}
# fail MESSAGE -- stop here, with the Worker's log if it has one.
fail() {
    echo "$1" >&2
    docker logs "$WK" 2>&1 | tail -40 >&2
    exit 1
}
# wait_healthy -- until the Worker answers its health check, or fail.
wait_healthy() {
    for _ in $(seq 180); do
        curl -sf -o /dev/null "$URL/_/health" && return 0
        docker inspect -f '{{.State.Running}}' "$WK" 2>/dev/null | grep -q true \
            || fail "the Worker's container is not running"
        sleep 1
    done
    fail "the Worker never answered $URL/_/health"
}
trap cleanup EXIT
KEEP=0 cleanup
OUT="$(mktemp -d)"

if [ -n "${BUILD:-}" ]; then
    cp -r "$BUILD" "$OUT/build"
else
    echo "==> Building the Worker"
    # As root, since the caches are root's, but whatever it writes into the
    # source tree is handed back to whoever ran this, however the build ends.
    docker run --rm -v "$ROOT":/src -w /src/crates/postrust-worker \
        -v postrust-wk-cargo:/usr/local/cargo/registry \
        -v postrust-wk-bin:/usr/local/cargo/bin-extra \
        -v "$PREFIX-target":/target -e CARGO_TARGET_DIR=/target \
        -v postrust-wk-cache:/root/.cache \
        "$RUST_IMAGE" bash -c "
            set -e
            trap 'chown -R $(id -u):$(id -g) /src/crates/postrust-worker/build 2>/dev/null || true' EXIT
            rustup target add wasm32-unknown-unknown >/dev/null 2>&1
            export PATH=/usr/local/cargo/bin-extra/bin:\$PATH
            command -v worker-build >/dev/null || cargo install -q --root /usr/local/cargo/bin-extra \
                worker-build --version $WORKER_BUILD_VERSION --locked
            worker-build --release" \
        || { echo "build failed"; exit 1; }
    # The build writes into the source tree; take it from there.
    cp -r "$ROOT/crates/postrust-worker/build" "$OUT/build"
fi
echo "    wasm: $(wc -c < "$OUT/build/index_bg.wasm") bytes, $(gzip -9 -c "$OUT/build/index_bg.wasm" | wc -c) gzipped"

echo "==> Starting PostgreSQL and the Worker"
docker network create "$NET" >/dev/null
docker run -d --name "$PG" --network "$NET" -e POSTGRES_PASSWORD=postgres postgres:16-alpine >/dev/null
ready=0
for _ in $(seq 60); do
    docker exec "$PG" pg_isready -U postgres -q 2>/dev/null && { ready=1; break; }
    sleep 1
done
[ "$ready" = 1 ] || { docker logs "$PG" 2>&1 | tail -20; echo "PostgreSQL never became ready"; exit 1; }
sleep 1
docker exec -i "$PG" psql -q -v ON_ERROR_STOP=1 -U postgres < "$HERE/fixture.sql" || exit 1
# More rows than the default ceiling, so the ceiling has something to cut.
docker exec "$PG" psql -q -v ON_ERROR_STOP=1 -U postgres -c "CREATE TABLE many (id int PRIMARY KEY); \
    INSERT INTO many SELECT generate_series(1, 1500); GRANT SELECT ON many TO web_anon;" || exit 1

cat > "$OUT/wrangler.toml" <<TOML
name = "postrust-test"
main = "build/worker/shim.mjs"
compatibility_date = "2025-09-01"

[[hyperdrive]]
binding = "HYPERDRIVE"
id = "00000000000000000000000000000000"
localConnectionString = "postgres://authenticator:authenticator@$PG:5432/postgres?sslmode=disable"

[[kv_namespaces]]
binding = "POSTRUST_SCHEMA_CACHE"
id = "00000000000000000000000000000000"

[vars]
PGRST_DB_SCHEMAS = "public"
PGRST_DB_ANON_ROLE = "web_anon"
PGRST_SERVER_CORS_ORIGINS = "https://app.example"
# Unquoted, so wrangler passes a boolean and a number, not strings. The TTL
# also keeps the KV copy from expiring while the checks below run.
PGRST_JWT_SECRET_IS_BASE64 = true
PGRST_WORKER_SCHEMA_TTL = 600
TOML
cat > "$OUT/.dev.vars" <<VARS
PGRST_JWT_SECRET=$SECRET_B64
POSTRUST_RELOAD_SECRET=$RELOAD
VARS

docker run -d --name "$WK" --network "$NET" -p "127.0.0.1:$PORT:8787" \
    -v "$OUT":/app -w /app -v postrust-wk-npm:/root/.npm \
    "$NODE_IMAGE" npx --yes "wrangler@$WRANGLER_VERSION" dev --ip 0.0.0.0 --port 8787 \
        --test-scheduled --show-interactive-dev-session=false --log-level debug >/dev/null
URL="http://127.0.0.1:$PORT"
wait_healthy

ALICE=$(python3 "$HERE/jwt.py" "$SECRET" web_user alice)
BOB=$(python3 "$HERE/jwt.py" "$SECRET" web_user bob)
NOBODY=$(python3 "$HERE/jwt.py" "$SECRET" no_such_role mallory)

passed=0
failed=0
# expect LABEL STATUS BODY -- curl arguments...
#   BODY is JSON compared as a value, `~text` for a substring, `#N` for a JSON
#   array of N elements, or `*` for any.
expect() {
    local label="$1" status="$2" body="$3"
    shift 4
    local got_status got_body
    got_body=$(curl -s -o - -w '\n%{http_code}' "$@")
    got_status=$(tail -n1 <<<"$got_body")
    got_body=$(sed '$d' <<<"$got_body")
    if [ "$got_status" = "$status" ] && python3 - "$body" "$got_body" <<'PY'
import json, sys
want, got = sys.argv[1], sys.argv[2]
if want == "*":
    sys.exit(0)
if want.startswith("~"):
    sys.exit(0 if want[1:] in got else 1)
if want.startswith("#"):
    try:
        got = json.loads(got)
    except ValueError:
        sys.exit(1)
    sys.exit(0 if isinstance(got, list) and len(got) == int(want[1:]) else 1)
try:
    sys.exit(0 if json.loads(want) == json.loads(got) else 1)
except ValueError:
    sys.exit(1)
PY
    then
        printf '    ok    %s\n' "$label"
        passed=$((passed + 1))
    else
        printf '    FAIL  %s\n          want %s %s\n          got  %s %s\n' \
            "$label" "$status" "$body" "$got_status" "${got_body:0:300}"
        failed=$((failed + 1))
    fi
}
auth() { echo "Authorization: Bearer $1"; }

echo "==> REST"
expect "anon reads only public rows" 200 '[{"id":1,"task":"alice public"}]' \
    -- "$URL/todos?select=id,task&order=id"
expect "a filter binds its parameter" 200 '[{"id":1}]' \
    -- "$URL/todos?select=id&id=eq.1"
expect "alice reads her own rows" 200 '[{"id":1},{"id":2}]' \
    -- "$URL/todos?select=id&order=id" -H "$(auth "$ALICE")"
expect "bob reads his own rows" 200 '[{"id":3}]' \
    -- "$URL/todos?select=id&order=id" -H "$(auth "$BOB")"
expect "a role that does not exist is refused" 401 '*' \
    -- "$URL/todos" -H "$(auth "$NOBODY")"
expect "a token that does not verify is refused" 401 '*' \
    -- "$URL/todos" -H "Authorization: Bearer nope"
expect "anon cannot write" 401 '*' \
    -- -X POST "$URL/todos" -H "Content-Type: application/json" -d '{"owner":"mallory","task":"x"}'
expect "alice writes her own row" 201 '[{"owner":"alice","task":"new"}]' \
    -- -X POST "$URL/todos?select=owner,task" -H "$(auth "$ALICE")" \
       -H "Content-Type: application/json" -H "Prefer: return=representation" \
       -d '{"owner":"alice","task":"new"}'
expect "alice cannot write as bob" 401 '*' \
    -- -X POST "$URL/todos" -H "$(auth "$ALICE")" -H "Content-Type: application/json" \
       -d '{"owner":"bob","task":"x"}'
expect "alice cannot update bob's row" 200 '[]' \
    -- -X PATCH "$URL/todos?id=eq.3" -H "$(auth "$ALICE")" -H "Content-Type: application/json" \
       -H "Prefer: return=representation" -d '{"task":"hijacked"}'
expect "a function runs as the token's role, with its claims" 200 '[{"whoami":"web_user:alice"}]' \
    -- "$URL/rpc/whoami" -H "$(auth "$ALICE")"
expect "an exact count comes back in Content-Range" 200 '*' \
    -- "$URL/todos?select=id" -H "Prefer: count=exact" -H "$(auth "$ALICE")"
expect "a missing table names itself" 404 '~todo' \
    -- "$URL/todo"
expect "a singular read of many rows is refused" 406 '*' \
    -- "$URL/todos?select=id" -H "$(auth "$ALICE")" -H "Accept: application/vnd.pgrst.object+json"
expect "rows are capped at the default ceiling" 200 '#1000' \
    -- "$URL/many?select=id&limit=5000"

echo "==> One request cannot take the next one down"
# A write refused for its token never has its body read. In a Worker an
# unread request stream outlives the response and throws, which restarts the
# isolate and fails whatever request it is serving next -- found by
# PostgREST's suite, which sends exactly this sequence. It happens about half
# the time, so the log is checked as well: wrangler reports the exception at
# debug level.
for i in 1 2 3; do
    curl -s -o /dev/null -X POST "$URL/todos" -H "Authorization: Bearer nope" \
        -H "Content-Type: application/json" -d '{"owner":"x","task":"y"}'
    curl -s -o /dev/null -X POST "$URL/rpc/whoami" -H "$(auth "$NOBODY")" \
        -H "Content-Type: application/json" -d '{}'
    # Answered without looking at the body, before the engine is involved.
    curl -s -o /dev/null -X POST "$URL/_/reload" -H "Authorization: Bearer wrong" \
        -H "Content-Type: application/json" -d '{"a body":"nobody reads"}'
    curl -s -o /dev/null -X OPTIONS "$URL/todos" -H "Origin: https://app.example" \
        -d 'a preflight with a body'
done
expect "the request after refused writes is answered" 200 '[{"id":1}]' \
    -- "$URL/todos?select=id&id=eq.1"
expect "and a write after them lands" 200 '[]' \
    -- -X PATCH "$URL/todos?id=eq.999" -H "$(auth "$ALICE")" -H "Content-Type: application/json" \
       -H "Prefer: return=representation" -d '{"task":"nothing matches"}'
restarts=$(docker logs "$WK" 2>&1 | grep -c "uncaught exception")
if [ "$restarts" = 0 ]; then
    printf '    ok    no uncaught exception in the Worker log\n'; passed=$((passed + 1))
else
    printf '    FAIL  %s uncaught exceptions in the Worker log\n' "$restarts"; failed=$((failed + 1))
fi

echo "==> CORS and OPTIONS"
expect "OPTIONS says what a table allows" 200 '*' \
    -- -X OPTIONS "$URL/todos" -H "Origin: https://app.example"
allow=$(curl -s -D - -o /dev/null -X OPTIONS "$URL/todos" -H "Origin: https://app.example" | tr -d '\r')
grep -qi '^allow: .*POST' <<<"$allow" && grep -qi '^access-control-allow-origin: https://app.example' <<<"$allow" \
    && { printf '    ok    the preflight carries Allow and the origin\n'; passed=$((passed + 1)); } \
    || { printf '    FAIL  preflight headers\n%s\n' "$allow"; failed=$((failed + 1)); }
other=$(curl -s -D - -o /dev/null "$URL/todos" -H "Origin: https://elsewhere.example" | tr -d '\r')
grep -qi '^access-control-allow-origin' <<<"$other" \
    && { printf '    FAIL  an origin not on the list was allowed\n'; failed=$((failed + 1)); } \
    || { printf '    ok    an origin not on the list is not allowed\n'; passed=$((passed + 1)); }

echo "==> The schema cache"
docker exec "$PG" psql -q -U postgres -c "CREATE TABLE notes (id int PRIMARY KEY, body text); \
    GRANT SELECT ON notes TO web_anon; INSERT INTO notes VALUES (1, 'hello');" >/dev/null
expect "a table created after the cache was loaded is not known yet" 404 '*' \
    -- "$URL/notes"
expect "reload refuses the wrong secret" 401 '*' \
    -- -X POST "$URL/_/reload" -H "Authorization: Bearer wrong"
expect "reload with the secret" 204 '*' \
    -- -X POST "$URL/_/reload" -H "Authorization: Bearer $RELOAD"
expect "the new table answers after a reload" 200 '[{"id":1,"body":"hello"}]' \
    -- "$URL/notes"
docker exec "$PG" psql -q -U postgres -c "CREATE TABLE later (id int); GRANT SELECT ON later TO web_anon;" >/dev/null
expect "the cron trigger reloads it too" 200 '*' \
    -- "$URL/__scheduled?cron=*/5+*+*+*+*"
expect "the table the cron found answers" 200 '[]' \
    -- "$URL/later"

echo "==> A fresh isolate, with the schema cache from KV"
# workerd keeps KV under .wrangler/state in the mounted directory, so a
# restart is a new isolate -- no cache of its own -- reading what the reload
# above wrote. A table dropped since is still in that copy, which is how the
# check knows the cache came from KV and not from the catalog: PostgreSQL
# refuses the query (PGRST504), where a cache just read from the catalog would
# not know the table at all (PGRST205).
docker exec "$PG" psql -q -U postgres -c "DROP TABLE later;" >/dev/null
docker restart "$WK" >/dev/null
wait_healthy
expect "a filter on a typed column, with the cache read from KV" 200 '[{"id":1,"body":"hello"}]' \
    -- "$URL/notes?id=eq.1"
expect "the cache was read from KV: it still has the table dropped since" 404 '~PGRST504' \
    -- "$URL/later"

echo "==> What the writes left behind"
state=$(docker exec "$PG" psql -U postgres -Atc "SELECT string_agg(id || ':' || owner || ':' || task, ',' ORDER BY id) FROM todos")
if [ "$state" = "1:alice:alice public,2:alice:alice private,3:bob:bob private,4:alice:new" ]; then
    printf '    ok    only alice'"'"'s own write landed\n'
    passed=$((passed + 1))
else
    printf '    FAIL  table state\n          got  %s\n' "$state"
    failed=$((failed + 1))
fi

echo "==> With the database gone"
# A fresh isolate with nothing in KV and no PostgreSQL to load the schema cache
# from: the request fails before the engine runs. A browser must still be able
# to read that answer, and the body nobody read must not take the isolate down.
docker stop "$PG" >/dev/null
docker stop "$WK" >/dev/null
docker run --rm -v "$OUT":/app "$NODE_IMAGE" rm -rf /app/.wrangler/state
docker start "$WK" >/dev/null
wait_healthy
for i in 1 2 3; do
    early=$(curl -s -D - -o /dev/null -X POST "$URL/todos" -H "Origin: https://app.example" \
        -H "Content-Type: application/json" -d '{"owner":"x","task":"y"}' | tr -d '\r')
done
grep -q '^HTTP/[0-9.]* 5' <<<"$early" \
    && grep -qi '^access-control-allow-origin: https://app.example' <<<"$early" \
    && { printf '    ok    an error before the engine still carries the CORS headers\n'; passed=$((passed + 1)); } \
    || { printf '    FAIL  an error before the engine\n%s\n' "$early"; failed=$((failed + 1)); }
expect "the Worker still answers after them" 200 '~OK' \
    -- "$URL/_/health"
restarts=$(docker logs "$WK" 2>&1 | grep -c "uncaught exception")
if [ "$restarts" = 0 ]; then
    printf '    ok    still no uncaught exception in the Worker log\n'; passed=$((passed + 1))
else
    printf '    FAIL  %s uncaught exceptions in the Worker log\n' "$restarts"; failed=$((failed + 1))
fi

echo
echo "$passed passed, $failed failed"
if [ "$failed" -gt 0 ]; then
    echo "--- worker log"
    docker logs "$WK" 2>&1 | tail -40
    exit 1
fi
