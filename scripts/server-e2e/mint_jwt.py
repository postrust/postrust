#!/usr/bin/env python3
"""Mint an HS256 JWT for the end-to-end checks.

Hand-rolled rather than pulled from PyJWT: the checks have to run wherever the
repository is cloned, and a pip install is a dependency the rest of this
harness does not have. HS256 is an HMAC over two base64url segments, which is
short enough to be obviously correct here.
"""

import argparse
import base64
import hashlib
import hmac
import json
import time


def b64(raw: bytes) -> str:
    """base64url with the padding stripped, as JWT requires."""
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def mint(key: bytes, claims: dict) -> str:
    header = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    payload = b64(json.dumps(claims, separators=(",", ":")).encode())
    signed = f"{header}.{payload}".encode()
    return f"{header}.{payload}.{b64(hmac.new(key, signed, hashlib.sha256).digest())}"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    secret = parser.add_mutually_exclusive_group(required=True)
    secret.add_argument("--secret", help="the signing key, as text")
    secret.add_argument(
        "--b64-secret",
        help="base64 text whose decoded bytes are the signing key, which is "
        "what PGRST_JWT_SECRET_IS_BASE64 selects",
    )
    parser.add_argument(
        "--expires-in",
        type=int,
        default=300,
        help="seconds until exp; ignored if the claims set one (default: 300)",
    )
    parser.add_argument("claims", help="claims, as a JSON object")
    args = parser.parse_args()

    key = base64.b64decode(args.b64_secret) if args.b64_secret else args.secret.encode()
    claims = json.loads(args.claims)
    claims.setdefault("exp", int(time.time()) + args.expires_in)
    print(mint(key, claims))


if __name__ == "__main__":
    main()
