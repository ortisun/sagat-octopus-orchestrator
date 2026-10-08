# syntax=docker/dockerfile:1.7
# Control plane image. Build from the repository root:
#   docker build -f infra/docker/Dockerfile -t sagat-octopus-orchestrator/control-plane:dev .

ARG RUST_VERSION=1.98.0

FROM rust:${RUST_VERSION}-slim-bookworm AS builder
WORKDIR /src
# The whole workspace is needed to resolve member manifests; the lock file keeps the build exact.
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates crates
COPY apps apps
COPY tests tests
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p sagat-control-plane --bin control-plane \
    && cp target/release/control-plane /control-plane

FROM debian:bookworm-slim AS runtime
# CA certificates are needed to reach the OIDC provider's JWKS endpoint over HTTPS.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin sagat
COPY --from=builder /control-plane /usr/local/bin/control-plane
USER 10001
ENV SAGAT_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
# The probe is the binary itself: the image has no shell tooling or curl.
HEALTHCHECK --interval=10s --timeout=4s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/control-plane", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/control-plane"]
CMD ["serve"]
