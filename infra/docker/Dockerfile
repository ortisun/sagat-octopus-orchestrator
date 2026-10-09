# syntax=docker/dockerfile:1.7
# Images of the two processes, built from one workspace. Build from the repository root:
#   docker build -f infra/docker/Dockerfile --target control-plane   -t sagat-octopus-orchestrator/control-plane:dev .
#   docker build -f infra/docker/Dockerfile --target execution-engine -t sagat-octopus-orchestrator/execution-engine:dev .
# Without --target the last stage (the control plane) is built.

ARG RUST_VERSION=1.98.0

FROM rust:${RUST_VERSION}-slim-bookworm AS builder
WORKDIR /src
# The whole workspace is needed to resolve member manifests; the lock file keeps the build exact.
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates crates
COPY apps apps
COPY tests tests
# The contracts are compiled into the binaries: the workflow IR schema is embedded by the codec and
# the worker protocol is generated from worker.proto at build time.
COPY specs/001-platform-foundation/contracts specs/001-platform-foundation/contracts
# The cache mount below is shared by every checkout that builds this Dockerfile on the host, and
# cargo decides that a workspace crate or a build script is up to date by comparing modification
# times. COPY keeps the times of the files in the context, which can be older than what another
# checkout left in the cache: the build would then reuse its generated code (a worker.proto of
# another branch) or its compiled crates. Touching the sources makes the workspace crates rebuild;
# dependencies are identified by checksum and stay cached.
RUN find crates apps tests specs rust-toolchain.toml Cargo.toml Cargo.lock -type f -exec touch {} +
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
        -p sagat-control-plane --bin control-plane \
        -p sagat-execution-engine --bin execution-engine \
    && cp target/release/control-plane target/release/execution-engine /

FROM debian:bookworm-slim AS runtime
# CA certificates are needed to reach the OIDC provider's JWKS endpoint over HTTPS.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin sagat

# The execution engine: the worker gRPC API, the scheduler loop and the outbox dispatcher.
FROM runtime AS execution-engine
COPY --from=builder /execution-engine /usr/local/bin/execution-engine
USER 10001
ENV SAGAT_GRPC_ADDR=0.0.0.0:9000
EXPOSE 9000
# The probe is the binary itself (GET /livez on the loopback admin listener): no shell tooling.
HEALTHCHECK --interval=10s --timeout=4s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/execution-engine", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/execution-engine"]
CMD ["serve"]

# The control plane (the default target: it is the last stage).
FROM runtime AS control-plane
COPY --from=builder /control-plane /usr/local/bin/control-plane
USER 10001
ENV SAGAT_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
# The probe is the binary itself: the image has no shell tooling or curl.
HEALTHCHECK --interval=10s --timeout=4s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/control-plane", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/control-plane"]
CMD ["serve"]
