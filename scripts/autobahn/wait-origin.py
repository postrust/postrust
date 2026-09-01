#!/usr/bin/env python3
"""Block until the WebSocket origin actually completes a handshake.

A TCP probe is not enough. Docker publishes a port by binding a forwarder on
the host as soon as the container is created, so `nc -z` succeeds while the
process inside is still starting. The suite then runs against nothing and
writes an empty report -- which is how a baseline run once produced `{}` and
looked like a crash rather than a race.

So probe the thing that matters: a real RFC 6455 handshake returning 101.
"""

import base64
import os
import socket
import sys
import time

DEADLINE_SECONDS = 60


def handshake(port):
    key = base64.b64encode(os.urandom(16)).decode()
    with socket.create_connection(("127.0.0.1", port), timeout=2) as sock:
        sock.sendall(
            (
                f"GET /chat HTTP/1.1\r\n"
                f"Host: 127.0.0.1:{port}\r\n"
                "Upgrade: websocket\r\n"
                "Connection: Upgrade\r\n"
                f"Sec-WebSocket-Key: {key}\r\n"
                "Sec-WebSocket-Version: 13\r\n\r\n"
            ).encode()
        )
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(4096)
            if not chunk:
                return False
            buf += chunk
        return b"101" in buf.split(b"\r\n", 1)[0]


def main(port):
    deadline = time.monotonic() + DEADLINE_SECONDS
    while time.monotonic() < deadline:
        try:
            if handshake(port):
                return 0
        except (OSError, socket.timeout):
            pass
        time.sleep(0.25)
    print(
        f"error: nothing completed a WebSocket handshake on 127.0.0.1:{port} "
        f"within {DEADLINE_SECONDS}s",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main(int(sys.argv[1])))
