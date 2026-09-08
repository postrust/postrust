#!/bin/bash
# End-to-end checks against a real server process and a real PostgreSQL.
#
# These cover the wiring in `crates/postrust-server/src/main.rs`, which no
# other test reaches. `cargo test` builds a `Router` in-process and calls it;
# it never starts the binary, so the listener setup, the CORS layer, the admin
# port, the schema reloader and the pool options are all invisible to it. Every
# one of those was a configuration option that existed and did nothing, and a
# router-level test would not have noticed.
#
# What is checked here, and nowhere else:
#
#   * `admin_server_port`, and that the API is *not* on it
#   * `server_cors_origins`, allowed and denied
#   * `max_body_size` over real HTTP, refused for its size and not for a 4xx
#     the request would have earned anyway
#   * `log_level`
#   * `app_settings`, `db_tx_isolation`, `role_settings`, `db_pre_request`, read
#     back from inside the request's own transaction
#   * `server_unix_socket`, including a stale socket and a path that is not one
#   * `db_channel_enabled`, with a negative control either side of the NOTIFY
#   * `db_pool_size`, counted in `pg_stat_activity`
#   * `openapi_mode`, `openapi_server_proxy_uri`, and two schemas sharing a
#     table name
#   * `jwt_secret_is_base64`, `jwt_role_claim_key` including a nested claim,
#     and that the token cache never outlives a token's own `exp`
#
# Usage:
#   scripts/server-e2e/e2e.sh            # brings up its own PostgreSQL
#   E2E_KEEP_DB=1 scripts/server-e2e/e2e.sh
#   E2E_DATABASE_URL=postgres://... scripts/server-e2e/e2e.sh
#
# Requirements: docker (unless E2E_DATABASE_URL points at your own), cargo,
# curl, python3.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
WORK="${E2E_WORK:-$HERE/.work}"
mkdir -p "$WORK"

DB=postrust-e2e-db
DB_PORT="${E2E_DB_PORT:-55440}"
DB_IMAGE="${E2E_DB_IMAGE:-postgres:16-alpine}"
BASE_PORT="${E2E_BASE_PORT:-3900}"

# A Unix socket path has to fit in `sun_path`, which is 108 bytes including the
# terminator -- so it cannot live under $WORK, whose path depends on where the
# repository was cloned. /tmp keeps it short wherever this runs.
SOCK="${E2E_SOCKET:-/tmp/postrust-e2e.sock}"
NOTSOCK="$WORK/not-a-socket.txt"

LOG="$WORK/server.log"
BIN="$ROOT/target/debug/postrust"

