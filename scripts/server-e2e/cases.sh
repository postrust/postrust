# The checks themselves. Sourced by e2e.sh, which owns the database, the
# helpers and the counters. Not executable on its own.

JSON() { python3 -c "import sys,json;d=json.load(sys.stdin);print($1)" 2>/dev/null; }

# ---------------------------------------------------------------------------
say "Admin port, CORS, body size and log level"
# ---------------------------------------------------------------------------
P=$(port 0); A=$(port 1)
start_server PGRST_SERVER_PORT="$P" PGRST_ADMIN_SERVER_PORT="$A" \
    PGRST_SERVER_CORS_ORIGINS="https://allowed.example" \
    PGRST_MAX_BODY_SIZE=256 PGRST_LOG_LEVEL=warn
wait_http "http://127.0.0.1:$P/healthz" || exit 1

for path in live health ready; do
    check "admin /$path answers" "200" \
        "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$A/$path")"
done
contains "admin /ready reports the database" '"database":true' "$(curl -s "http://127.0.0.1:$A/ready")"
# The reason for a separate port is that it can be exposed where the API is
# not, so the API must not be reachable on it.
check "the API is not on the admin port" "404" \
    "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$A/api/users")"

allowed=$(curl -s -D- -o /dev/null -H 'Origin: https://allowed.example' \
    "http://127.0.0.1:$P/api/users?limit=1" | tr -d '\r' | grep -i '^access-control-allow-origin:')
check "a configured origin is echoed" "access-control-allow-origin: https://allowed.example" "$allowed"
denied=$(curl -s -D- -o /dev/null -H 'Origin: https://evil.example' \
    "http://127.0.0.1:$P/api/users?limit=1" | tr -d '\r' | grep -ci '^access-control-allow-origin:')
check "any other origin gets no CORS header" "0" "$denied"

# Asserting only on a 4xx would pass with the limit ignored entirely: an
# oversized insert is refused for other reasons too. The body has to say size.
big=$(python3 -c "print('{\"name\":\"' + 'x' * 4096 + '\"}')")
contains "an oversized body is refused for its size" "length limit exceeded" \
    "$(curl -s -X POST -H 'content-type: application/json' -d "$big" "http://127.0.0.1:$P/api/products")"
under=$(curl -s -X POST -H 'content-type: application/json' -d '{"name":"ok"}' "http://127.0.0.1:$P/api/products")
case "$under" in
    *"length limit exceeded"*) bad "a body inside the limit is read" "refused for its size" ;;
    *) ok "a body inside the limit is read" ;;
esac

if grep -qE '\bINFO\b' "$LOG"; then
    bad "log_level=warn suppresses INFO" "INFO lines present"
else
    ok "log_level=warn suppresses INFO"
fi
stop_server

# ---------------------------------------------------------------------------
say "Settings applied to the request's transaction"
# ---------------------------------------------------------------------------
P=$(port 2)
start_server PGRST_SERVER_PORT="$P" \
    PGRST_APP_SETTINGS_TENANT=acme \
    PGRST_DB_TX_ISOLATION="repeatable read" \
    PGRST_ROLE_SETTINGS='{"web_anon":{"statement_timeout":7000}}' \
    PGRST_DB_PRE_REQUEST=public.e2e_allow
wait_http "http://127.0.0.1:$P/healthz" || exit 1

seen=$(curl -s -X POST "http://127.0.0.1:$P/api/rpc/e2e_settings")
contains "app.settings.<name> reaches the database" '"tenant":"acme"'            "$seen"
contains "db_tx_isolation is applied"               '"isolation":"repeatable read"' "$seen"
contains "role_settings statement_timeout applied"  '"timeout":"7s"'             "$seen"
contains "the request runs as the anonymous role"   '"role":"web_anon"'          "$seen"
check "a pre-request hook that succeeds lets the request through" "200" \
    "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/api/users?limit=1")"
stop_server

