"""jwt.py SECRET ROLE SUB -- an HS256 token valid for an hour."""
import base64, hashlib, hmac, json, sys, time

def b64(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()

secret, role, sub = sys.argv[1:4]
signing = b64(json.dumps({"alg": "HS256", "typ": "JWT"}).encode()) + "." + b64(
    json.dumps({"role": role, "sub": sub, "exp": int(time.time()) + 3600}).encode())
print(signing + "." + b64(hmac.new(secret.encode(), signing.encode(), hashlib.sha256).digest()))
