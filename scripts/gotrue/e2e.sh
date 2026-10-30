#!/bin/bash
# Supabase Auth (GoTrue) in front of Postrust, end to end.
#
#   scripts/gotrue/e2e.sh jwks     # asymmetric keys, PGRST_JWT_JWKS_URL
#   scripts/gotrue/e2e.sh hs256    # GoTrue's legacy shared secret, PGRST_JWT_SECRET
#
# What a Supabase project's users do: sign up with GoTrue, get an access token,
# and call the database's API with it -- here Postrust's instead of PostgREST's.
# The check that matters is the last one: row-level security, written the way
# Supabase writes it (`auth.uid()` is `request.jwt.claims ->> 'sub'`), sees
# each user's own rows and nobody else's.
#
# `jwks` gives GoTrue an ES256 signing key (`GOTRUE_JWT_KEYS`, as a project
# migrated to asymmetric keys has), so its tokens carry a `kid` and Postrust
# verifies them with the public key it fetches from GoTrue's
# `/.well-known/jwks.json`. GoTrue's legacy HS256 secret is set as
# `PGRST_JWT_SECRET` beside it, and a service token signed with that is
# accepted too: the two sources combine.
#
# `hs256` is a project that has not migrated: GoTrue signs with the shared
# secret, and Postrust is given the same secret.
#
# Needs docker, curl, python3 and openssl. Starts PostgreSQL and GoTrue in
# containers on the host network, and Postrust from POSTRUST_BIN (default
# target/debug/postrust, built with `cargo build -p postrust-server`). The
# ports can be moved with PG_PORT, GOTRUE_PORT and POSTRUST_PORT.
#
# On Docker Desktop the "host network" is the VM's, not the machine's, so run
# the script itself from a container on it:
#
#   docker run --rm --network host -v /var/run/docker.sock:/var/run/docker.sock \
#       -v "$PWD:/src" docker:cli sh -c \
#       'apk add -q bash curl python3 openssl && POSTRUST_BIN=... /src/scripts/gotrue/e2e.sh jwks'
#
# where POSTRUST_BIN runs a Linux build on that network too.
set -euo pipefail

MODE=${1:-jwks}
case "$MODE" in jwks | hs256) ;; *) echo "usage: $0 jwks|hs256" >&2; exit 2 ;; esac

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
POSTRUST_BIN=${POSTRUST_BIN:-$ROOT/target/debug/postrust}
GOTRUE_IMAGE=${GOTRUE_IMAGE:-supabase/auth:v2.197.0}
PG_IMAGE=${PG_IMAGE:-postgres:16-alpine}
PG_PORT=${PG_PORT:-54329}
GOTRUE_PORT=${GOTRUE_PORT:-59999}
POSTRUST_PORT=${POSTRUST_PORT:-53099}
PG=gotrue-e2e-pg-$$
AUTH=gotrue-e2e-auth-$$
LOG=$(mktemp)
# GoTrue's legacy secret: HS256, at least 32 characters.
SECRET=super-secret-jwt-token-with-at-least-32-characters

POSTRUST_PID=
cleanup() {
    status=$?
    [ -n "$POSTRUST_PID" ] && kill "$POSTRUST_PID" 2>/dev/null || true
    if [ "$status" -ne 0 ]; then
        echo "--- postrust log"; cat "$LOG" || true
        echo "--- gotrue log"; docker logs "$AUTH" 2>&1 | tail -40 || true
    fi
    docker rm -f "$PG" "$AUTH" >/dev/null 2>&1 || true
    rm -f "$LOG"
    exit "$status"
}
trap cleanup EXIT

FAILED=0
ok() { echo "ok   - $1"; }
bad() { echo "FAIL - $1: $2"; FAILED=1; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "expected $2, got $3"; fi; }
json() { python3 -c "import sys,json; d=json.load(sys.stdin); print($1)"; }

wait_for() {
    for _ in $(seq 1 120); do
        curl -fs -o /dev/null "$1" && return 0
        sleep 0.5
    done
    echo "$1 never answered" >&2
    return 1
}

echo "# PostgreSQL"
docker run -d --name "$PG" --network host -e POSTGRES_PASSWORD=postgres \
    "$PG_IMAGE" -p "$PG_PORT" >/dev/null
for _ in $(seq 1 60); do
    docker exec "$PG" pg_isready -q -U postgres -p "$PG_PORT" && break
    sleep 0.5
