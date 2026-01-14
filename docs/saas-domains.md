# SaaS Domain Management

> **Beta Feature** - This feature is currently in beta and will move to stable in Q2 2026. APIs may change before the stable release.

Multi-tenant domain management API for SaaS applications. Allow your customers to bring their own custom domains with automatic SSL, domain verification, and reverse proxy routing.

## Features

- **Domain Verification**: DNS TXT and HTTP challenge methods
- **SSL/TLS**: Automatic ACME (Let's Encrypt) + manual certificate upload
- **Authentication**: JWT + API Key dual authentication
- **Multi-tenant**: Complete tenant isolation with quotas
- **Dynamic Routing**: Per-domain routes without server restart
- **Hot Reload**: PostgreSQL LISTEN/NOTIFY for instant config updates

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
| `PUT` | `/api/v1/domains/:id` | Update domain |
| `DELETE` | `/api/v1/domains/:id` | Remove domain |
| `POST` | `/api/v1/domains/:id/verify` | Trigger verification |
| `POST` | `/api/v1/domains/:id/ssl/provision` | Provision SSL via ACME |
| `POST` | `/api/v1/domains/:id/ssl/upload` | Upload manual certificate |

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
| `GET` | `/.well-known/acme-challenge/:token` | ACME challenge (Let's Encrypt) |

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

2. Serve the verification file at:

```
https://app.example.com/.well-known/postrust-verification/<token>
```

Content: `postrust-verify=<token>`

3. Trigger verification (same as DNS method).

## SSL/TLS Certificates

### Automatic ACME Provisioning

After domain verification, provision SSL automatically:

```bash
curl -X POST http://localhost:8080/api/v1/domains/<id>/ssl/provision \
  -H "Authorization: ApiKey pk_live_abc123..."
```

This uses Let's Encrypt to issue and automatically renew certificates.

### Manual Certificate Upload

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

### Hot Reload

The proxy automatically reloads configuration when domains, routes, or upstreams change. This uses PostgreSQL LISTEN/NOTIFY:

```sql
-- Automatic triggers notify on changes
LISTEN proxy_config_change;
```

No server restart required when adding or modifying domains.

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

# 4. Provision SSL
curl -X POST "http://localhost:8080/api/v1/domains/$DOMAIN_ID/ssl/provision" \
  -H "Authorization: ApiKey $API_KEY"

# 5. Create upstream
UPSTREAM_ID=$(curl -s -X POST http://localhost:8080/api/v1/upstreams \
  -H "Authorization: ApiKey $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name": "backend", "lb_strategy": "round_robin"}' \
  | jq -r '.data.id')

# 6. Add backend server
curl -X POST "http://localhost:8080/api/v1/upstreams/$UPSTREAM_ID/backends" \
  -H "Authorization: ApiKey $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"address": "10.0.0.1:8080", "scheme": "http"}'

# 7. Create route
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
