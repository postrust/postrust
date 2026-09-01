#!/usr/bin/env bash
#
# End-to-end ACME issuance against Pebble, Let's Encrypt's test CA.
#
#   scripts/acme/run.sh
#
# Pebble deliberately misbehaves -- it rejects a share of nonces, varies
# challenge ordering, and returns states a happy-path client does not expect.
# That is why the test is worth running: it breaks clients that only handle the
# good case.
#
# What has to be true for an ACME test to mean anything, and how it is arranged:
#
#   1. A CA the client will talk to.        pebble, on the acmenet subnet.
#   2. The CA must trust nothing of ours,   pebble's root is fetched from its
#      but we must trust its directory.     management API and passed to the
#                                           client explicitly.
#   3. The test domain must resolve to      challtestsrv answers every query
#      something the CA can reach.          with one address, set below.
#   4. The CA must reach our challenge      pebble validates http-01 on port
#      endpoint at that address.            5002 (not 80). A socat container
#                                           sits at that address and forwards
#                                           to the test's listener on the host.
#
# Everything created is removed on exit.

set -euo pipefail

cd "$(dirname "$0")/../.."

WORK="scripts/acme/.work"
PROXY_ADDR="10.30.50.4"          # left free for this by docker-compose.yml
PEBBLE_HTTP_PORT=5002            # pebble's http-01 validation port
CHALLENGE_PORT="${CHALLENGE_PORT:-15080}"
PG_PORT="${PG_PORT:-55436}"
TEST_DOMAIN="${TEST_DOMAIN:-acme-test.example}"

mkdir -p "$WORK"

cleanup() {
  echo "--- cleaning up"
  docker rm -f postrust-acme-socat >/dev/null 2>&1 || true
  docker rm -f postrust-acme-pg >/dev/null 2>&1 || true
  docker compose --profile acme down >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "--- postgres"
docker rm -f postrust-acme-pg >/dev/null 2>&1 || true
docker run -d --name postrust-acme-pg \
  -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=postrust_test \
  -p "${PG_PORT}:5432" postgres:16 >/dev/null
until docker exec postrust-acme-pg psql -U postgres -d postrust_test -tAc 'select 1' >/dev/null 2>&1; do
  sleep 0.5
done

echo "--- pebble + challtestsrv"
docker compose --profile acme up -d pebble challtestsrv >/dev/null

echo "--- waiting for pebble's directory"
# Its own certificate is self-signed, hence -k here; the client under test is
# given the real root instead.
until curl -fsSk https://127.0.0.1:14000/dir >/dev/null 2>&1; do sleep 0.5; done

echo "--- fetching pebble's root"
# Not /roots/0 from the management API: that is the root pebble *issues* from,
# which is not what signed pebble's own directory certificate. The directory is
# served under test/certs/localhost/cert.pem, signed by the minica root baked
# into the image -- so that is the one the client has to trust. Using /roots/0
# fails with "unable to get local issuer certificate", which surfaces from the
# client as an unhelpful "client error (Connect)".
docker cp postrust-pebble:/test/certs/pebble.minica.pem "$WORK/pebble-root.pem"
test -s "$WORK/pebble-root.pem" || { echo "error: empty root certificate" >&2; exit 1; }

echo "--- pointing challtestsrv at ${PROXY_ADDR}"
curl -fsS -X POST http://127.0.0.1:8055/set-default-ipv4 \
  -d "{\"ip\":\"${PROXY_ADDR}\"}" >/dev/null

echo "--- forwarding ${PROXY_ADDR}:${PEBBLE_HTTP_PORT} to the host"
# The challenge listener runs on the host, so something on acmenet has to carry
# pebble's validation request to it. socat is enough and avoids building the
# proxy into an image for a test.
docker rm -f postrust-acme-socat >/dev/null 2>&1 || true
docker run -d --name postrust-acme-socat \
  --network "$(docker compose --profile acme config --format json \
      | python3 -c 'import sys,json; print(json.load(sys.stdin)["networks"]["acmenet"].get("name","acmenet"))')" \
  --ip "$PROXY_ADDR" \
  alpine/socat \
  "TCP-LISTEN:${PEBBLE_HTTP_PORT},fork,reuseaddr" \
  "TCP:host.docker.internal:${CHALLENGE_PORT}" >/dev/null

echo "--- running the test"
DATABASE_URL="postgres://postgres:postgres@127.0.0.1:${PG_PORT}/postrust_test" \
ACME_DIRECTORY="https://127.0.0.1:14000/dir" \
ACME_ROOT_PEM="$(pwd)/$WORK/pebble-root.pem" \
ACME_CHALLENGE_PORT="$CHALLENGE_PORT" \
ACME_TEST_DOMAIN="$TEST_DOMAIN" \
  cargo test -p postrust-proxy --test acme_issuance -- --ignored --test-threads=1 --nocapture \
  2>&1 | tee "$WORK/test.log"

# The tests skip themselves when their environment is absent, so that CI's
# workspace-wide `-- --ignored` pass does not fail for want of a CA. That makes
# a silent skip possible here, where a run was the whole point.
if grep -q "SKIPPED" "$WORK/test.log"; then
  echo "error: the tests skipped themselves; the harness did not set up correctly" >&2
  exit 1
fi
if ! grep -q "^test result: ok. 2 passed" "$WORK/test.log"; then
  echo "error: expected 2 passing tests; see $WORK/test.log" >&2
  exit 1
fi
echo "--- 2 tests passed against pebble"