done
psql_() { docker exec -i "$PG" psql -q -v ON_ERROR_STOP=1 -U postgres -p "$PG_PORT" -d postgres "$@"; }
# What a Supabase database has before GoTrue migrates it -- its own schema and
# role, and the API roles its migrations grant to -- and a table of notes
# under the policy every Supabase tutorial writes.
psql_ <<'SQL'
CREATE ROLE anon NOLOGIN;
CREATE ROLE authenticated NOLOGIN;
CREATE ROLE service_role NOLOGIN BYPASSRLS;
CREATE ROLE supabase_auth_admin LOGIN PASSWORD 'postgres' CREATEROLE NOINHERIT;
CREATE SCHEMA auth AUTHORIZATION supabase_auth_admin;
GRANT CREATE ON DATABASE postgres TO supabase_auth_admin;
ALTER ROLE supabase_auth_admin SET search_path = auth;

CREATE TABLE public.notes (
    id serial PRIMARY KEY,
    owner uuid NOT NULL,
    body text NOT NULL
);
ALTER TABLE public.notes ENABLE ROW LEVEL SECURITY;
CREATE POLICY own_notes ON public.notes FOR SELECT TO authenticated
    USING (owner = (current_setting('request.jwt.claims', true)::json ->> 'sub')::uuid);
GRANT USAGE ON SCHEMA public TO anon, authenticated, service_role;
GRANT SELECT ON public.notes TO authenticated, service_role;
SQL

