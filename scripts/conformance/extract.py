#!/usr/bin/env python3
"""
Extract HTTP request cases from PostgREST's hspec-wai specs.

The specs cannot be executed against a foreign server (they drive the WAI
Application in-process and import PostgREST.Config directly), but the request
side of each example is a literal we can lift out and replay over HTTP.

We deliberately do NOT try to interpret the hspec expectations. The comparison
is differential: the same request goes to stock PostgREST and to Postrust, and
the reference implementation's live response is the oracle.

Header lists that name a binding (`[auth]`) are resolved against the `let`
bindings in the same file, and `generateJWT` payloads are signed here with the
suite's own secret, so auth-bearing examples are covered too.
"""
import base64
import hashlib
import hmac
import json
import os
import re
import sys

SPEC_ROOT = sys.argv[1]
OUT = sys.argv[2]

# test/spec/SpecHelper.hs: baseCfg uses this as the HS256 signing key.
JWT_SECRET = b"reallyreallyreallyreallyverysafe"

VERB = {
    "get": "GET", "post": "POST", "patch": "PATCH",
    "put": "PUT", "delete": "DELETE", "head": "HEAD", "options": "OPTIONS",
}

SITE = re.compile(
    r"""(?:(?P<req>request\s+method(?P<mc>[A-Z][a-z]+)\s+)|(?<![\w'])(?P<verb>get|post|patch|put|delete|head|options)\s+)"
        (?P<path>(?:[^"\\]|\\.)*)"
    """,
    re.VERBOSE,
)


