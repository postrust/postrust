#!/usr/bin/env bash
# Brings the example up and checks, through Apollo Router, what the blog post
# and README say it does. Leaves it running with KEEP=1.
#
#   examples/federation/check.sh
#   POSTRUST_IMAGE=postrust:dev examples/federation/check.sh   # a local build
set -euo pipefail
cd "$(dirname "$0")"

ROUTER=${ROUTER:-http://localhost:4000/}
SECRET=an-example-secret-of-at-least-32-characters
compose() { docker compose -f compose.yaml "$@"; }

cleanup() { [ "${KEEP:-}" = 1 ] || compose down --volumes --remove-orphans >/dev/null 2>&1; }
trap cleanup EXIT

compose up --detach --wait catalog reviews
compose up --detach router
for _ in $(seq 60); do
  curl --silent --fail "$ROUTER" -H 'content-type: application/json' \
    --data '{"query":"{ __typename }"}' >/dev/null 2>&1 && break
  sleep 2
done

token() {
  python3 - "$SECRET" "$1" <<'PY'
import base64, hashlib, hmac, json, sys, time
enc = lambda b: base64.urlsafe_b64encode(b).rstrip(b"=").decode()
head = enc(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
body = enc(json.dumps({"role": sys.argv[2], "exp": int(time.time()) + 600}).encode())
sig = hmac.new(sys.argv[1].encode(), f"{head}.{body}".encode(), hashlib.sha256).digest()
print(f"{head}.{body}.{enc(sig)}")
PY
}

failures=0
# ask URL QUERY [TOKEN] -- the response, keys sorted. URL "catalog" asks that
# subgraph from inside its own container rather than through the router.
ask() {
  local body
  body=$(python3 -c 'import json,sys; print(json.dumps({"query": sys.argv[1]}))' "$2")
  local args=(--silent -H 'content-type: application/json' --data "$body")
  [ -n "${3:-}" ] && args+=(-H "authorization: Bearer $3")
  if [ "$1" = catalog ]; then
    compose exec -T catalog curl "${args[@]}" http://localhost:3000/v1/graphql
  else
    curl "${args[@]}" "$1"
  fi | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin), sort_keys=True))'
}

# check NAME URL QUERY EXPECTED [TOKEN]
check() {
  local got want
  got=$(ask "$2" "$3" "${5:-}")
  want=$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1]), sort_keys=True))' "$4")
  if [ "$got" = "$want" ]; then
    echo "ok    $1"
  else
    echo "FAIL  $1"; echo "      want $want"; echo "      got  $got"
    failures=$((failures + 1))
  fi
}

check "catalog to reviews, anonymous: published reviews only" "$ROUTER" \
  '{ catalog_products(order_by: [{id: asc}]) { name reviews(order_by: [{id: asc}]) { rating } } }' \
  '{"data":{"catalog_products":[{"name":"Mechanical keyboard","reviews":[{"rating":5}]},{"name":"USB-C dock","reviews":[{"rating":4}]}]}}'

check "catalog to reviews, as an editor: every review" "$ROUTER" \
  '{ catalog_products(order_by: [{id: asc}]) { name reviews(order_by: [{id: asc}]) { rating } } }' \
  '{"data":{"catalog_products":[{"name":"Mechanical keyboard","reviews":[{"rating":5},{"rating":2}]},{"name":"USB-C dock","reviews":[{"rating":4}]}]}}' \
  "$(token editor)"

check "reviews to catalog, through a relationship" "$ROUTER" \
  '{ reviews_reviews(order_by: [{id: asc}]) { rating product { name price_cents } } }' \
  '{"data":{"reviews_reviews":[{"rating":5,"product":{"name":"Mechanical keyboard","price_cents":12900}},{"rating":4,"product":{"name":"USB-C dock","price_cents":8900}}]}}'

# Through the router the subgraph's error is redacted, as Apollo Router does
# by default; the subgraph itself says why.
BAD=$(SECRET=wrong-wrong-wrong-wrong-wrong-wrong-wrong token editor)
check "a token that does not verify gets nothing through the router" "$ROUTER" \
  '{ catalog_products { name } }' \
  '{"data":null,"errors":[{"message":"Subgraph errors redacted","path":[]}]}' \
  "$BAD"

check "and the subgraph refuses it as invalid-jwt" catalog \
  '{ catalog_products { name } }' \
  '{"errors":[{"extensions":{"code":"invalid-jwt","path":"$"},"message":"Could not verify JWT: JWT cryptographic operation failed"}]}' \
  "$BAD"

[ "$failures" = 0 ] && echo "all checks passed" || { echo "$failures failed"; exit 1; }
