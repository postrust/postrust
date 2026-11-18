# Build stage. The toolchain rust-toolchain.toml pins, on the Debian release
# the runtime stage uses, so the binary links against the same glibc. Not
# nightly: an unpinned nightly broke the 3.1.0 image on a compiler regression
# in code that stable builds, and nothing here needs it.
FROM rust:1.98.1-bookworm AS builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary with the admin UI and OpenTelemetry. `otel` is
# inactive until OTEL_EXPORTER_OTLP_ENDPOINT or PGRST_ADMIN_SERVER_PORT is set;
# see docs/observability.md.
RUN cargo build --release --locked -p postrust-server --features admin-ui,otel

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
