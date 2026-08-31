#!/bin/bash
# The Garden's echo server stands in for an origin: it reports how it parsed
# what the proxy forwarded. 0xdafe (56062) is the port the Garden's own
# transducer images use.
set -euo pipefail

python3 /tools/echo_server.py 127.0.0.1 "$((0xdafe))" &

exec postrust-proxy /app/postrust-proxy.toml
