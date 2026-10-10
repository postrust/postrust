"""Invoke a function in the Lambda runtime emulator as a function URL would.

    invoke.py URL METHOD PATH [QUERY] [--token T] [--body B] [--header K:V]

Prints the status, then the body, then `content-range` when there is one.
"""
import argparse
import base64
import json
import urllib.request

p = argparse.ArgumentParser()
p.add_argument("url")
p.add_argument("method")
p.add_argument("path")
p.add_argument("query", nargs="?", default="")
p.add_argument("--token")
p.add_argument("--body")
p.add_argument("--header", action="append", default=[])
a = p.parse_args()

host = "abc.lambda-url.us-east-1.on.aws"
headers = {"host": host, "accept": "application/json"}
if a.token:
    headers["authorization"] = "Bearer " + a.token
if a.body is not None:
    headers["content-type"] = "application/json"
for h in a.header:
    k, v = h.split(":", 1)
    headers[k.strip().lower()] = v.strip()

# The payload format 2.0 event a function URL sends.
event = {
    "version": "2.0",
    "routeKey": "$default",
    "rawPath": a.path,
    "rawQueryString": a.query,
    "headers": headers,
    "isBase64Encoded": False,
    "body": a.body,
    "requestContext": {
        "accountId": "anonymous",
        "apiId": "abc",
        "domainName": host,
        "domainPrefix": "abc",
        "http": {"method": a.method, "path": a.path, "protocol": "HTTP/1.1",
                 "sourceIp": "203.0.113.1", "userAgent": "serverless-test"},
        "requestId": "req",
        "routeKey": "$default",
        "stage": "$default",
        "time": "10/Oct/2026:00:00:00 +0000",
        "timeEpoch": 0,
    },
}
req = urllib.request.Request(a.url, data=json.dumps(event).encode(), method="POST")
r = json.load(urllib.request.urlopen(req, timeout=60))
if "statusCode" not in r:
    print("INVOKE-ERROR", json.dumps(r)[:300])
    raise SystemExit
body = r.get("body") or ""
if r.get("isBase64Encoded"):
    body = base64.b64decode(body).decode()
hdrs = {k.lower(): v for k, v in (r.get("headers") or {}).items()}
print(r["statusCode"])
print(body)
print(hdrs.get("content-range", ""))
