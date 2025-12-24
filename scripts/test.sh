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

echo "==> Running tests..."
DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test" cargo test --all

echo "==> Tests completed!"

# Optional: Stop PostgreSQL after tests
# docker-compose down
