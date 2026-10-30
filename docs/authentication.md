# Authentication

Postrust uses JWT (JSON Web Tokens) for authentication and PostgreSQL's Row-Level Security (RLS) for authorization.

> Examples below use root-level paths (e.g. `/users`), which require
> [compatibility mode](configuration.md#compatibility-settings); by default,
> prefix them with `/api`.

## Overview

```
┌─────────────┐     ┌──────────────┐     ┌──────────────┐
│   Client    │────▶│   Postrust   │────▶│  PostgreSQL  │
│  + JWT      │     │  Validates   │     │  RLS Checks  │
│             │     │  Token       │     │  Role Perms  │
└─────────────┘     └──────────────┘     └──────────────┘
```

## Configuration

### JWT Secret

`PGRST_JWT_SECRET` is read as PostgREST reads `jwt-secret`. A plain string is
an HMAC secret, and admits HS256 tokens:

```bash
# HS256 secret (minimum 32 characters)
PGRST_JWT_SECRET="your-super-secret-key-at-least-32-characters"

# Base64 encoded secret
PGRST_JWT_SECRET_IS_BASE64=true
PGRST_JWT_SECRET="eW91ci1zZWNyZXQ..."
```

### Asymmetric keys: JWK and JWKS

A JSON Web Key, or a JSON Web Key Set (`{"keys": [...]}`), is a set of
verification keys. That is how an identity provider's public keys are given:
the provider signs with its private key, and Postrust verifies with the public
one.

```bash
# One RSA public key
PGRST_JWT_SECRET='{"kty":"RSA","alg":"RS256","n":"0vx7agoebGcQ...","e":"AQAB"}'

# A JWKS, as an identity provider publishes it at its jwks_uri
PGRST_JWT_SECRET='{"keys":[{"kid":"2024-01","kty":"EC","crv":"P-256","x":"...","y":"..."},
                           {"kid":"2024-07","kty":"OKP","crv":"Ed25519","x":"..."}]}'
```

| `kty` | Algorithms | Notes |
|-------|------------|-------|
| `RSA` | RS256, RS384, RS512, PS256, PS384, PS512 | Modulus of at least 2048 bits |
| `EC` | ES256 (`crv: P-256`), ES384 (`crv: P-384`) | |
| `OKP` | EdDSA (`crv: Ed25519`) | |
| `oct` | HS256, HS384, HS512 | `k` is the HMAC secret, base64url |

**The algorithm comes from the key, never from the token.** A key admits the
algorithm its `alg` names, or, with no `alg`, every algorithm its type can do.
A token whose header names an algorithm the key does not admit is refused
before any signature is checked, so neither `alg: none` nor an HS256 token
"signed" with an RSA public key can get through.

**Choosing the key.** A token with a `kid` is checked against the key with that
`kid` and no other; an unknown `kid` is refused (`PGRST301`, "No suitable key
or wrong key type"). A token without a `kid` is tried against every key that
admits its algorithm, and passes if any of them verifies it. A plain string
secret has no `kid` to match, and accepts HS256 tokens whatever `kid` they
carry.

**Keys that cannot verify are set aside.** A key with `"use": "enc"`,
`key_ops` without `verify`, an encryption `alg` (`RSA-OAEP`, ...), or a type or
curve not listed above (P-521, X25519, ...) is skipped with a warning, because
providers' key sets routinely include them. A key that claims to be usable and
is malformed, or a secret that starts with `{` and is not valid JSON, stops the
server from starting: it is a mistake, and quietly treating it as an HMAC
secret would make the public key a signing secret.

The key set is parsed once, when the configuration is read. To have it
fetched from the identity provider instead, and kept current as the provider
rotates its keys, give its URL: see the next section.

### Keys from a JWKS URL

`PGRST_JWT_JWKS_URL` names the URL an identity provider publishes its public
keys at -- its `jwks_uri`. Postrust fetches the set from there, verifies
tokens with it exactly as with a JWKS given in `PGRST_JWT_SECRET`, and fetches
it again as time passes, so a key the provider rotates in is picked up without
a restart and a key it withdraws stops being accepted.

```bash
# Supabase Auth (GoTrue), on asymmetric signing keys
PGRST_JWT_JWKS_URL="https://<project>.supabase.co/auth/v1/.well-known/jwks.json"
# Auth0
PGRST_JWT_JWKS_URL="https://<tenant>.auth0.com/.well-known/jwks.json"
# Clerk
PGRST_JWT_JWKS_URL="https://<instance>.clerk.accounts.dev/.well-known/jwks.json"
# Amazon Cognito
PGRST_JWT_JWKS_URL="https://cognito-idp.<region>.amazonaws.com/<pool-id>/.well-known/jwks.json"
# Keycloak
PGRST_JWT_JWKS_URL="https://<host>/realms/<realm>/protocol/openid-connect/certs"
```

**When it is fetched.**

- **At start-up**, before the server answers anything (it waits at most 5
  seconds). If the provider cannot be reached the server starts anyway -- see
  "When the provider is down" below.
- **Again when the set is due**: after the response's `Cache-Control: max-age`,
  held to between 60 seconds and a day, or every `PGRST_JWT_JWKS_REFRESH`
  seconds (default 300) when the response gives no `max-age`. `no-cache` and
  `no-store` mean 60 seconds. The provider knows how often it rotates; the
  bounds keep `max-age=0` from becoming a fetch per request, and a year-long
  `max-age` from keeping a withdrawn key valid for a year.
- **When a token names a `kid` the set does not hold**: that may be a key the
  provider has just rotated in, so the set is fetched once more before the
  token is refused. At most once every 30 seconds -- anybody can mint a token
  with a made-up `kid`, and without the limit every such request would be a
  request to the provider. A token whose `kid` is still unknown afterwards is
  refused with `PGRST301`, "No suitable key or wrong key type". Requests that
  arrive while that fetch is in flight wait for it rather than starting their
  own.

A token whose key is already held never waits for any of this: verifying it
is the same in-memory check as with a fixed key set.

**What may be fetched.**

- `https` only, or `http` to `localhost`, `127.0.0.0/8` or `[::1]` -- a
  development provider, or a sidecar. A key set fetched over plain HTTP can be
  replaced by anyone on the path with keys of their own, and then they can sign
  any token they like. Any other `http` URL, or one that is not an absolute
  URL, stops the server from starting.
- The URL is read strictly: user info (`https://a@b/`), percent-encoding or
  non-ASCII in the host, and backslashes are refused rather than interpreted,
  so the host checked is always the host connected to.
- A redirect is followed only to the same host, at most three times. The URL
  configured names the party trusted to say which keys are valid; a redirect to
  another host is that trust handed to someone nobody chose.
- 5 seconds per fetch, and at most 1 MiB of response. Real key sets are a few
  kilobytes.

**What is done with the answer.** A 200 whose body parses as a JWKS with at
least one usable key replaces the set in force, whole. Anything else -- another
status, a body that is not a JWKS, a set of nothing but encryption keys -- is a
failed fetch. Every change of the set also empties the token cache
(`PGRST_JWT_CACHE_ENABLED`), so a token verified by a key the provider has
withdrawn is refused from the next request on, not served from the cache until
it expires. A fetch that brings the same set again changes nothing.

**When the provider is down.**

- A failed fetch keeps the last set that loaded, logs a warning, and is
  retried after 1, 2, 4, ... seconds, up to 30. An identity provider's bad
  minute does not log every user out.
- If no set has *ever* loaded, every token it would have verified is refused
  (`PGRST301`) -- the server fails closed -- and `/_/ready`, on the API port
  and on the admin port, answers `503` with `"jwks": false` and the reason. The
  server does not exit: an outage that crash-looped every replica would turn a
  partial outage into a total one. It becomes ready by itself once a retry
  succeeds. Requests without a token are still served as the anonymous role.

**With `PGRST_JWT_SECRET` as well.** Both may be set, and then a token either
one verifies is accepted: the keys in force are the secret's and the fetched
set's together. That is the deployment where a backend mints its own tokens
with a shared secret -- a service token, or Supabase's legacy `service_role`
key -- while users arrive with tokens from the identity provider. Neither
source widens the other: every key still admits only its own algorithms, and a
token naming a `kid` is still checked only against keys with that `kid`. A
token the secret verifies is accepted even before the fetched set has loaded.

**On each host.**

| Host | How the set is kept current |
|---|---|
| Server | Fetched at start-up, then by a background task whenever it is due. |
| Lambda | Fetched in the init phase and kept for the life of the sandbox. A sandbox is frozen between invocations, so there is no background task: the request that finds the set due fetches it (one request, at most every refresh interval, pays the fetch). |
| Worker | Fetched by the first request that needs it, and copied into the `POSTRUST_SCHEMA_CACHE` KV namespace under a key of its own, so a cold isolate reads it there instead of asking the provider. See [workers.md](workers.md#keys-from-a-jwks-url). |

An unknown `kid` is fetched for on every host.

### Supabase Auth (GoTrue)

Supabase Auth signs a user's access token with the project's signing key and
publishes the public half at `/auth/v1/.well-known/jwks.json`. Its tokens name
the database role in `role` (`authenticated`, or `anon`), the user's id in
`sub`, and `authenticated` as their audience -- which is what Postrust reads
by default, so a Supabase database's policies work unchanged:

```bash
PGRST_DB_ANON_ROLE=anon
PGRST_JWT_AUD=authenticated
PGRST_JWT_JWKS_URL="https://<project>.supabase.co/auth/v1/.well-known/jwks.json"
# The legacy JWT secret too, while service keys signed with it are in use:
PGRST_JWT_SECRET="<the project's legacy JWT secret>"
```

```sql
-- auth.uid() is this, and Supabase's own policies call it:
create policy "own notes" on notes for select to authenticated
  using (owner = (current_setting('request.jwt.claims', true)::json ->> 'sub')::uuid);
```

A project still on the legacy HS256 secret sets only `PGRST_JWT_SECRET`.
Self-hosted GoTrue publishes the same endpoint at `/.well-known/jwks.json` once
`GOTRUE_JWT_KEYS` holds an asymmetric key. `scripts/gotrue/e2e.sh` runs this
end to end against the `supabase/auth` image -- sign-up, token, RLS by `sub`
-- in both modes, and CI runs it on every change.

### Issuer

`PGRST_JWT_ISS` names the one issuer to accept. When it is set, a token's
`iss` must equal it exactly, and a token with **no** `iss` is refused too —
unlike `aud`, where an absent claim means "for everybody". Unset, `iss` is not
looked at. (Not a PostgREST setting.)

```bash
PGRST_JWT_ISS="https://auth.example.com/"
```

### Anonymous Role

Set a role for unauthenticated requests:

```bash
PGRST_DB_ANON_ROLE="web_anon"
```

## JWT Token Format

### Required Claims

```json
{
  "role": "web_user",
  "exp": 1735689600
}
```

| Claim | Description | Required |
|-------|-------------|----------|
| `role` | PostgreSQL role to use | Yes |
| `exp` | Expiration timestamp | Recommended |
| `iat` | Issued at timestamp | Optional |
| `nbf` | Not before timestamp | Optional |
| `aud` | Audience | Optional |
| `iss` | Issuer | Required when `PGRST_JWT_ISS` is set |
| `sub` | Subject (user ID) | Optional |

#### How the claims are checked

A claim that is absent is not a claim that is wrong. A token carrying no `exp`
does not expire; one carrying no `aud` is for anybody. Only a claim that is
*present and not the kind of thing that claim is* is rejected — `"exp":
"soon"` is a malformed token, and so is an `aud` that is neither a string nor
a list of them.

**`exp` is checked to the second.** `nbf` and `iat` are given thirty seconds
of slack. The asymmetry is deliberate: `nbf` and `iat` describe a token that
is not valid *yet*, and a client whose clock runs slightly fast mints one
through nobody's fault, so refusing it makes a working deployment fail
intermittently. Forgiving an `exp` is a different thing — it keeps a session
alive past the second its issuer said it ended.

**A role claim must be a string.** `"role": 42` or `"role": {"a": 1}` names no
role; it falls back to `PGRST_DB_ANON_ROLE` exactly as an absent claim does,
rather than asking PostgreSQL for a role of that spelling.

Faults are reported in a fixed order, so a token that is both expired and out
of audience is reported as expired — the first thing wrong with it, and the
one the client can act on.

### Custom Claims

Add any custom claims for use in RLS policies:

```json
{
  "role": "web_user",
  "sub": "user123",
  "email": "user@example.com",
  "organization_id": 42,
  "permissions": ["read", "write"],
  "exp": 1735689600
}
```

## Creating Tokens

### Using jwt.io

1. Go to [jwt.io](https://jwt.io)
2. Select HS256 algorithm
3. Enter your payload
4. Enter your secret
5. Copy the encoded token

### Using Node.js

```javascript
const jwt = require('jsonwebtoken');

const token = jwt.sign(
  {
    role: 'web_user',
    sub: 'user123',
    email: 'user@example.com'
  },
  process.env.JWT_SECRET,
  { expiresIn: '1h' }
);
```

### Using Python

```python
import jwt
import datetime

token = jwt.encode(
    {
        'role': 'web_user',
        'sub': 'user123',
        'email': 'user@example.com',
        'exp': datetime.datetime.utcnow() + datetime.timedelta(hours=1)
    },
    os.environ['JWT_SECRET'],
    algorithm='HS256'
)
```

### Using Rust

```rust
use jsonwebtoken::{encode, Header, EncodingKey};
use serde::{Serialize, Deserialize};

#[derive(Serialize)]
struct Claims {
    role: String,
    sub: String,
    exp: i64,
}

let claims = Claims {
    role: "web_user".to_string(),
    sub: "user123".to_string(),
    exp: chrono::Utc::now().timestamp() + 3600,
};

let token = encode(
    &Header::default(),
    &claims,
    &EncodingKey::from_secret(secret.as_bytes())
)?;
```

## Making Authenticated Requests

Include the token in the `Authorization` header:

```bash
curl http://localhost:3000/users \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."
```

## PostgreSQL Roles

### Creating Roles

```sql
-- Anonymous role (unauthenticated requests)
CREATE ROLE web_anon NOLOGIN;

-- Authenticated user role
CREATE ROLE web_user NOLOGIN;

-- Admin role
CREATE ROLE web_admin NOLOGIN;

-- Grant schema access
GRANT USAGE ON SCHEMA public TO web_anon, web_user, web_admin;
```

### Granting Permissions

```sql
-- Anonymous users can only read
GRANT SELECT ON public.products TO web_anon;

-- Authenticated users can read and write
GRANT SELECT, INSERT, UPDATE, DELETE ON public.orders TO web_user;

-- Admins can do everything
GRANT ALL ON ALL TABLES IN SCHEMA public TO web_admin;
GRANT ALL ON ALL SEQUENCES IN SCHEMA public TO web_admin;
```

### The Role Postrust Connects As

Connect as a role with no privileges of its own, that can become the roles a
token may name. Each request runs `SET LOCAL ROLE` to the token's role (or the
anonymous one), so that role's grants and row-level security policies are what
apply:

```sql
CREATE ROLE authenticator LOGIN NOINHERIT PASSWORD '...';
GRANT web_anon, web_user, web_admin TO authenticator;
```

```bash
DATABASE_URL=postgres://authenticator:...@db/app
```

`NOINHERIT` means the connection holds none of those roles' privileges until a
request switches to one, so nothing runs with more than the token allows. The
schema cache describes every table a role the authenticator can become may
use, so it does not need to read anything itself.

A request's queries run as the role it switched to, whatever the connection
logged in as, so its grants and policies apply either way. What the login role
decides is everything that runs *before* a switch, or without one -- loading the
schema cache, reading GraphQL enum tables, and the subscription broker's
triggers. Connected as a superuser or as the tables'
owner, all of that runs with privileges that ignore grants and row-level
security, and a request that somehow skipped its switch would too. With a
`NOINHERIT` authenticator it can do nothing it was not given. On PostgreSQL 16
and later, each grant must also carry the `SET` option (the default), since
switching to the role is what a request does.

## Row-Level Security (RLS)

RLS allows fine-grained access control at the row level.

### Enabling RLS

```sql
ALTER TABLE orders ENABLE ROW LEVEL SECURITY;
```

### Policy Examples

The verified claims reach the database in every transaction a request runs,
REST and GraphQL alike -- including each GraphQL subscription, which runs as
the identity its WebSocket connection was opened with (see
[Subscriptions](#subscriptions) below). The examples read them from
`request.jwt.claims`, the whole token as one JSON document, which is set on
every request and so is always defined; see
[Accessing Claims in SQL](#accessing-claims-in-sql) for the per-claim
settings and why they need more care.

#### Users Can Only See Their Own Orders

```sql
CREATE POLICY orders_user_select ON orders
    FOR SELECT
    TO web_user
    USING (
        user_id = (current_setting('request.jwt.claims', true)::json->>'sub')::integer
    );
```

#### Users Can Only Modify Their Own Data

```sql
CREATE POLICY orders_user_insert ON orders
    FOR INSERT
    TO web_user
    WITH CHECK (
        user_id = (current_setting('request.jwt.claims', true)::json->>'sub')::integer
    );

CREATE POLICY orders_user_update ON orders
    FOR UPDATE
    TO web_user
    USING (
        user_id = (current_setting('request.jwt.claims', true)::json->>'sub')::integer
    );
```

#### Organization-Based Access

```sql
CREATE POLICY orders_org_access ON orders
    FOR ALL
    TO web_user
    USING (
        organization_id =
            (current_setting('request.jwt.claims', true)::json->>'organization_id')::integer
    );
```

#### Role-Based Access

The `role` claim is the database role the request runs as -- the token's
`role`, or the anonymous role when it named none -- so a policy attached `TO`
a role has already settled it. An application-level role that does not map to
a database role belongs in a claim of its own:

```sql
CREATE POLICY admin_full_access ON orders
    FOR ALL
    TO web_admin
    USING (true);

CREATE POLICY user_own_orders ON orders
    FOR SELECT
    TO web_user
    USING (
        user_id = (current_setting('request.jwt.claims', true)::json->>'sub')::integer
        OR current_setting('request.jwt.claims', true)::json->>'app_role' = 'manager'
    );
```

## Accessing Claims in SQL

The whole token is one setting, `request.jwt.claims`, a JSON document with
`role` set to the role the request runs as. It is the form to reach for: it is
set on every request, so it is never missing, and it reaches nested claims.

```sql
-- The role the request runs as
SELECT current_setting('request.jwt.claims', true)::json->>'role';

-- The subject (user ID)
SELECT current_setting('request.jwt.claims', true)::json->>'sub';

-- A custom claim
SELECT (current_setting('request.jwt.claims', true)::json->>'organization_id')::integer;

-- A nested claim
SELECT current_setting('request.jwt.claims', true)::json->'user'->>'email';
```

Each top-level claim is also its own setting, `request.jwt.claims.<name>`, for
a policy that would rather not parse JSON. Three things are true of those that
are not true of the document:

- **A missing claim reads as `''` as well as `NULL`.** PostgreSQL keeps a
  setting a connection has ever seen, emptied, after the transaction that set it
  ends. Connections are pooled, so a token without `organization_id` reads
  `NULL` on a connection that has never served one with it and `''` on one that
  has -- and `''::integer` is an error. Wrap every read in `NULLIF(..., '')`:

  ```sql
  SELECT NULLIF(current_setting('request.jwt.claims.organization_id', true), '')::integer;
  ```

- **Only top-level claims, and only names a setting can have.** A name made of
  letters, digits and underscores, starting with a letter or underscore, with
  dots between parts. `user_id` and `org.region` are settings; `2fa` and
  `https://hasura.io/jwt/claims` are not, and are only in the document.
- **A claim that is an object or array is its JSON text.** `user` holding
  `{"email": "a@example.com"}` is one setting, `request.jwt.claims.user`, whose
  value is that JSON; there is no `request.jwt.claims.user.email`. Read nested
  claims from the document.

Setting names are case-insensitive in PostgreSQL, so two claims whose names
differ only in case are the same setting, and one of them is kept.

### In Functions

```sql
CREATE FUNCTION get_my_orders()
RETURNS SETOF orders
LANGUAGE sql
SECURITY DEFINER
AS $$
    SELECT * FROM orders
    WHERE user_id = (current_setting('request.jwt.claims', true)::json->>'sub')::integer;
$$;
```

## Subscriptions

A GraphQL subscription is authenticated as an HTTP request is, once per
WebSocket connection. Browsers cannot set headers on a WebSocket, so the token
goes in the `connection_init` payload, under `headers` as Hasura's clients send
it or at the top level:

```json
{"type": "connection_init", "payload": {"headers": {"Authorization": "Bearer <token>"}}}
```

Headers on the upgrade request are read too, and the payload's win where both
name the same header. `X-Hasura-Admin-Secret` and `X-Hasura-Role` are honoured
from either place exactly as they are over HTTP. A connection HTTP would refuse
-- a wrong admin secret, a role the token does not allow -- is refused at
`connection_init`. Every operation on the connection then runs as that
identity: its role, its session variables and its claims, in every transaction
a live query re-reads in.

The identity is the one the connection was opened with. A token that expires
while a subscription is open does not end it; a client that needs it to should
reconnect with a fresh token.

## Error Handling

A `401` also carries a `WWW-Authenticate` challenge. Where no token was
offered it is a plain `Bearer`; where a token was read and refused it names
which refusal, so a client can tell "rotate your key" from "fix your clock"
without parsing the body:

```
WWW-Authenticate: Bearer error="invalid_token", error_description="JWT expired"
```

### No identity — `PGRST302`

Nothing said who is asking, and there is no `PGRST_DB_ANON_ROLE` to be.

```json
{
  "code": "PGRST302",
  "message": "Anonymous access is disabled"
}
```
HTTP Status: 401

### The token could not be read — `PGRST301`

The signature did not verify, the key was wrong, or the `Authorization`
header was not a bearer token. A token for which the server holds no key at
all — an unknown `kid`, or an algorithm no configured key admits — is reported
as `No suitable key or wrong key type`. So is every token while keys from
`PGRST_JWT_JWKS_URL` have never loaded.

```json
{
  "code": "PGRST301",
  "message": "JWT cryptographic operation failed"
}
```
HTTP Status: 401

### The token was read and refused — `PGRST303`

The signature verified and the claims were not acceptable. The message names
which claim: `JWT expired`, `JWT not yet valid`, `JWT issued at future`,
`JWT not in audience`, `JWT issuer is not the expected one`, `The JWT 'exp'
claim must be a number`, or `The JWT 'aud' claim must be a string or an array
of strings`.

```json
{
  "code": "PGRST303",
  "message": "JWT expired"
}
```
HTTP Status: 401

### No JWT secret configured — `PGRST300`

A token was offered and the server has nothing to verify it with.

```json
{
  "code": "PGRST300",
  "message": "Server lacks JWT secret"
}
```
HTTP Status: 500

### Insufficient Permissions

```json
{
  "code": "42501",
  "message": "permission denied for table orders"
}
```
HTTP Status: 403

## Best Practices

1. **Use Short-Lived Tokens**: Set reasonable expiration times (15 min - 1 hour)

2. **Refresh Tokens**: Implement token refresh mechanism in your client

3. **Secure Secrets**:
   - Use environment variables
   - Never commit secrets to version control
   - Rotate secrets periodically

4. **Validate All Claims**: Check `exp`, `nbf`, and `aud` claims

5. **Use RLS Everywhere**: Enable RLS on all tables with sensitive data

6. **Test Policies**: Verify RLS policies work as expected:
   ```sql
   SET LOCAL ROLE web_user;
   SET LOCAL request.jwt.claims.sub TO '123';
   SELECT * FROM orders; -- Should only show user's orders
   ```

7. **Audit Access**: Log authentication events for security monitoring
