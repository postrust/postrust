#!/bin/bash
# The Lambda adapter, in the runtime it runs in.
#
# Builds `bootstrap` with cargo-lambda, starts it in AWS's own
# `provided:al2023` image -- which carries the Lambda runtime interface
# emulator -- beside a PostgreSQL with row-level security, and invokes it with
# the events a function URL sends. The function logs in as a least-privilege
# `authenticator`, as a deployment does: connected as a superuser, a function
# that never switches role still answers, and answers with every row.
#
# The adapter used to be a request path of its own. It bound no parameters and
# switched to no role, and nothing ran it, so nothing noticed. This is what
# runs it.
#
# Usage:
#   scripts/serverless/lambda.sh                     # build, then test
#   BOOTSTRAP=path/to/bootstrap scripts/serverless/lambda.sh   # test a build
#
# Needs docker, python3 and curl. Containers, the network and volumes are all
# named `postrust-sls-*` and removed on exit, except the two build caches.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PORT="${PORT:-59010}"
NET=postrust-sls-net
PG=postrust-sls-pg
FN=postrust-sls-lambda
SECRET=reallyreallyreallyreallyverysafe
CARGO_LAMBDA_IMAGE="${CARGO_LAMBDA_IMAGE:-ghcr.io/cargo-lambda/cargo-lambda:latest}"
LAMBDA_IMAGE="${LAMBDA_IMAGE:-public.ecr.aws/lambda/provided:al2023}"
OUT="$(mktemp -d)"

cleanup() {
    docker rm -f "$PG" "$FN" >/dev/null 2>&1
    docker network rm "$NET" >/dev/null 2>&1
    rm -rf "$OUT"
}
trap cleanup EXIT
cleanup
OUT="$(mktemp -d)"

if [ -n "${BOOTSTRAP:-}" ]; then
    cp "$BOOTSTRAP" "$OUT/bootstrap"
else
    echo "==> Building bootstrap"
    docker run --rm -v "$ROOT":/src:ro -w /src \
        -v postrust-sls-cargo:/usr/local/cargo/registry \
        -v postrust-sls-target:/target -e CARGO_TARGET_DIR=/target \
        "$CARGO_LAMBDA_IMAGE" cargo lambda build --release -p postrust-lambda \
        || { echo "build failed"; exit 1; }
    docker run --rm -v postrust-sls-target:/target -v "$OUT":/out alpine \
        cp /target/lambda/postrust-lambda/bootstrap /out/bootstrap || exit 1
fi
chmod 755 "$OUT/bootstrap"
echo "    bootstrap: $(du -h "$OUT/bootstrap" | cut -f1)"

echo "==> Starting PostgreSQL and the function"
docker network create "$NET" >/dev/null
docker run -d --name "$PG" --network "$NET" -e POSTGRES_PASSWORD=postgres postgres:16-alpine >/dev/null
for _ in $(seq 60); do docker exec "$PG" pg_isready -U postgres -q 2>/dev/null && break; sleep 1; done
sleep 1
docker exec -i "$PG" psql -q -v ON_ERROR_STOP=1 -U postgres < "$HERE/fixture.sql" || exit 1

docker run -d --name "$FN" --network "$NET" -p "127.0.0.1:$PORT:8080" \
    -v "$OUT":/var/runtime:ro \
    -e DATABASE_URL="postgres://authenticator:authenticator@$PG/postgres" \
    -e PGRST_DB_SCHEMAS=public -e PGRST_DB_ANON_ROLE=web_anon \
    -e PGRST_JWT_SECRET="$SECRET" -e PGRST_LOG_LEVEL=info \
    "$LAMBDA_IMAGE" bootstrap >/dev/null
URL="http://127.0.0.1:$PORT/2015-03-31/functions/function/invocations"
for _ in $(seq 30); do curl -s -o /dev/null "http://127.0.0.1:$PORT/" && break; sleep 1; done

ALICE=$(python3 "$HERE/jwt.py" "$SECRET" web_user alice)
BOB=$(python3 "$HERE/jwt.py" "$SECRET" web_user bob)
NOBODY=$(python3 "$HERE/jwt.py" "$SECRET" no_such_role mallory)

