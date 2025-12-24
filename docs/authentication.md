# Authentication

Postrust uses JWT (JSON Web Tokens) for authentication and PostgreSQL's Row-Level Security (RLS) for authorization.

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

Set the secret used to sign tokens:

```bash
# HS256 secret (minimum 32 characters)
PGRST_JWT_SECRET="your-super-secret-key-at-least-32-characters"

# Base64 encoded secret
PGRST_JWT_SECRET_IS_BASE64=true
PGRST_JWT_SECRET="eW91ci1zZWNyZXQ..."
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
| `sub` | Subject (user ID) | Optional |

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

## Row-Level Security (RLS)

RLS allows fine-grained access control at the row level.

### Enabling RLS

```sql
ALTER TABLE orders ENABLE ROW LEVEL SECURITY;
```

### Policy Examples

#### Users Can Only See Their Own Orders

```sql
CREATE POLICY orders_user_select ON orders
    FOR SELECT
    TO web_user
    USING (
        user_id = current_setting('request.jwt.claims.sub', true)::integer
    );
```

#### Users Can Only Modify Their Own Data

```sql
CREATE POLICY orders_user_insert ON orders
    FOR INSERT
    TO web_user
    WITH CHECK (
        user_id = current_setting('request.jwt.claims.sub', true)::integer
    );

CREATE POLICY orders_user_update ON orders
    FOR UPDATE
    TO web_user
    USING (
        user_id = current_setting('request.jwt.claims.sub', true)::integer
    );
```

#### Organization-Based Access

```sql
CREATE POLICY orders_org_access ON orders
    FOR ALL
    TO web_user
    USING (
        organization_id = current_setting('request.jwt.claims.organization_id', true)::integer
    );
```

#### Role-Based Access

```sql
CREATE POLICY admin_full_access ON orders
    FOR ALL
    TO web_admin
    USING (true);

CREATE POLICY user_own_orders ON orders
    FOR SELECT
    TO web_user
    USING (
        user_id = current_setting('request.jwt.claims.sub', true)::integer
        OR current_setting('request.jwt.claims.role', true) = 'manager'
    );
```

## Accessing Claims in SQL

JWT claims are available as GUC (Grand Unified Configuration) variables:

```sql
-- Get the role claim
SELECT current_setting('request.jwt.claims.role', true);

-- Get the subject (user ID)
SELECT current_setting('request.jwt.claims.sub', true);

-- Get custom claims
SELECT current_setting('request.jwt.claims.organization_id', true)::integer;

-- Get nested claims
SELECT current_setting('request.jwt.claims.user.email', true);
```

### In Functions

```sql
CREATE FUNCTION get_my_orders()
RETURNS SETOF orders
LANGUAGE sql
SECURITY DEFINER
AS $$
    SELECT * FROM orders
    WHERE user_id = current_setting('request.jwt.claims.sub', true)::integer;
$$;
```

## Error Handling

### Missing Token

```json
{
  "code": "PGRST301",
  "message": "JWT token required"
}
```
HTTP Status: 401

### Invalid Token

```json
{
  "code": "PGRST302",
  "message": "Invalid JWT token"
}
```
HTTP Status: 401

### Expired Token

```json
{
  "code": "PGRST303",
  "message": "JWT token has expired"
}
```
HTTP Status: 401

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
