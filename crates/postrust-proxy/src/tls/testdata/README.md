# TLS test fixtures

Self-signed certificates for the checks in `tls::validate`, generated once so
the tests need no certificate authority and no clock.

| File | What it is for |
| --- | --- |
| `valid.pem` / `valid.key.pem` | A correct pair covering `example.test` and `www.example.test` |
| `other.pem` / `other.key.pem` | A different domain *and* a different key — used for "wrong domain", and its key paired with `valid.pem` for "key does not match the chain" |
| `expired.pem` / `expired.key.pem` | Expired in 2020 |
| `wildcard.pem` / `wildcard.key.pem` | `*.example.test`, for the one-label wildcard rule |
| `expiry.pem` | No SAN at all, so the common-name fallback has something to read |

Regenerate with:

```bash
cd crates/postrust-proxy/src/tls/testdata

openssl req -x509 -newkey rsa:2048 -nodes -keyout valid.key.pem \
  -subj /CN=example.test \
  -addext "subjectAltName=DNS:example.test,DNS:www.example.test" \
  -days 365 -out valid.pem

openssl req -x509 -newkey rsa:2048 -nodes -keyout other.key.pem \
  -subj /CN=other.test -addext "subjectAltName=DNS:other.test" \
  -days 365 -out other.pem

openssl req -x509 -newkey rsa:2048 -nodes -keyout wildcard.key.pem \
  -subj "/CN=*.example.test" -addext "subjectAltName=DNS:*.example.test" \
  -days 365 -out wildcard.pem

openssl req -x509 -newkey rsa:2048 -nodes -keyout expired.key.pem \
  -subj /CN=example.test -addext "subjectAltName=DNS:example.test" \
  -not_before 20200101000000Z -not_after 20200102000000Z -out expired.pem

openssl req -x509 -newkey rsa:2048 -nodes -keyout /dev/null \
  -subj /CN=expiry.test -days 3650 -out expiry.pem
```

`valid.pem`, `other.pem` and `wildcard.pem` expire in 2027. When they do, the
tests that assert *acceptance* will start failing — regenerate them rather than
loosening the expiry check, which is one of the four things the upload path
exists to enforce.

`expired.pem` is supposed to be expired and always will be, so
`an_expired_certificate_is_refused` needs no maintenance.

These are throwaway keys for local tests. They authorise nothing, and none of
them is trusted by anything.