echo "# GoTrue ($GOTRUE_IMAGE, $MODE)"
auth_env=(
    -e GOTRUE_API_HOST=127.0.0.1
    -e PORT="$GOTRUE_PORT"
    -e API_EXTERNAL_URL="http://127.0.0.1:$GOTRUE_PORT"
    -e GOTRUE_SITE_URL=http://127.0.0.1:3000
    -e GOTRUE_DB_DRIVER=postgres
    -e GOTRUE_DB_DATABASE_URL="postgres://supabase_auth_admin:postgres@127.0.0.1:$PG_PORT/postgres?sslmode=disable"
    -e GOTRUE_DB_NAMESPACE=auth
    -e GOTRUE_JWT_SECRET="$SECRET"
    -e GOTRUE_JWT_EXP=3600
    -e GOTRUE_JWT_AUD=authenticated
    # Logged as deprecated, and still needed: without it GoTrue mints
    # `"role": ""`, which names no database role. Supabase sets it too.
    -e GOTRUE_JWT_DEFAULT_GROUP_NAME=authenticated
    -e GOTRUE_JWT_ADMIN_ROLES=service_role
    -e GOTRUE_DISABLE_SIGNUP=false
    -e GOTRUE_EXTERNAL_EMAIL_ENABLED=true
    -e GOTRUE_MAILER_AUTOCONFIRM=true
    -e GOTRUE_RATE_LIMIT_EMAIL_SENT=1000
    -e GOTRUE_LOG_LEVEL=warn
)
if [ "$MODE" = jwks ]; then
    # A fresh P-256 key, as the private JWK GoTrue signs with. Its public half
    # is what /.well-known/jwks.json publishes.
    pem=$(openssl ecparam -name prime256v1 -genkey -noout 2>/dev/null)
    KEYS=$(printf '%s\n' "$pem" | openssl ec -text -noout 2>/dev/null | python3 -c '
import base64, json, re, sys
text = sys.stdin.read()
def hexblock(name):
    m = re.search(name + r":\s*\n((?:\s+[0-9a-f:]+\n)+)", text)
    return bytes.fromhex(re.sub(r"[\s:]", "", m.group(1)))
priv = hexblock("priv")[-32:].rjust(32, b"\0")
pub = hexblock("pub")
assert pub[0] == 4 and len(pub) == 65
b64 = lambda raw: base64.urlsafe_b64encode(raw).rstrip(b"=").decode()
print(json.dumps([{
    "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig",
    "key_ops": ["sign", "verify"], "kid": "e2e-es256",
    "x": b64(pub[1:33]), "y": b64(pub[33:]), "d": b64(priv),
}]))')
    auth_env+=(-e GOTRUE_JWT_KEYS="$KEYS")
fi
docker run -d --name "$AUTH" --network host "${auth_env[@]}" "$GOTRUE_IMAGE" >/dev/null
wait_for "http://127.0.0.1:$GOTRUE_PORT/health"

JWKS_URL="http://127.0.0.1:$GOTRUE_PORT/.well-known/jwks.json"
if [ "$MODE" = jwks ]; then
    check "GoTrue publishes the ES256 key" "e2e-es256" \
        "$(curl -fs "$JWKS_URL" | json "d['keys'][0]['kid']")"
fi

echo "# Postrust ($POSTRUST_BIN)"
postrust_env=(
    PGRST_DB_URI="postgres://postgres:postgres@127.0.0.1:$PG_PORT/postgres"
    PGRST_DB_SCHEMAS=public
    PGRST_DB_ANON_ROLE=anon
    PGRST_JWT_AUD=authenticated
    PGRST_JWT_SECRET="$SECRET"
    PGRST_SERVER_HOST=127.0.0.1
    PGRST_SERVER_PORT="$POSTRUST_PORT"
)
[ "$MODE" = jwks ] && postrust_env+=(PGRST_JWT_JWKS_URL="$JWKS_URL")
env "${postrust_env[@]}" "$POSTRUST_BIN" >"$LOG" 2>&1 &
POSTRUST_PID=$!
API="http://127.0.0.1:$POSTRUST_PORT"
wait_for "$API/_/ready"
if [ "$MODE" = jwks ]; then
    check "ready reports the keys loaded" "True" "$(curl -fs "$API/_/ready" | json "d['jwks']")"
fi

signup() {
    curl -fs -X POST "http://127.0.0.1:$GOTRUE_PORT/signup" \
        -H 'content-type: application/json' \
        -d "{\"email\":\"$1@example.com\",\"password\":\"correct-horse-battery\"}"
}
alice=$(signup alice)
bob=$(signup bob)
ALICE=$(json "d['access_token']" <<<"$alice")
BOB=$(json "d['access_token']" <<<"$bob")
ALICE_ID=$(json "d['user']['id']" <<<"$alice")
BOB_ID=$(json "d['user']['id']" <<<"$bob")

header() { python3 -c "
import base64, json, sys
part = sys.argv[1].split('.')[0]
print(json.loads(base64.urlsafe_b64decode(part + '=' * (-len(part) % 4)))[sys.argv[2]])" "$1" "$2"; }
if [ "$MODE" = jwks ]; then
    check "GoTrue signs with the ES256 key" "ES256 e2e-es256" \
        "$(header "$ALICE" alg) $(header "$ALICE" kid)"
else
    check "GoTrue signs with the shared secret" "HS256" "$(header "$ALICE" alg)"
fi

psql_ <<SQL
INSERT INTO public.notes (owner, body) VALUES
    ('$ALICE_ID', 'alice''s note'),
    ('$BOB_ID', 'bob''s note');
SQL

notes() { curl -s "$API/api/notes?select=body&order=id" ${1:+-H "authorization: Bearer $1"}; }
check "alice sees her own note" "[{\"body\":\"alice's note\"}]" "$(notes "$ALICE")"
check "bob sees his own note" "[{\"body\":\"bob's note\"}]" "$(notes "$BOB")"
# A signature with one of its bytes changed (not the last character, whose
# low bits are padding and may decode to the same signature).
TAMPERED=$(python3 -c "
import sys
t = sys.argv[1]; i = len(t) - 10
print(t[:i] + ('A' if t[i] != 'A' else 'B') + t[i + 1:])" "$ALICE")
check "a tampered token is refused" "PGRST301" "$(notes "$TAMPERED" | json "d['code']")"

if [ "$MODE" = jwks ]; then
    # A backend's service token, signed with the legacy secret: accepted
    # beside the provider's keys, and as the role it names.
    SERVICE=$(python3 - "$SECRET" <<'PY'
import base64, hashlib, hmac, json, sys, time
b64 = lambda raw: base64.urlsafe_b64encode(raw).rstrip(b"=").decode()
head = b64(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
body = b64(json.dumps({"role": "service_role", "aud": "authenticated",
                       "exp": int(time.time()) + 600}).encode())
mac = hmac.new(sys.argv[1].encode(), f"{head}.{body}".encode(), hashlib.sha256).digest()
print(f"{head}.{body}.{b64(mac)}")
PY
)
    check "a service token signed with the secret sees every note" "2" \
        "$(notes "$SERVICE" | json "len(d)")"
fi

exit "$FAILED"
