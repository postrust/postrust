# SaaS Domain Management

> **Beta Feature** - This feature is currently in beta and will move to stable in Q2 2026. APIs may change before the stable release.

Multi-tenant domain management API for SaaS applications. Allow your customers to bring their own custom domains, with domain verification and reverse proxy routing.

Automatic SSL is implemented over ACME HTTP-01: a verified domain with
`ssl_provider = "acme"` is queued, and a background worker obtains and renews
its certificate. See [SSL/TLS Certificates](#ssltls-certificates).

## Features

- **Domain Verification**: DNS TXT and HTTP challenge methods
- **SSL/TLS**: certificates obtained and renewed automatically via ACME HTTP-01
  (Let's Encrypt), and served per-domain by SNI — see
  [Serving an issued certificate](#serving-an-issued-certificate)
- **Authentication**: JWT + API Key dual authentication
- **Multi-tenant**: Complete tenant isolation with quotas
- **Database-backed routing**: per-domain routes stored in PostgreSQL rather
  than a config file, and editable through the admin API

Configuration is read at startup and there is no hot reload — see
[Applying a configuration change](#applying-a-configuration-change).

## Authentication

The SaaS API supports two authentication methods:

### JWT Authentication

```bash
curl -X GET http://localhost:8080/api/v1/domains \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..."
```

JWT tokens must include:
- `sub`: User ID (required)
- `tenant_id`: Tenant UUID (required)
- `role`: User role (optional)
- `scopes`: Array of permission scopes (optional, defaults to `["*"]`)

### API Key Authentication

```bash
curl -X GET http://localhost:8080/api/v1/domains \
  -H "Authorization: ApiKey pk_live_abc123..."
```

API keys are prefixed with `pk_` for identification and can have restricted scopes.

## API Endpoints

### Tenants

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/v1/tenant` | Get current tenant info |
| `PUT` | `/api/v1/tenant` | Update tenant settings |
| `GET` | `/api/v1/tenant/usage` | Get usage statistics |

### API Keys

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/v1/auth/api-keys` | Create API key |
| `GET` | `/api/v1/auth/api-keys` | List API keys |
| `GET` | `/api/v1/auth/api-keys/:id` | Get API key details |
| `DELETE` | `/api/v1/auth/api-keys/:id` | Revoke API key |

### Domains

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/v1/domains` | List domains |
| `POST` | `/api/v1/domains` | Add domain |
| `GET` | `/api/v1/domains/:id` | Get domain details |
| `PUT` | `/api/v1/domains/:id` | Update verification method or SSL provider |
| `DELETE` | `/api/v1/domains/:id` | Remove domain |
| `POST` | `/api/v1/domains/:id/verify` | Trigger verification |
| `POST` | `/api/v1/domains/:id/enable` | Enable a verified domain |
| `POST` | `/api/v1/domains/:id/disable` | Disable a domain |
| `POST` | `/api/v1/domains/:id/ssl/provision` | Queue (or requeue) ACME issuance |
| `POST` | `/api/v1/domains/:id/ssl/upload` | Upload a certificate |

### Routes

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/v1/domains/:id/routes` | List routes |
| `POST` | `/api/v1/domains/:id/routes` | Create route |
| `GET` | `/api/v1/domains/:id/routes/:rid` | Get route details |
| `PUT` | `/api/v1/domains/:id/routes/:rid` | Update route |
| `DELETE` | `/api/v1/domains/:id/routes/:rid` | Delete route |

### Upstreams

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/v1/upstreams` | List upstreams |
| `POST` | `/api/v1/upstreams` | Create upstream |
| `GET` | `/api/v1/upstreams/:id` | Get upstream details |
| `PUT` | `/api/v1/upstreams/:id` | Update upstream |
| `DELETE` | `/api/v1/upstreams/:id` | Delete upstream |
| `POST` | `/api/v1/upstreams/:id/backends` | Add backend server |
| `DELETE` | `/api/v1/upstreams/:id/backends/:bid` | Remove backend |

### Well-Known (Public)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/.well-known/postrust-verification/:token` | HTTP verification challenge |
| `GET` | `/.well-known/acme-challenge/:token` | ACME HTTP-01 challenge |

## Domain Verification

### DNS TXT Verification

1. Add your domain:

```bash
curl -X POST http://localhost:8080/api/v1/domains \
  -H "Authorization: ApiKey pk_live_abc123..." \
  -H "Content-Type: application/json" \
  -d '{
    "domain": "app.example.com",
    "verification_method": "dns"
  }'
```

Response:
```json
{
  "success": true,
  "data": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "domain": "app.example.com",
    "verification_status": "pending",
    "verification_method": "dns",
    "verification_token": "postrust-verify=abc123def456"
  }
}
```

2. Create the DNS TXT record:

```
_postrust-verification.app.example.com TXT "postrust-verify=abc123def456"
```

3. Trigger verification:

```bash
curl -X POST http://localhost:8080/api/v1/domains/550e8400-e29b-41d4-a716-446655440000/verify \
  -H "Authorization: ApiKey pk_live_abc123..."
```

### HTTP Challenge Verification

1. Add your domain with HTTP verification:

```bash
curl -X POST http://localhost:8080/api/v1/domains \
  -H "Authorization: ApiKey pk_live_abc123..." \
  -H "Content-Type: application/json" \
  -d '{
    "domain": "app.example.com",
    "verification_method": "http"
  }'
```

2. The proxy serves the challenge itself, at:

```
https://app.example.com/.well-known/postrust-verification/<token>
```

It answers only for a challenge that is in the database, unexpired, unresolved,
and whose domain matches the request's `Host`.

3. Trigger verification (same as DNS method).

### Prefer DNS verification

HTTP verification asks the claimant to serve content at a path on the domain —
but once that domain's DNS points at this proxy, this proxy is what serves that
path. Passing therefore shows that the domain resolves here and that a
challenge was issued for it. It does not show that the claimant controls the
domain.

DNS verification does show control, because the TXT record can only be placed
by whoever holds the zone. It is the default (`verification_method` defaults to
`dns`), and it is the one to use for anything that matters.

## SSL/TLS Certificates

### Automatic ACME provisioning

A domain with `ssl_provider = "acme"` that passes verification is queued
(`ssl_status = "pending"`). A background worker then places the order, answers
the HTTP-01 challenge, and stores the certificate. Nothing else to call.

```bash
# Verify. That is the whole trigger.
curl -X POST http://localhost:8080/api/v1/domains/<id>/verify \
  -H "Authorization: ApiKey pk_live_abc123..."

# Watch ssl_status go pending -> provisioning -> active
curl http://localhost:8080/api/v1/domains/<id> \
  -H "Authorization: ApiKey pk_live_abc123..."
```

Configure the CA in the proxy's TOML:

```toml
[tls]
acme_enabled = true
acme_email = "ops@example.com"
acme_staging = true          # until the whole flow works; see below
cert_dir = "/var/lib/postrust/certs"
```

**Use staging first.** Let's Encrypt's production rate limits are per-domain
per-week and easy to exhaust while debugging. `acme_staging = true` issues
untrusted certificates against far looser limits.

The worker needs `DATABASE_URL`: the ACME account, the pending challenges and
the certificates all live in the database, and the challenge is served from a
table so that any instance can answer whichever one the CA happens to reach.
Without a database the worker logs that it is not starting rather than failing
later on the first order.

**There is no `provision` endpoint**, and an older version of this document
documented one that was never in the router. Issuance takes several round trips
to the CA plus a challenge fetch, and retrying under a rate limit is the normal
failure mode — none of which belongs in a request. Verification queues; the
worker works.

#### Provisioning explicitly

Verification queues automatically, but there are cases where you need to ask:
a domain whose provider you have just switched to `acme`, or one that failed and
has been fixed.

```bash
curl -X POST http://localhost:8080/api/v1/domains/<id>/ssl/provision \
  -H "Authorization: ApiKey pk_live_abc123..."
```

Returns **202 Accepted**. It sets state and returns; the worker does the work.
It does not issue inline, and that is deliberate — see the note above. Poll
`GET /domains/<id>` and watch `ssl_status`.

The call is idempotent, and it is also how you retry: it clears the attempt
count, the recorded error and the backoff, so the next pass picks the domain up
immediately rather than waiting out a wait computed for a cause you have just
fixed.

It refuses (404) a domain that is not verified, or whose `ssl_provider` is not
`acme`. Queueing either would have the worker place an order that cannot
succeed, and spend a rate limit doing it.

#### When it fails

Failures record `ssl_error` on the domain and retry with exponential backoff
from one minute, capped at four hours, giving up after ten attempts. The usual
cause is that the domain's DNS was never pointed at the proxy, so the CA could
not fetch the challenge; the error says so. Fix the cause, then call
`ssl/provision`.

#### Renewal

Certificates within 30 days of expiry are requeued automatically. Let's Encrypt
issues for 90, so there is a month to notice a problem.

#### Why HTTP-01 and not DNS-01

HTTP-01 needs only that the domain resolves to the proxy, which is already true
for any domain it serves. DNS-01 would need write access to each tenant's zone,
which a proxy has no way to obtain. The trade is that HTTP-01 cannot issue
wildcards.

#### Testing it

`scripts/acme/run.sh` runs the whole flow against
[Pebble](https://github.com/letsencrypt/pebble), Let's Encrypt's test CA, which
deliberately misbehaves. See `scripts/acme/README.md`.

### Manual certificate upload

For a certificate that comes from somewhere else — an internal CA, or one issued
by hand.

```bash
curl -X POST http://localhost:8080/api/v1/domains/<id>/ssl/upload \
  -H "Authorization: ApiKey pk_live_abc123..." \
  -H "Content-Type: application/json" \
  -d '{"cert_pem": "-----BEGIN CERTIFICATE-----\n...", "key_pem": "-----BEGIN PRIVATE KEY-----\n..."}'
```

The certificate is **checked before it is stored**, and a rejection says which
check failed:

| Check | Why it is refused |
| --- | --- |
| The chain and key parse | Nothing usable otherwise |
| **The key matches the chain** | The listener would accept the upload and then fail every TLS handshake, with nothing in the logs pointing at the upload |
| Not expired | Same, immediately |
| Covers this domain | A certificate for another name authorises nothing here. Wildcards count, for exactly one label — `*.example.com` covers `api.example.com`, not `example.com` and not `a.b.example.com` |

On success the domain's `ssl_provider` becomes `manual` and its `ssl_status`
becomes `active`. The stored certificate has `auto_renew` turned off: nothing
here can renew a certificate it did not obtain, so the renewal scan must not
queue it for an ACME worker that has no authorization for the domain. Upload a
replacement before it expires, or switch the domain to `acme`.

### Changing a domain

```bash
curl -X PUT http://localhost:8080/api/v1/domains/<id> \
  -H "Authorization: ApiKey pk_live_abc123..." \
  -H "Content-Type: application/json" \
  -d '{"ssl_provider": "acme"}'
```

Partial — absent fields are left alone. Two fields can change:
`verification_method` (which takes effect on the next verification attempt, and
does not un-verify a verified domain) and `ssl_provider`. Moving a **verified**
domain to `acme` queues it for issuance.

**The domain name is not updatable.** It is the identity of the record and what
the verification token proves control of, so a rename would carry a proof of
ownership over to a name nobody has proved anything about. Delete and re-add.

For custom certificates:

```bash
curl -X POST http://localhost:8080/api/v1/domains/<id>/ssl/upload \
  -H "Authorization: ApiKey pk_live_abc123..." \
  -H "Content-Type: application/json" \
  -d '{
    "certificate": "-----BEGIN CERTIFICATE-----\n...",
    "private_key": "-----BEGIN PRIVATE KEY-----\n...",
    "chain": "-----BEGIN CERTIFICATE-----\n..."
  }'
```

## Routing Configuration

### Create an Upstream

```bash
curl -X POST http://localhost:8080/api/v1/upstreams \
  -H "Authorization: ApiKey pk_live_abc123..." \
  -H "Content-Type: application/json" \
  -d '{
    "name": "api-backend",
    "lb_strategy": "round_robin",
    "health_check_path": "/health",
    "health_check_interval": 30
  }'
```

### Add Backend Servers

```bash
curl -X POST http://localhost:8080/api/v1/upstreams/<upstream_id>/backends \
  -H "Authorization: ApiKey pk_live_abc123..." \
  -H "Content-Type: application/json" \
  -d '{
    "address": "10.0.0.1:8080",
    "scheme": "http",
    "weight": 100
  }'
```

### Create Routes

```bash
curl -X POST http://localhost:8080/api/v1/domains/<domain_id>/routes \
  -H "Authorization: ApiKey pk_live_abc123..." \
  -H "Content-Type: application/json" \
  -d '{
    "name": "api-route",
    "path_pattern": "/api",
    "path_type": "prefix",
    "methods": ["GET", "POST", "PUT", "DELETE"],
    "upstream_id": "<upstream_id>",
    "strip_path": true,
    "add_headers": {
      "X-Forwarded-Host": "app.example.com"
    }
  }'
```

### Path Types

| Type | Description | Example |
|------|-------------|---------|
| `prefix` | Matches path prefix | `/api` matches `/api/users` |
| `exact` | Exact path match | `/api` matches only `/api` |
| `regex` | Regular expression | `/api/v[0-9]+` |

### Load Balancing Strategies

| Strategy | Description |
|----------|-------------|
| `round_robin` | Distribute requests evenly |
| `random` | Random backend selection |
| `ip_hash` | Consistent hashing by client IP |
| `least_connections` | Route to least busy backend |
| `weighted` | Weighted distribution |

## API Keys

### Create an API Key

```bash
curl -X POST http://localhost:8080/api/v1/auth/api-keys \
  -H "Authorization: Bearer <jwt>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Production API Key",
    "scopes": ["domains:read", "domains:write", "routes:write"],
    "expires_in_days": 365
  }'
```

Response:
```json
{
  "success": true,
  "data": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "name": "Production API Key",
    "key": "pk_live_abc123def456...",
    "key_prefix": "pk_live_abc",
    "scopes": ["domains:read", "domains:write", "routes:write"],
    "expires_at": "2026-01-13T00:00:00Z"
  },
  "message": "Store this key securely - it will not be shown again"
}
```

### Available Scopes

| Scope | Description |
|-------|-------------|
| `*` | Full access |
| `domains:read` | Read domain information |
| `domains:write` | Create/update/delete domains |
| `routes:read` | Read routes |
| `routes:write` | Create/update/delete routes |
| `upstreams:read` | Read upstreams |
| `upstreams:write` | Create/update/delete upstreams |
| `api-keys:read` | Read API keys |
| `api-keys:write` | Create/revoke API keys |

## Database Schema

The SaaS module uses the following PostgreSQL tables:

```sql
-- Tenants (SaaS customers)
proxy_tenants

-- API Keys
proxy_api_keys

-- Custom domains
proxy_domains

-- Domain routes
proxy_domain_routes

-- Upstreams (backend groups)
proxy_domain_upstreams

-- Backend servers
proxy_domain_backends

-- Verification challenges
proxy_verification_challenges

-- Audit log
proxy_domain_audit_log
```

Run the migration to create these tables:

```bash
sqlx migrate run
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `PROXY_JWT_SECRET` | JWT signing secret | Required for JWT auth |
| `PROXY_ACME_EMAIL` | Email for ACME registration | Required for auto-SSL |
| `PROXY_ACME_DIRECTORY` | ACME directory URL | Let's Encrypt production |

### Applying a configuration change

**The proxy needs a restart.** Routes, upstreams and domains are read from the
database once, at startup, by `load_from_database`. Writing a row — through the
admin API or directly — changes what the *next* start will load, and nothing
about the process already running.

There is no LISTEN/NOTIFY channel, no watcher and no reload endpoint. Earlier
versions of this document described a `proxy_config_change` channel, and the
crate carried a `ConfigReloader` whose channel nobody read alongside a
`POST /config/reload` that answered "Configuration reload requested" without
reloading. Both were removed rather than left to be believed; this section was
the last place still describing them.

Certificates are the exception: those are re-read on a timer and do not need a
restart. See [Serving an issued certificate](#serving-an-issued-certificate).

### Serving an issued certificate

A certificate in the store is served automatically, chosen per handshake by the
name the client asked for (SNI).

The HTTPS listener is built with an SNI resolver backed by `CertificateStore`
rather than with a single fixed certificate, so each tenant domain is answered
with its own. `tls.cert_file` and `tls.key_file`, where configured, become the
fallback for a handshake whose SNI matches nothing stored and for a client that
sends no SNI at all.

Neither is required — a multi-tenant deployment can serve stored certificates
alone — but the listener has to be asked for. `tls.acme_enabled` asks for it;
so does `server.https_enabled`, which is the switch to use when certificates
arrive by `ssl/upload` rather than from a CA. A `DATABASE_URL` on its own does
not: keeping routes in PostgreSQL is not a request to terminate TLS. See
[the proxy's TLS section](./proxy.md#tls).

A wildcard covers exactly one label, as RFC 6125 requires — `*.example.com`
answers for `a.example.com`, and not for `a.b.example.com` or the bare
`example.com`.

**Timing.** A certificate issued or uploaded while the proxy is running is
picked up within a minute; the resolver re-reads the store on a timer. The read
goes to the database rather than to any cache, so a renewal performed by another
instance is picked up too.

## Error Responses

All errors follow a consistent format:

```json
{
  "success": false,
  "error": "Error message here"
}
```

### HTTP Status Codes

| Code | Description |
|------|-------------|
| `200` | Success |
| `201` | Created |
| `400` | Bad request / validation error |
| `401` | Unauthorized (missing/invalid auth) |
| `403` | Forbidden (tenant suspended, insufficient scope) |
| `404` | Resource not found |
| `409` | Conflict (e.g., domain already exists) |
| `429` | Rate limited / quota exceeded |
| `500` | Internal server error |

## Example: Complete Domain Setup

```bash
# 1. Create API key (using JWT)
API_KEY=$(curl -s -X POST http://localhost:8080/api/v1/auth/api-keys \
  -H "Authorization: Bearer $JWT" \
  -H "Content-Type: application/json" \
  -d '{"name": "Setup Key"}' | jq -r '.data.key')

# 2. Add domain
DOMAIN_ID=$(curl -s -X POST http://localhost:8080/api/v1/domains \
  -H "Authorization: ApiKey $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"domain": "app.example.com", "verification_method": "dns"}' \
  | jq -r '.data.id')

# 3. After adding DNS TXT record, verify
curl -X POST "http://localhost:8080/api/v1/domains/$DOMAIN_ID/verify" \
  -H "Authorization: ApiKey $API_KEY"

# 4. Create upstream
UPSTREAM_ID=$(curl -s -X POST http://localhost:8080/api/v1/upstreams \
  -H "Authorization: ApiKey $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name": "backend", "lb_strategy": "round_robin"}' \
  | jq -r '.data.id')

# 5. Add backend server
curl -X POST "http://localhost:8080/api/v1/upstreams/$UPSTREAM_ID/backends" \
  -H "Authorization: ApiKey $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"address": "10.0.0.1:8080", "scheme": "http"}'

# 6. Create route
curl -X POST "http://localhost:8080/api/v1/domains/$DOMAIN_ID/routes" \
  -H "Authorization: ApiKey $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "all-traffic",
    "path_pattern": "/",
    "path_type": "prefix",
    "upstream_id": "'"$UPSTREAM_ID"'"
  }'

# Done! Traffic to app.example.com now routes to your backend
```
