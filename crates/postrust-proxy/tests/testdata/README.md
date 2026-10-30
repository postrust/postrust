# Control-plane test fixtures

For the certificate-upload chain check in `tests/control_plane.rs`. All three
cover `upload-chain.test` and expire in 2035.

| File | What it is for |
| --- | --- |
| `ca.pem` | A private root, handed to `UploadTrust::Roots` |
| `leaf.pem` / `leaf.key.pem` | Issued by that root: trusted with it, untrusted by the public roots |
| `selfsigned.pem` / `selfsigned.key.pem` | Self-signed, so trusted by nothing |

The root's key was discarded after signing, so nothing more can be issued
under it. Regenerate with:

```bash
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout ca.key.pem -subj "/CN=Postrust Test Root CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" -days 3650 -out ca.pem

openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout leaf.key.pem -subj /CN=upload-chain.test -out leaf.csr
printf '%s\n' 'basicConstraints=critical,CA:FALSE' \
  'keyUsage=critical,digitalSignature' 'extendedKeyUsage=serverAuth' \
  'subjectAltName=DNS:upload-chain.test' > leaf.ext
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key.pem -CAcreateserial \
  -days 3650 -extfile leaf.ext -out leaf.pem

openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout selfsigned.key.pem -subj /CN=upload-chain.test \
  -addext "subjectAltName=DNS:upload-chain.test" -days 3650 -out selfsigned.pem

rm leaf.csr leaf.ext ca.srl ca.key.pem
```

Throwaway keys for local tests; they authorise nothing.
