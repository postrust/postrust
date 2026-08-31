#!/usr/bin/env python3
"""A WebSocket echo origin that negotiates permessage-deflate.

Autobahn's own `wstest -m echoserver` does not offer the extension, which left
all 216 compression cases (sections 12 and 13) scored UNIMPLEMENTED -- a
coverage hole, not a proxy result. The `websockets` library negotiates
permessage-deflate by default, so pointing the suite at this origin turns those
cases into a real exercise of the tunnel carrying compressed, fragmented frames.

Echoes binary as binary and text as text, which is what the suite expects.
"""

import asyncio
import sys

import websockets


async def echo(connection):
    try:
        async for message in connection:
            await connection.send(message)
    except websockets.exceptions.ConnectionClosed:
        pass


async def main(host, port):
    # max_size=None so the section 9 large-payload cases are not rejected by the
    # origin before they ever reach the tunnel.
    async with websockets.serve(
        echo,
        host,
        port,
        max_size=None,
        compression="deflate",
    ):
        await asyncio.Future()


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1], int(sys.argv[2])))
