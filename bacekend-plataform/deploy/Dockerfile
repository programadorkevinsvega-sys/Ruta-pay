FROM rust:1-bookworm AS builder
WORKDIR /workspace

# Cargo workspace metadata is shared at build time. The final image contains
# only the platform backend binary and does not run the passthrough backend.
COPY . .
RUN cargo build --release -p bacekend-plataform

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home payvia
COPY --from=builder /workspace/target/release/bacekend-plataform /usr/local/bin/bacekend-plataform
USER 10001:10001
EXPOSE 8080
EXPOSE 9090
ENTRYPOINT ["/usr/local/bin/bacekend-plataform"]
