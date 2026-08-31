#!/usr/bin/env bash
# Install postrust-proxy as an HTTP Garden target and (optionally) run it.
#
# HTTP Garden (https://github.com/narfindustries/http-garden) is a differential
# fuzzer for HTTP servers and proxies. It sends a payload through a proxy under
# test and shows how a set of origin servers parsed the result -- which is how
# request-smuggling and header-handling discrepancies surface. That is the class
# of bug postrust-proxy is most exposed to: nothing in the forwarder strips
# hop-by-hop headers.
#
# The Garden lives outside this repo (it is GPL-3.0, and its images build dozens
# of other servers). This script clones it into scripts/http-garden/garden,
# which is gitignored, and installs our target into it.
#
# Usage:
#   scripts/http-garden/setup.sh              # clone, install target, build
#   scripts/http-garden/setup.sh --no-build   # install only
#   GARDEN_DIR=/elsewhere scripts/http-garden/setup.sh
#
# Then, from the Garden directory:
#   ./garden.sh start postrust hyper nginx gunicorn
#   ./garden.sh repl
#   garden> payload 'GET / HTTP/1.1\r\nHost: a\r\nConnection: keep-alive\r\n\r\n' \
#           | transduce postrust | fanout | grid
#
# Requirements: docker, git, python3 (the Garden's repl needs its own deps --
# see its README; `uv sync` inside the Garden directory is the quick path).
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
GARDEN_DIR="${GARDEN_DIR:-$HERE/garden}"
GARDEN_REPO="${GARDEN_REPO:-https://github.com/narfindustries/http-garden}"
BUILD=1
[ "${1:-}" = "--no-build" ] && BUILD=0

die() { printf 'error: %s\n' "$1" >&2; exit 1; }
stage() { printf '\n==> %s\n' "$1"; }

command -v docker >/dev/null || die "docker not found"
command -v git >/dev/null || die "git not found"

stage "Fetching HTTP Garden into $GARDEN_DIR"
if [ -d "$GARDEN_DIR/.git" ]; then
    git -C "$GARDEN_DIR" pull --ff-only || printf 'warning: could not fast-forward, using existing checkout\n'
else
    git clone --depth 1 "$GARDEN_REPO" "$GARDEN_DIR" || die "clone failed"
fi

stage "Installing the postrust target"
TARGET_DIR="$GARDEN_DIR/images/postrust"
mkdir -p "$TARGET_DIR"
cp "$HERE/images/postrust/Dockerfile" \
   "$HERE/images/postrust/postrust-proxy.toml" \
   "$HERE/images/postrust/start.sh" "$TARGET_DIR/"

# Snapshot the working tree, so the image tests uncommitted changes. target/ is
# excluded because it is large and the container rebuilds anyway; .git because
# nothing in the build reads it.
stage "Snapshotting the working tree into the image context"
rm -rf "$TARGET_DIR/src"
mkdir -p "$TARGET_DIR/src"
if command -v rsync >/dev/null; then
    rsync -a --exclude target --exclude .git --exclude 'scripts/http-garden/garden' \
        "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" "$ROOT/crates" "$TARGET_DIR/src/"
else
    cp "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" "$TARGET_DIR/src/"
    cp -R "$ROOT/crates" "$TARGET_DIR/src/crates"
    rm -rf "$TARGET_DIR/src/crates"/*/target
fi

# The Garden discovers targets from its docker-compose.yml. `role: transducer`
# is what marks a target as a proxy rather than an origin server.
stage "Registering the target in the Garden's docker-compose.yml"
python3 - "$GARDEN_DIR/docker-compose.yml" <<'PY'
import sys, re
path = sys.argv[1]
text = open(path).read()
if re.search(r'^  postrust:$', text, re.M):
    print("already registered")
    sys.exit(0)
entry = (
    "  postrust:\n"
    "    build:\n"
    "      context: ./images/postrust\n"
    "    volumes:\n"
    "    - ./tools:/tools\n"
    "    x-props:\n"
    "      role: transducer\n"
)
# Insert in the services block, keeping it roughly alphabetical is not required;
# appending before the first top-level key after `services:` is enough.
m = re.search(r'^services:\n', text, re.M)
if not m:
    sys.exit("no services: block in docker-compose.yml")
text = text[:m.end()] + entry + text[m.end():]
open(path, "w").write(text)
print("registered")
PY
[ $? -eq 0 ] || die "could not register the target"

if [ "$BUILD" = "1" ]; then
    stage "Building the soil base image and the postrust target"
    ( cd "$GARDEN_DIR" && docker build -t http-garden-soil:latest ./images/http-garden-soil ) \
        || die "soil image build failed"
    ( cd "$GARDEN_DIR" && docker compose build postrust ) || die "postrust image build failed"
fi

cat <<EOF

Done. Next:

  cd $GARDEN_DIR
  ./garden.sh start postrust hyper nginx gunicorn
  ./garden.sh repl

In the repl, send a payload through the proxy and compare how origins read it:

  garden> payload 'GET / HTTP/1.1\r\nHost: a\r\nConnection: keep-alive\r\n\r\n' | transduce postrust | fanout | grid

Re-run this script after changing proxy source to refresh the snapshot.
EOF
