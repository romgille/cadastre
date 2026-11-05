# syntax=docker/dockerfile:1

# ---- build ------------------------------------------------------------------
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# --locked: the build must use exactly the dependency versions in Cargo.lock.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/cadastre /cadastre

# ---- runtime ----------------------------------------------------------------
# distroless/cc: glibc and libgcc, no shell, no package manager. The `nonroot`
# variant runs as uid 65532; override with `user:` to match the owner of the
# output folder (see compose.example.yaml).
FROM gcr.io/distroless/cc-debian12:nonroot

ARG VERSION=dev
LABEL org.opencontainers.image.title="cadastre" \
      org.opencontainers.image.description="Keeps a folder of Markdown up to date with what your Docker containers are running." \
      org.opencontainers.image.source="https://github.com/romgille/cadastre" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.version="${VERSION}"

COPY --from=build /cadastre /cadastre

# Sensible container defaults. The Docker URL points at a read-only socket
# proxy, never at the Docker socket itself.
ENV CADASTRE_DOCKER_URL=http://socket-proxy:2375 \
    CADASTRE_OUT_DIR=/out \
    CADASTRE_STATE_DIR=/state

# Exec form: there is no shell in this image. The check reads the heartbeat
# file, so it fails if the loop is stuck or if the last run failed.
HEALTHCHECK --interval=60s --timeout=5s --start-period=60s --retries=3 \
    CMD ["/cadastre", "healthcheck"]

ENTRYPOINT ["/cadastre"]
CMD ["run"]