passed=0
failed=0
# expect LABEL STATUS BODY -- invoke.py arguments...
#   BODY is JSON compared as a value, `~text` for a substring, or `*` for any.
expect() {
    local label="$1" status="$2" body="$3"
    shift 4
    local out got_status got_body
    out=$(python3 "$HERE/invoke.py" "$URL" "$@" 2>&1)
    got_status=$(sed -n 1p <<<"$out")
    got_body=$(sed -n 2p <<<"$out")
    if [ "$got_status" = "$status" ] && python3 - "$body" "$got_body" <<'PY'
import json, sys
want, got = sys.argv[1], sys.argv[2]
if want == "*":
    sys.exit(0)
if want.startswith("~"):
    sys.exit(0 if want[1:] in got else 1)
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

echo "==> REST"
expect "anon reads only public rows" 200 '[{"id":1,"task":"alice public"}]' \
    -- GET /api/todos "select=id,task&order=id"
expect "a filter binds its parameter" 200 '[{"id":1}]' \
    -- GET /api/todos "select=id&id=eq.1"
expect "alice reads her own rows" 200 '[{"id":1},{"id":2}]' \
    -- GET /api/todos "select=id&order=id" --token "$ALICE"
expect "bob reads his own rows" 200 '[{"id":3}]' \
    -- GET /api/todos "select=id&order=id" --token "$BOB"
expect "a role that does not exist is refused" 401 '*' \
    -- GET /api/todos "" --token "$NOBODY"
expect "anon cannot write" 401 '*' \
    -- POST /api/todos "" --body '{"owner":"mallory","task":"x"}'
expect "alice writes her own row" 201 '[{"owner":"alice","task":"new"}]' \
    -- POST /api/todos "select=owner,task" --token "$ALICE" \
       --body '{"owner":"alice","task":"new"}' --header "prefer:return=representation"
expect "alice cannot write as bob" 401 '*' \
    -- POST /api/todos "" --token "$ALICE" --body '{"owner":"bob","task":"x"}'
expect "alice cannot update bob's row" 200 '[]' \
    -- PATCH /api/todos "id=eq.3" --token "$ALICE" --body '{"task":"hijacked"}' \
       --header "prefer:return=representation"
expect "a function runs as the token's role, with its claims" 200 '[{"whoami":"web_user:alice"}]' \
    -- GET /api/rpc/whoami "" --token "$ALICE"

echo "==> GraphQL"
expect "anon reads only public rows" 200 '{"data":{"todos":[{"id":1}]}}' \
    -- POST /v1/graphql "" --body '{"query":"{ todos(order_by: {id: asc}) { id } }"}'
expect "alice reads her own rows, the new one too" 200 '{"data":{"todos":[{"id":1},{"id":2},{"id":4}]}}' \
    -- POST /v1/graphql "" --token "$ALICE" \
       --body '{"query":"{ todos(order_by: {id: asc}) { id } }"}'
expect "no Subscription root on a host without WebSockets" 200 \
    '{"data":{"__schema":{"subscriptionType":null}}}' \
    -- POST /v1/graphql "" --body '{"query":"{ __schema { subscriptionType { name } } }"}'

echo "==> What the writes left behind"
state=$(docker exec "$PG" psql -U postgres -Atc "SELECT string_agg(id || ':' || owner || ':' || task, ',' ORDER BY id) FROM todos")
if [ "$state" = "1:alice:alice public,2:alice:alice private,3:bob:bob private,4:alice:new" ]; then
    printf '    ok    only alice'"'"'s own write landed\n'
    passed=$((passed + 1))
else
    printf '    FAIL  table state\n          got  %s\n' "$state"
    failed=$((failed + 1))
fi

echo "==> Health"
expect "healthz" 200 '~OK' -- GET /healthz

echo
echo "$passed passed, $failed failed"
if [ "$failed" -gt 0 ]; then
    echo "--- function log"
    docker logs "$FN" 2>&1 | tail -30
    exit 1
fi
