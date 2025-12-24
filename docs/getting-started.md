# Getting Started

This guide will help you get Postrust up and running in minutes.

## Prerequisites

- PostgreSQL 12 or later
- Rust 1.75+ (for building from source) or Docker

## Installation

### Option 1: Using Docker (Recommended)

The fastest way to get started is with Docker:

```bash
# Clone the repository
git clone https://github.com/postrust/postrust.git
cd postrust

# Start PostgreSQL and Postrust
docker-compose up -d

# API is available at http://localhost:3000
curl http://localhost:3000/
```

### Option 2: Building from Source

```bash
# Clone the repository
git clone https://github.com/postrust/postrust.git
cd postrust

# Build in release mode
cargo build --release

# Binary is at target/release/postrust
./target/release/postrust --help
```

### Option 3: Pre-built Binaries

Download pre-built binaries from the [Releases page](https://github.com/postrust/postrust/releases).

## Your First API

### 1. Create a Database Table

Connect to your PostgreSQL database and create a simple table:

```sql
CREATE TABLE products (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    price DECIMAL(10, 2) NOT NULL,
    in_stock BOOLEAN DEFAULT true
);

INSERT INTO products (name, price) VALUES
    ('Widget', 29.99),
    ('Gadget', 49.99),
    ('Gizmo', 19.99);
```

### 2. Create a Database Role

Postrust uses PostgreSQL roles for access control:

```sql
-- Create an anonymous role
CREATE ROLE web_anon NOLOGIN;

-- Grant access to the products table
GRANT USAGE ON SCHEMA public TO web_anon;
GRANT SELECT ON public.products TO web_anon;
```

### 3. Start Postrust

```bash
# Set required environment variables
export DATABASE_URL="postgres://user:password@localhost:5432/mydb"
export PGRST_DB_ANON_ROLE="web_anon"

# Start the server
./target/release/postrust
```

### 4. Make Your First Request

```bash
# Get all products
curl http://localhost:3000/products

# Response:
# [
#   {"id": 1, "name": "Widget", "price": 29.99, "in_stock": true},
#   {"id": 2, "name": "Gadget", "price": 49.99, "in_stock": true},
#   {"id": 3, "name": "Gizmo", "price": 19.99, "in_stock": true}
# ]
```

## Basic Operations

### Filtering

```bash
# Products under $30
curl "http://localhost:3000/products?price=lt.30"

# Products in stock
curl "http://localhost:3000/products?in_stock=eq.true"

# Products matching name pattern
curl "http://localhost:3000/products?name=like.G*"
```

### Selecting Columns

```bash
# Only get name and price
curl "http://localhost:3000/products?select=name,price"
```

### Ordering

```bash
# Order by price descending
curl "http://localhost:3000/products?order=price.desc"
```

### Pagination

```bash
# Get first 10 products
curl "http://localhost:3000/products?limit=10"

# Get products 11-20
curl "http://localhost:3000/products?limit=10&offset=10"
```

## Adding Authentication

### 1. Set a JWT Secret

```bash
export PGRST_JWT_SECRET="your-super-secret-key-at-least-32-chars"
```

### 2. Create an Authenticated Role

```sql
CREATE ROLE web_user NOLOGIN;
GRANT USAGE ON SCHEMA public TO web_user;
GRANT ALL ON public.products TO web_user;
```

### 3. Make Authenticated Requests

```bash
# Create a JWT token (use your preferred method)
# Token payload: {"role": "web_user", "sub": "user123"}

curl http://localhost:3000/products \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIs..."
```

## Next Steps

- [Configuration Reference](./configuration.md) - All configuration options
- [API Reference](./api-reference.md) - Complete API documentation
- [Authentication](./authentication.md) - JWT and Row-Level Security
- [Deployment](./deployment.md) - Deploy to production
