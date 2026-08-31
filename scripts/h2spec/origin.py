#!/usr/bin/env python3
"""A trivial HTTP/1.1 origin for the h2spec run.

h2spec cares about the HTTP/2 conversation with the proxy, not about what is
behind it, so the origin only has to answer promptly and identically every
time. Keep-alive matters: h2spec opens many connections in quick succession.
"""

import http.server
import socketserver
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _respond(self):
        body = b"ok"
        self.send_response(200)
        self.send_header("content-type", "text/plain")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = _respond
    do_POST = _respond
    do_PUT = _respond
    do_HEAD = _respond

    def log_message(self, *args):
        pass


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


if __name__ == "__main__":
    port = int(sys.argv[1])
    Server(("127.0.0.1", port), Handler).serve_forever()