say()  { printf '\n==> %s\n' "$*"; }
pass=0; fail=0
ok()   { printf '  \033[32mPASS\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad()  { printf '  \033[31mFAIL\033[0m %s\n         %s\n' "$1" "$2"; fail=$((fail + 1)); }
check() { [ "$2" = "$3" ] && ok "$1" || bad "$1" "expected [$2], got [$3]"; }
contains() { case "$3" in *"$2"*) ok "$1" ;; *) bad "$1" "expected to contain [$2], got [$3]" ;; esac; }

OWN_DB=0
cleanup() {
    stop_server
    rm -f "$SOCK" "$NOTSOCK"
    if [ "$OWN_DB" = "1" ] && [ "${E2E_KEEP_DB:-0}" != "1" ]; then
        docker rm -f "$DB" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Server lifecycle
# ---------------------------------------------------------------------------

SRV_PID=""
start_server() { # VAR=VAL...
    : > "$LOG"
    env "$@" "$BIN" >>"$LOG" 2>&1 &
    SRV_PID=$!
}
stop_server() {
    [ -n "${SRV_PID:-}" ] || return 0
    kill "$SRV_PID" 2>/dev/null
    wait "$SRV_PID" 2>/dev/null
    SRV_PID=""
}
# Wait for a URL, or give up and show why.
wait_http() {
    for _ in $(seq 1 80); do
        curl -sf -m 2 "$1" >/dev/null 2>&1 && return 0
        kill -0 "$SRV_PID" 2>/dev/null || break
        sleep 0.25
    done
    bad "server start" "never answered $1"
    tail -15 "$LOG" | sed 's/^/         /'
    return 1
}
port() { echo $((BASE_PORT + $1)); }

# ---------------------------------------------------------------------------
# Database
# ---------------------------------------------------------------------------

if [ -n "${E2E_DATABASE_URL:-}" ]; then
    export DATABASE_URL="$E2E_DATABASE_URL"
    PSQL() { psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -tAc "$1"; }
    PSQL_FILE() { psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -q -f -; }
else
    command -v docker >/dev/null || { echo "docker is required (or set E2E_DATABASE_URL)"; exit 1; }
    say "Starting PostgreSQL ($DB_IMAGE) on port $DB_PORT"
    docker rm -f "$DB" >/dev/null 2>&1 || true
    docker run -d --name "$DB" -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres \
        -e POSTGRES_DB=postrust_e2e -p "$DB_PORT:5432" "$DB_IMAGE" >/dev/null
    OWN_DB=1
    for _ in $(seq 1 60); do
        docker exec "$DB" pg_isready -h 127.0.0.1 -U postgres -d postrust_e2e >/dev/null 2>&1 && break
        sleep 1
    done
    export DATABASE_URL="postgres://postgres:postgres@localhost:$DB_PORT/postrust_e2e"
    PSQL() { docker exec -i "$DB" psql -U postgres -d postrust_e2e -v ON_ERROR_STOP=1 -tAc "$1"; }
    PSQL_FILE() { docker exec -i "$DB" psql -U postgres -d postrust_e2e -v ON_ERROR_STOP=1 -q -f -; }
fi

say "Loading fixtures"
PSQL_FILE < "$ROOT/scripts/init-db.sql" >/dev/null
PSQL_FILE < "$ROOT/scripts/test-fixtures.sql" >/dev/null

# Helpers this suite needs, kept here rather than in the shared fixtures: they
# exist to read settings back out of the request's own transaction, which is
# not something the other suites ask for.
PSQL_FILE <<'SQL' >/dev/null
CREATE OR REPLACE FUNCTION public.e2e_settings() RETURNS json LANGUAGE sql STABLE AS $$
  SELECT json_build_object(
    'tenant',    current_setting('app.settings.tenant', true),
    'isolation', current_setting('transaction_isolation', true),
    'timeout',   current_setting('statement_timeout', true),
    'role',      current_user
  );
$$;
CREATE OR REPLACE FUNCTION public.e2e_deny() RETURNS void LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'refused by pre-request hook'; END;
$$;
CREATE OR REPLACE FUNCTION public.e2e_allow() RETURNS void LANGUAGE plpgsql AS $$
BEGIN PERFORM 1; END;
$$;
GRANT EXECUTE ON FUNCTION public.e2e_settings() TO web_anon;
GRANT EXECUTE ON FUNCTION public.e2e_deny() TO web_anon;
GRANT EXECUTE ON FUNCTION public.e2e_allow() TO web_anon;
SQL

export PGRST_DB_ANON_ROLE=web_anon
export PGRST_DB_SCHEMAS=public

say "Building the server (admin-ui)"
# admin-ui, because the OpenAPI section needs the admin surface mounted. Built
# here rather than assumed: a binary left over from `cargo test --workspace` has
# default features, and every OpenAPI check would then 404 -- including the one
# asserting that `openapi_mode=disabled` 404s, which would pass for entirely
# the wrong reason.
(cd "$ROOT" && cargo build -q -p postrust-server --features admin-ui) || exit 1

. "$HERE/cases.sh"

printf '\n'
if [ "$fail" -ne 0 ]; then
    printf '\033[31mFAILED\033[0m  %s passed, %s failed\n' "$pass" "$fail"
    exit 1
fi
printf '\033[32mok\033[0m  %s checks passed\n' "$pass"
