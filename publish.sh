#!/bin/bash
set -e

echo "Publishing postrust crates to crates.io..."
echo ""

# Leaf crates (no internal dependencies)
echo "=== Step 1: Publishing leaf crates ==="
echo "Publishing postrust-sql..."
cargo publish -p postrust-sql
echo "Publishing postrust-auth..."
cargo publish -p postrust-auth

echo "Waiting 30s for crates.io to index..."
sleep 30

# Level 2: depends on sql
echo ""
echo "=== Step 2: Publishing postrust-core ==="
cargo publish -p postrust-core

echo "Waiting 30s for crates.io to index..."
sleep 30

# Level 3: depends on core
echo ""
echo "=== Step 3: Publishing postrust-response and postrust-graphql ==="
echo "Publishing postrust-response..."
cargo publish -p postrust-response
echo "Publishing postrust-graphql..."
cargo publish -p postrust-graphql

echo "Waiting 30s for crates.io to index..."
sleep 30

# Level 4: depends on all above
echo ""
echo "=== Step 4: Publishing server adapters ==="
echo "Publishing postrust-server..."
cargo publish -p postrust-server
echo "Publishing postrust-lambda..."
cargo publish -p postrust-lambda
echo "Publishing postrust-worker..."
cargo publish -p postrust-worker

echo ""
echo "All crates published successfully!"
