# HTTP Garden target for postrust-proxy, in the "transducer" role: it accepts a
# request on :80, forwards it to the Garden's echo server, and the Garden
# compares what arrived against what was sent.
#
# Unlike the upstream Garden images, this one does NOT clone from APP_REPO. The
# point of running it locally is to test the working tree, including uncommitted
# changes, so setup.sh snapshots the source into ./src and this COPYs it.
FROM http-garden-soil:latest

# cmake and a C toolchain are for aws-lc-sys, which rustls pulls in. Even a
# --no-default-features build needs it: rustls is a non-optional dependency of
# postrust-proxy, only rustls-acme is behind the `acme` feature.
RUN apt -y update \
 && apt -y install --no-install-recommends curl cmake \
 && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable

COPY src /app/postrust

# The soil image exports CFLAGS/LDFLAGS carrying ASan and UBSan, which the C
# builds inside aws-lc-sys do not survive. Drop them for the cargo build, the
# same way the Garden's own pingora target does.
#
# --no-default-features drops the `acme` feature: the Garden exercises HTTP
# parsing and forwarding, and nothing here talks to a certificate authority.
RUN cd /app/postrust \
 && . ~/.cargo/env \
 && unset CC CXX CFLAGS CXXFLAGS LDFLAGS \
 && cargo build --release -p postrust-proxy --bin postrust-proxy --no-default-features \
 && cp target/release/postrust-proxy /usr/local/bin/postrust-proxy \
 && rm -rf target ~/.cargo/registry

COPY postrust-proxy.toml /app/postrust-proxy.toml

ARG BACKEND_HOST=127.0.0.1
ARG BACKEND_PORT=56062
RUN sed -i -e "s/BACKEND_HOST_PLACEHOLDER/$BACKEND_HOST/g" \
           -e "s/BACKEND_PORT_PLACEHOLDER/$BACKEND_PORT/g" /app/postrust-proxy.toml

COPY start.sh /app/start.sh
RUN chmod +x /app/start.sh
CMD ["/app/start.sh"]
