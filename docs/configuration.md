# Configuration

Postrust is configured entirely through environment variables, making it easy to deploy in containerized and serverless environments.

## Required Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgres://user:pass@host:5432/db` |

## Database Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_DB_SCHEMAS` | Comma-separated list of schemas to expose | `public` |
| `PGRST_DB_ANON_ROLE` | Role for unauthenticated requests | (none) |
| `PGRST_DB_POOL_SIZE` | Connection pool size | `10` |
| `PGRST_DB_POOL_TIMEOUT` | Pool timeout in seconds | `30` |
| `PGRST_DB_TX_ISOLATION` | Transaction isolation level | `read committed` |

### Database URL Format

```
postgres://[user[:password]@][host][:port][/database][?options]
```

Examples:
```bash
# Local development
DATABASE_URL="postgres://postgres:postgres@localhost:5432/mydb"

# With SSL
DATABASE_URL="postgres://user:pass@host:5432/db?sslmode=require"

# AWS RDS
DATABASE_URL="postgres://user:pass@mydb.xxx.us-east-1.rds.amazonaws.com:5432/mydb"
```

### Schema Configuration

Expose multiple schemas:

```bash
# Expose public and api schemas
PGRST_DB_SCHEMAS="public,api"
```

Access different schemas via the `Accept-Profile` header:

```bash
curl http://localhost:3000/users \
  -H "Accept-Profile: api"
```

## Authentication Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_JWT_SECRET` | Secret for HS256/HS384/HS512 | (none) |
| `PGRST_JWT_SECRET_IS_BASE64` | Is secret base64 encoded? | `false` |
| `PGRST_JWT_AUD` | Required audience claim | (none) |
| `PGRST_JWT_ROLE_CLAIM_KEY` | Claim key containing role | `role` |

### JWT Secret

```bash
# Plain text secret (min 32 characters for HS256)
PGRST_JWT_SECRET="your-super-secret-key-at-least-32-characters"

# Base64 encoded secret
PGRST_JWT_SECRET_IS_BASE64=true
PGRST_JWT_SECRET="eW91ci1zdXBlci1zZWNyZXQta2V5LWF0LWxlYXN0LTMyLWNoYXJhY3RlcnM="
```

### Custom Role Claim

By default, Postrust looks for the role in the `role` claim. Override this:

```bash
# Use nested claim
PGRST_JWT_ROLE_CLAIM_KEY="user.role"

# JWT payload: {"user": {"role": "admin"}}
```

## Server Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_SERVER_HOST` | Server bind address | `127.0.0.1` |
| `PGRST_SERVER_PORT` | Server port | `3000` |
| `PGRST_SERVER_CORS_ORIGINS` | Allowed CORS origins | `*` |

### CORS Configuration

```bash
# Allow specific origins
PGRST_SERVER_CORS_ORIGINS="https://example.com,https://app.example.com"

# Allow all origins (development only)
PGRST_SERVER_CORS_ORIGINS="*"
```

## Logging Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_LOG_LEVEL` | Log level | `info` |
| `RUST_LOG` | Detailed Rust logging | (none) |

### Log Levels

- `error` - Only errors
- `warn` - Warnings and errors
- `info` - General information (default)
- `debug` - Detailed debugging
- `trace` - Very verbose

```bash
# Production
PGRST_LOG_LEVEL="warn"

# Development
PGRST_LOG_LEVEL="debug"
RUST_LOG="postrust=debug,sqlx=info"
```

## Request Limits

| Variable | Description | Default |
|----------|-------------|---------|
| `PGRST_MAX_ROWS` | Maximum rows returned | `1000` |
| `PGRST_MAX_BODY_SIZE` | Maximum request body (bytes) | `10485760` |

## Example Configurations

### Development

```bash
DATABASE_URL="postgres://postgres:postgres@localhost:5432/dev"
PGRST_DB_ANON_ROLE="anon"
PGRST_LOG_LEVEL="debug"
PGRST_SERVER_HOST="0.0.0.0"
```

### Production

```bash
DATABASE_URL="postgres://user:pass@prod-db:5432/app?sslmode=require"
PGRST_DB_SCHEMAS="api"
PGRST_DB_ANON_ROLE="web_anon"
PGRST_DB_POOL_SIZE="20"
PGRST_JWT_SECRET="${JWT_SECRET}"
PGRST_LOG_LEVEL="warn"
PGRST_SERVER_HOST="0.0.0.0"
PGRST_SERVER_PORT="8080"
PGRST_MAX_ROWS="500"
```

### AWS Lambda

```bash
DATABASE_URL="${DATABASE_URL}"
PGRST_DB_ANON_ROLE="web_anon"
PGRST_JWT_SECRET="${JWT_SECRET}"
PGRST_LOG_LEVEL="info"
# Note: Pool size should be small for Lambda
PGRST_DB_POOL_SIZE="1"
```

## Configuration File (Optional)

You can also use a `.env` file:

```bash
# .env
DATABASE_URL=postgres://user:pass@localhost:5432/mydb
PGRST_DB_ANON_ROLE=web_anon
PGRST_JWT_SECRET=your-secret-key
```

Load it automatically:
```bash
# The server reads .env files by default
./postrust
```

## Validation

Postrust validates configuration on startup:

```bash
./postrust

# Output:
# INFO postrust: Configuration validated
# INFO postrust: Connected to database
# INFO postrust: Server listening on 0.0.0.0:3000
```

Common validation errors:

- `DATABASE_URL is required` - Set the database connection string
- `Invalid DATABASE_URL format` - Check the URL syntax
- `JWT_SECRET must be at least 32 characters` - Use a longer secret
- `Unknown schema: xyz` - Schema doesn't exist in database