P=$(port 3)
start_server PGRST_SERVER_PORT="$P" PGRST_DB_PRE_REQUEST=public.e2e_deny
wait_http "http://127.0.0.1:$P/healthz" || exit 1
code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/api/users?limit=1")
[ "$code" != "200" ] && ok "a pre-request hook that raises aborts the request" \
    || bad "a pre-request hook that raises aborts the request" "answered 200"
stop_server

# The defaults have to be checked too, or the tests above would pass against a
# server that applied those settings unconditionally.
P=$(port 4)
start_server PGRST_SERVER_PORT="$P"
wait_http "http://127.0.0.1:$P/healthz" || exit 1
seen=$(curl -s -X POST "http://127.0.0.1:$P/api/rpc/e2e_settings")
contains "the default isolation level is read committed" '"isolation":"read committed"' "$seen"
contains "no app setting is set when none is configured" '"tenant":null' "$seen"
stop_server

# ---------------------------------------------------------------------------
say "Unix domain socket"
# ---------------------------------------------------------------------------
rm -f "$SOCK"
start_server PGRST_SERVER_UNIX_SOCKET="$SOCK"
for _ in $(seq 1 80); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] && ok "the socket is created" || bad "the socket is created" "nothing at $SOCK"
check "requests are served over it" "200" \
    "$(curl -s -o /dev/null -w '%{http_code}' --unix-socket "$SOCK" http://localhost/api/users?limit=1)"
if curl -s -o /dev/null -m 2 "http://127.0.0.1:3000/api/users" 2>/dev/null; then
    bad "no TCP listener when a socket is configured" "something answered on :3000"
else
    ok "no TCP listener when a socket is configured"
fi
stop_server

# A socket file outlives the process that made it, and binding over one fails
# with EADDRINUSE against a server that is not running.
[ -S "$SOCK" ] || ok "(the socket was already cleaned up)"
start_server PGRST_SERVER_UNIX_SOCKET="$SOCK"
for _ in $(seq 1 80); do
    [ -S "$SOCK" ] && curl -sf -m 2 --unix-socket "$SOCK" http://localhost/healthz >/dev/null 2>&1 && break
    sleep 0.25
done
check "it restarts over a stale socket" "200" \
    "$(curl -s -o /dev/null -w '%{http_code}' --unix-socket "$SOCK" http://localhost/healthz)"
contains "and says it removed the stale one" "Removed stale socket" "$(cat "$LOG")"
stop_server
rm -f "$SOCK"

# Anything at that path that is not a socket is a configuration mistake, and
# deleting it would be the wrong way to find out.
echo "do not delete me" > "$NOTSOCK"
out=$(env PGRST_SERVER_UNIX_SOCKET="$NOTSOCK" "$BIN" 2>&1 | tail -3)
contains "a path that is not a socket is refused" "is not a socket" "$out"
[ "$(cat "$NOTSOCK")" = "do not delete me" ] \
    && ok "and the file is left alone" || bad "and the file is left alone" "it was touched"
rm -f "$NOTSOCK"

# ---------------------------------------------------------------------------
say "Schema cache reload on NOTIFY"
# ---------------------------------------------------------------------------
P=$(port 5)
PSQL "DROP TABLE IF EXISTS public.e2e_late CASCADE" >/dev/null
start_server PGRST_SERVER_PORT="$P" PGRST_DB_CHANNEL_ENABLED=true PGRST_DB_CHANNEL=pgrst
wait_http "http://127.0.0.1:$P/healthz" || exit 1
contains "the reloader is listening" "Listening on channel pgrst" "$(cat "$LOG")"

check "a table that does not exist is 404" "404" \
    "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/api/e2e_late")"
PSQL "CREATE TABLE public.e2e_late(id int primary key); GRANT SELECT ON public.e2e_late TO web_anon" >/dev/null
# The control that makes the next assertion mean something: creating the table
# must *not* be enough on its own, or the reload is not what we measured.
check "still 404 before the NOTIFY" "404" \
    "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/api/e2e_late")"
