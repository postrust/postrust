#!/bin/bash
# Postrust Test Script
# Runs tests with Docker PostgreSQL

set -e

echo "==> Starting PostgreSQL..."
docker-compose up -d postgres

echo "==> Waiting for PostgreSQL to be ready..."
until docker-compose exec -T postgres pg_isready -U postgres -d postrust_test; do
    echo "Waiting for database..."
    sleep 2
done

echo "==> PostgreSQL is ready!"

echo "==> Loading schema and fixtures..."
export PGPASSWORD=postgres
for f in scripts/init-db.sql scripts/test-fixtures.sql; do
    docker-compose exec -T postgres psql -q -v ON_ERROR_STOP=1 -U postgres -d postrust_test < "$f"
done

export DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test"

echo "==> Running tests..."
cargo test --all

# Tests that need a live database are marked #[ignore] so that `cargo test`
# passes without one; run them explicitly now that PostgreSQL is up.
echo "==> Running database-backed tests..."
cargo test --all -- --ignored

echo "==> Tests completed!"

# Optional: Stop PostgreSQL after tests
# docker-compose down
