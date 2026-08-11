# Build stage
FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary with admin-ui feature
RUN cargo build --release -p postrust-server --features admin-ui

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /app/target/release/postrust /usr/local/bin/postrust

# Create non-root user
RUN useradd -m -u 1000 postrust
USER postrust

# Expose port
EXPOSE 3000

# The server binds 127.0.0.1 by default, which inside a container means a
# published port reaches nothing -- the container looks healthy from the inside
# and is unreachable from the outside.
ENV PGRST_SERVER_HOST=0.0.0.0

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/_/health || exit 1

# Run the server
CMD ["postrust"]