PSQL "NOTIFY pgrst, 'reload schema'" >/dev/null
for _ in $(seq 1 40); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/api/e2e_late")" = "200" ] && break
    sleep 0.25
done
check "reachable after the NOTIFY, with no restart" "200" \
    "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/api/e2e_late")"
contains "and the reload is logged" "Schema cache reloaded" "$(cat "$LOG")"
stop_server
PSQL "DROP TABLE IF EXISTS public.e2e_late CASCADE" >/dev/null

P=$(port 6)
start_server PGRST_SERVER_PORT="$P"
wait_http "http://127.0.0.1:$P/healthz" || exit 1
grep -q "Listening on channel" "$LOG" \
    && bad "no reloader unless asked for" "it started anyway" \
    || ok "no reloader unless asked for"
stop_server

# ---------------------------------------------------------------------------
say "Connection pool"
# ---------------------------------------------------------------------------
P=$(port 7)
start_server PGRST_SERVER_PORT="$P" PGRST_DB_POOL_SIZE=3
wait_http "http://127.0.0.1:$P/healthz" || exit 1
# Wait on these PIDs specifically, not a bare `wait`: the server is a
# background job of this same shell, so `wait` alone never returns.
pids=""
for _ in $(seq 1 12); do
    curl -s -o /dev/null "http://127.0.0.1:$P/api/users?limit=1" &
    pids="$pids $!"
done
for pid in $pids; do wait "$pid"; done
n=$(PSQL "SELECT count(*) FROM pg_stat_activity
          WHERE datname = current_database() AND backend_type = 'client backend'
            AND application_name NOT LIKE 'psql%'")
# Not an equality: the GraphQL subscription broker holds connections of its own
# where that feature is compiled in. The point is that 12 concurrent requests
# do not open 12 backends.
[ "${n:-99}" -le 8 ] && ok "db_pool_size bounds the connections (saw $n for 12 concurrent requests)" \
    || bad "db_pool_size bounds the connections" "saw $n backends"
stop_server

# ---------------------------------------------------------------------------
say "OpenAPI"
# ---------------------------------------------------------------------------
P=$(port 8)
start_server PGRST_SERVER_PORT="$P"
wait_http "http://127.0.0.1:$P/healthz" || exit 1
spec=$(curl -s "http://127.0.0.1:$P/admin/openapi.json")
check "the specification is served" "200" \
    "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/admin/openapi.json")"
contains "a table in the database has a path" "/api/products" "$(echo "$spec" | JSON "list(d['paths'])")"
contains "the path names its schema" "public.products" "$(echo "$spec" | JSON "d['paths']['/api/products'].get('summary')")"
stop_server

P=$(port 9)
start_server PGRST_SERVER_PORT="$P" PGRST_OPENAPI_MODE=disabled
wait_http "http://127.0.0.1:$P/healthz" || exit 1
# First prove the admin surface is mounted at all, so a build without
# `admin-ui` cannot masquerade as a disabled specification.
check "the admin surface is mounted" "200" \
    "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/admin/swagger")"
check "disabled is a 404, not an empty document" "404" \
    "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$P/admin/openapi.json")"
stop_server

P=$(port 10)
start_server PGRST_SERVER_PORT="$P" PGRST_OPENAPI_SERVER_PROXY_URI=https://api.example.com
wait_http "http://127.0.0.1:$P/healthz" || exit 1
check "openapi_server_proxy_uri is advertised" "https://api.example.com" \
    "$(curl -s "http://127.0.0.1:$P/admin/openapi.json" | JSON "d['servers'][0]['url']")"
stop_server

# A URL carries no schema, so two exposed schemas holding the same table name
# want the same path. One of them used to overwrite the other, and which one
# depended on hash iteration order.
P=$(port 11)
start_server PGRST_SERVER_PORT="$P" PGRST_DB_SCHEMAS=public,api
wait_http "http://127.0.0.1:$P/healthz" || exit 1
spec=$(curl -s "http://127.0.0.1:$P/admin/openapi.json")
check "one path for a name two schemas share" "1" \
    "$(echo "$spec" | JSON "sum(1 for k in d['paths'] if k == '/api/users')")"
