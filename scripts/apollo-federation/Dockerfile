FROM rust:1.88-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p postrust-server --features admin-ui

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/postrust /usr/local/bin/postrust

RUN useradd --create-home --uid 1000 postrust
USER postrust

EXPOSE 4001
CMD ["postrust"]
