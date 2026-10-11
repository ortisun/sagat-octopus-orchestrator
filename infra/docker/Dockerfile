# syntax=docker/dockerfile:1.7
# Images of the three processes, built from one workspace. Build from the repository root:
#   docker build -f infra/docker/Dockerfile --target control-plane   -t sagat-octopus-orchestrator/control-plane:dev .
#   docker build -f infra/docker/Dockerfile --target execution-engine -t sagat-octopus-orchestrator/execution-engine:dev .
#   docker build -f infra/docker/Dockerfile --target sandbox-runner   -t sagat-octopus-orchestrator/sandbox-runner:dev .
#   docker build -f infra/docker/Dockerfile --target dev-idp          -t sagat-octopus-orchestrator/dev-idp:dev .
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
        -p sagat-sandbox-runner --bin sandbox-runner \
        -p sagat-dev-idp --bin dev-idp \
    && cp target/release/control-plane target/release/execution-engine target/release/sandbox-runner target/release/dev-idp /

# The Studio, built once and shipped as files: the control plane serves them only when told where
# they are (SAGAT_STUDIO_DIR), so an image that is not given the variable serves none.
FROM node:22-bookworm-slim AS studio
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json frontend/tsconfig.base.json frontend/.npmrc ./
COPY frontend/apps/studio/package.json apps/studio/package.json
COPY frontend/e2e/package.json e2e/package.json
RUN npm ci --ignore-scripts --no-audit --no-fund --workspace @sagat/studio
COPY frontend/apps/studio apps/studio
# The generated API types are committed; the build only needs the contract to have been read once.
COPY specs/001-platform-foundation/contracts specs/001-platform-foundation/contracts
RUN npm run build --workspace @sagat/studio

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

# The development identity provider: signs a token for anyone who asks, so it is a stage of its own
# and in no other image. It refuses to run unless SAGAT_ENV=dev. Its key is kept in /data so that
# tokens outlive a restart; the compose file mounts a volume there.
FROM runtime AS dev-idp
COPY --from=builder /dev-idp /usr/local/bin/dev-idp
RUN mkdir /data && chown 10001 /data
USER 10001
# The container is reached by the other containers of the stack, not by the host's network:
# compose publishes the port on the loopback of the host only.
ENV SAGAT_DEV_IDP_ADDR=0.0.0.0:9100 SAGAT_DEV_IDP_EXPOSED=true SAGAT_DEV_IDP_KEY_FILE=/data/key
EXPOSE 9100
HEALTHCHECK --interval=10s --timeout=4s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/dev-idp", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/dev-idp"]
CMD ["serve"]

# The sandbox runner: builds and runs the scripts of one tenant in containers of its own. It needs
# the container engine's socket, which makes it root-equivalent on its host: see
# docs/runbooks/scripts.md before running it anywhere that holds anything else. It is not started
# by default (compose profile `sandbox`).
FROM runtime AS sandbox-runner
COPY --from=builder /sandbox-runner /usr/local/bin/sandbox-runner
# The socket belongs to the engine's group; the compose file adds that group to this user.
USER 10001
ENV SAGAT_ADMIN_ADDR=127.0.0.1:9092
HEALTHCHECK --interval=10s --timeout=4s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/sandbox-runner", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/sandbox-runner"]
CMD ["serve"]

# The control plane (the default target: it is the last stage).
FROM runtime AS control-plane
COPY --from=builder /control-plane /usr/local/bin/control-plane
# The Studio's files; nothing serves them until SAGAT_STUDIO_DIR=/srv/studio is set.
COPY --from=studio /src/frontend/apps/studio/dist /srv/studio
USER 10001
ENV SAGAT_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
# The probe is the binary itself: the image has no shell tooling or curl.
HEALTHCHECK --interval=10s --timeout=4s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/control-plane", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/control-plane"]
CMD ["serve"]