check "the default schema owns the bare path" "public.users" \
    "$(echo "$spec" | JSON "d['paths']['/api/users']['summary']")"
desc=$(echo "$spec" | JSON "d['paths']['/api/users'].get('description', '')")
contains "the other schema is recorded, not dropped" "api" "$desc"
contains "and Accept-Profile is named as the way to reach it" "Accept-Profile" "$desc"
stop_server

# ---------------------------------------------------------------------------
say "JWT"
# ---------------------------------------------------------------------------
PLAIN='a-secret-key-of-at-least-32-characters!!'
B64=$(python3 -c "import base64;print(base64.b64encode(b'$PLAIN').decode())")
role_of() { # token port
    curl -s -H "Authorization: Bearer $1" -X POST "http://127.0.0.1:$2/api/rpc/e2e_settings" \
        | JSON "d[0]['e2e_settings']['role']"
}

# The configured secret is base64 text whose *decoded bytes* are the key.
tok=$(python3 "$HERE/mint_jwt.py" --b64-secret "$B64" '{"role":"web_user"}')
P=$(port 12)
start_server PGRST_SERVER_PORT="$P" PGRST_JWT_SECRET="$B64" PGRST_JWT_SECRET_IS_BASE64=true
wait_http "http://127.0.0.1:$P/healthz" || exit 1
check "a token signed with the decoded key is accepted" "web_user" "$(role_of "$tok" "$P")"
stop_server

# Same secret and same token with the flag off: the key is then the base64
# text itself, so the signature must not verify. Without this the test above
# would pass on a server that ignored the flag.
P=$(port 13)
start_server PGRST_SERVER_PORT="$P" PGRST_JWT_SECRET="$B64"
wait_http "http://127.0.0.1:$P/healthz" || exit 1
check "the same token is rejected with the flag off" "401" \
    "$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $tok" "http://127.0.0.1:$P/api/users?limit=1")"
stop_server

nested=$(python3 "$HERE/mint_jwt.py" --secret "$PLAIN" '{"user":{"role":"web_user"}}')
P=$(port 14)
start_server PGRST_SERVER_PORT="$P" PGRST_JWT_SECRET="$PLAIN" PGRST_JWT_ROLE_CLAIM_KEY="user.role"
wait_http "http://127.0.0.1:$P/healthz" || exit 1
check "a nested role claim is resolved" "web_user" "$(role_of "$nested" "$P")"
stop_server

P=$(port 15)
start_server PGRST_SERVER_PORT="$P" PGRST_JWT_SECRET="$PLAIN"
wait_http "http://127.0.0.1:$P/healthz" || exit 1
check "and is not found without the setting" "web_anon" "$(role_of "$nested" "$P")"
stop_server

# The cache must never outlive the token. An `exp` that has passed has to be
# refused even while a cached validation of it would still be in date.
P=$(port 16)
start_server PGRST_SERVER_PORT="$P" PGRST_JWT_SECRET="$PLAIN" \
    PGRST_JWT_CACHE_ENABLED=true PGRST_JWT_CACHE_MAX_LIFETIME=3600
wait_http "http://127.0.0.1:$P/healthz" || exit 1
short=$(python3 "$HERE/mint_jwt.py" --secret "$PLAIN" --expires-in 2 '{"role":"web_user"}')
check "a token is accepted while it is valid" "web_user" "$(role_of "$short" "$P")"
sleep 3
check "and refused once expired, despite an hour-long cache" "401" \
    "$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $short" "http://127.0.0.1:$P/api/users?limit=1")"
long=$(python3 "$HERE/mint_jwt.py" --secret "$PLAIN" '{"role":"web_user"}')
check "a valid token still works"        "web_user" "$(role_of "$long" "$P")"
check "and again, served from the cache" "web_user" "$(role_of "$long" "$P")"
stop_server