def b64url(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def make_jwt(payload_json):
    """Sign a JWT the way SpecHelper's generateJWT does: HS256, no extra claims."""
    header = b64url(b'{"alg":"HS256","typ":"JWT"}')
    try:
        compact = json.dumps(json.loads(payload_json), separators=(",", ":"))
    except Exception:
        return None
    body = b64url(compact.encode())
    signing_input = f"{header}.{body}".encode()
    sig = hmac.new(JWT_SECRET, signing_input, hashlib.sha256).digest()
    return f"{header}.{body}.{b64url(sig)}"


def skip_ws(s, i):
    """Advance past whitespace and Haskell line comments."""
    while i < len(s):
        if s[i] in " \t\r\n":
            i += 1
        elif s.startswith("--", i):
            j = s.find("\n", i)
            i = len(s) if j < 0 else j + 1
        else:
            break
    return i


def match_bracket(s, i):
    """s[i] == '['. Return index just past the matching ']', respecting strings."""
    depth = 0
    while i < len(s):
        c = s[i]
        if c == '"':
            i += 1
            while i < len(s) and s[i] != '"':
                i += 2 if s[i] == "\\" else 1
        elif c == "[":
            depth += 1
        elif c == "]":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return -1


def read_string(s, i):
    """s[i] == '"'. Return (value, next_index) with Haskell escapes resolved."""
    i += 1
    buf = []
    while i < len(s) and s[i] != '"':
        if s[i] == "\\":
            nxt = s[i + 1]
            buf.append({"n": "\n", "t": "\t", "\\": "\\", '"': '"'}.get(nxt, nxt))
            i += 2
        else:
            buf.append(s[i])
            i += 1
    return "".join(buf), i + 1


HDR_PAIR = re.compile(r'\(\s*"((?:[^"\\]|\\.)*)"\s*,\s*"((?:[^"\\]|\\.)*)"\s*\)')
# let auth = authHeaderJWT "<literal token>"
BIND_JWT_LIT = re.compile(r'(?:let\s+)?(\w+)\s*=\s*authHeaderJWT\s+"([^"]*)"')
# let auth = authHeaderJWT $ generateJWT [json| {...} |]
BIND_JWT_GEN = re.compile(
    r'(?:let\s+)?(\w+)\s*=\s*authHeaderJWT\s*\$\s*generateJWT\s*\[json\|(.*?)\|\]', re.S)
# let single = ("Accept", "application/vnd.pgrst.object+json")
BIND_PAIR = re.compile(r'(?:let\s+)?(\w+)\s*=\s*\(\s*"([^"]*)"\s*,\s*"([^"]*)"\s*\)')
# jwtPayload = [json| {...} |]  -- referenced by generateJWT jwtPayload
BIND_JSON = re.compile(r'(?:let\s+)?(\w+)\s*=\s*\[json\|(.*?)\|\]', re.S)
BIND_JWT_REF = re.compile(r'(?:let\s+)?(\w+)\s*=\s*authHeaderJWT\s*\$\s*generateJWT\s+(\w+)')


def collect_bindings(src):
    """Map binding name -> (header-name, header-value) for the resolvable forms."""
    binds = {}
    json_vals = {name: body.strip() for name, body in BIND_JSON.findall(src)}

    for name, token in BIND_JWT_LIT.findall(src):
        binds[name] = ("Authorization", f"Bearer {token}")
    for name, payload in BIND_JWT_GEN.findall(src):
        token = make_jwt(payload.strip())
        if token:
            binds[name] = ("Authorization", f"Bearer {token}")
    for name, ref in BIND_JWT_REF.findall(src):
        token = make_jwt(json_vals.get(ref, ""))
        if token:
            binds[name] = ("Authorization", f"Bearer {token}")
    for name, hname, hvalue in BIND_PAIR.findall(src):
        binds.setdefault(name, (hname, hvalue))
    return binds


# authHeaderJWT "<token>" appearing inline inside a header list
INLINE_JWT = re.compile(r'authHeaderJWT\s+"([^"]*)"')
INLINE_JWT_GEN = re.compile(r'authHeaderJWT\s*\$\s*generateJWT\s*\[json\|(.*?)\|\]', re.S)


def parse_headers(chunk, binds):
    """Return list of [name, value], or None if some entry can't be resolved."""
    inner = chunk.strip()[1:-1].strip()
    if not inner:
        return []

    headers = []
    residue = inner

    for hname, hvalue in HDR_PAIR.findall(inner):
        headers.append([hname, hvalue])
    residue = HDR_PAIR.sub("", residue)

    for token in INLINE_JWT.findall(residue):
        headers.append(["Authorization", f"Bearer {token}"])
    residue = INLINE_JWT.sub("", residue)

    for payload in INLINE_JWT_GEN.findall(residue):
        token = make_jwt(payload.strip())
        if not token:
            return None
        headers.append(["Authorization", f"Bearer {token}"])
    residue = INLINE_JWT_GEN.sub("", residue)

    # Whatever is left should be bare binding names.
    for word in re.findall(r"[A-Za-z_]\w*", residue):
        if word not in binds:
            return None
        headers.append(list(binds[word]))
    if re.sub(r"[A-Za-z_]\w*|[,\s]", "", residue):
        return None

    return headers


cases = []
stats = {"sites": 0, "extracted": 0, "unresolved_headers": 0, "auth_cases": 0}

for dirpath, _, files in os.walk(SPEC_ROOT):
    for fn in sorted(files):
        if not fn.endswith(".hs"):
            continue
        full = os.path.join(dirpath, fn)
        rel = os.path.relpath(full, SPEC_ROOT)
        src = open(full, encoding="utf-8").read()
        binds = collect_bindings(src)

        for m in SITE.finditer(src):
            stats["sites"] += 1
            method = m.group("mc").upper() if m.group("req") else VERB[m.group("verb")]
            path = m.group("path")
            i = m.end()

            if src[i:skip_ws(src, i) + 2].strip().startswith("<>"):
                continue  # path built by concatenation -- not a literal

            headers, body = [], None
            j = skip_ws(src, i)

            if j < len(src) and src[j] == "[" and not src.startswith("[json|", j):
                end = match_bracket(src, j)
                if end < 0:
                    continue
                parsed = parse_headers(src[j:end], binds)
                if parsed is None:
                    stats["unresolved_headers"] += 1
                    continue
                headers = parsed
                j = skip_ws(src, end)

            if src.startswith("[json|", j):
                end = src.find("|]", j)
                if end < 0:
                    continue
                body = src[j + 6:end].strip()
            elif j < len(src) and src[j] == '"':
                body, _ = read_string(src, j)

            if "\\" in path:
                path = path.encode().decode("unicode_escape")

            if any(h[0].lower() == "authorization" for h in headers):
                stats["auth_cases"] += 1

            cases.append({
                "id": f"{rel}:{src.count(chr(10), 0, m.start()) + 1}",
                "spec": rel,
                "method": method,
                "path": path,
                "headers": headers,
                "body": body,
                # Anything that isn't a plain read may change state, so the
                # runner restores the fixture data before replaying it.
                "mutating": method in ("POST", "PATCH", "PUT", "DELETE"),
            })
            stats["extracted"] += 1

seen = {}
for c in cases:
    key = (c["method"], c["path"], tuple(map(tuple, c["headers"])), c["body"])
    seen.setdefault(key, c)
unique = [c for c in seen.values() if c["path"].startswith("/")]

json.dump(unique, open(OUT, "w"), indent=1)
stats["unique"] = len(unique)
stats["mutating"] = sum(1 for c in unique if c["mutating"])
print(json.dumps(stats, indent=2))
