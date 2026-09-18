# syntax=docker/dockerfile:1
#
# Multi-stage image for gha-indie-worker-web-server.
# Prefer linux/arm64 — both clusters are aarch64 (Graviton on AWS/EC2, CAX on
# Hetzner), and building natively there beats emulating amd64 through QEMU.
# Nothing here is arm-specific: pass a --platform list for a multi-arch index.
#
#   docker buildx build --platform linux/arm64 -t gha-indie-worker-web-server:dev .
#   docker run --rm --platform linux/arm64 \
#     -e SOPS_AGE_KEY="$(cat ~/.config/sops/age/keys.txt)" gha-indie-worker-web-server:dev
#
# This crate resolves a dependency from a git remote. If that remote is
# private, pass a token as a BuildKit secret; it is tmpfs-mounted for the
# duration of one RUN and applied through process-scoped GIT_CONFIG_*
# variables, so it never reaches ~/.gitconfig, an image layer or
# `docker history` — unlike --build-arg:
#
#   GH_TOKEN="$(gh auth token)" docker buildx build \
#     --platform linux/arm64 --secret id=gh_token,env=GH_TOKEN -t ... .
#
# ores-sops boundary (https://github.com/ORESoftware/ores-sops):
# - never copy plaintext OR ciphertext environment material into an image layer;
#   env/enc/<name>.env.enc is committed to the repository but stays OUT of the
#   build context (.dockerignore), and env/dec/<name>.env is gitignored;
# - mount ciphertext read-only at /run/secrets/app.env at runtime;
# - supply the age key through SOPS_AGE_KEY_FILE (preferred) or SOPS_AGE_KEY;
# - the entrypoint decrypts only in process memory before exec'ing the server;
# - orchestrator env (including OTEL_*) wins over decrypted secrets.
#
# ores-otel (https://github.com/ores-otel):
#   The app exports OTLP in-process. Default collector is
#   dd-otel-collector.observability.svc.cluster.local (HTTP/protobuf :4318).
#   Wrong port silently drops spans. Do not EXPOSE 4317/4318.
#   The *-sidecar.rs image is a separate loopback probe helper on
#   127.0.0.1:9090 — do not bake it into this image, do not publish :9090.

############################
# Stage 1 — build + strip
############################
FROM rust:1.90-bookworm AS build
ARG TARGETARCH
# git: one dependency is resolved from a git remote. pkg-config/libssl-dev,
# build-essential, cmake and perl cover native builds (aws-lc-sys, ring,
# openssl-sys) pulled in transitively.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
         git ca-certificates pkg-config libssl-dev build-essential cmake perl \
    && apt-get clean \
    && find /var/lib/apt/lists -mindepth 1 -delete
WORKDIR /src
COPY . .
# /src/target is a BuildKit cache mount, so nothing under it survives this RUN:
# the stripped binary is copied to /usr/local/bin, and that is what stage 2 takes.
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git,sharing=locked \
    --mount=type=cache,target=/src/target,id=gha-indie-worker-web-server-target-${TARGETARCH},sharing=locked \
    --mount=type=secret,id=gh_token \
    set -eu; \
    if [ -s /run/secrets/gh_token ]; then \
      t="$(cat /run/secrets/gh_token)"; \
      export GIT_CONFIG_COUNT=2; \
      export GIT_CONFIG_KEY_0="url.https://x-access-token:${t}@github.com/.insteadOf"; \
      export GIT_CONFIG_VALUE_0="https://github.com/"; \
      export GIT_CONFIG_KEY_1="url.https://x-access-token:${t}@github.com/.insteadOf"; \
      export GIT_CONFIG_VALUE_1="ssh://git@github.com/"; \
    fi; \
    cargo build --release --bin gha-indie-worker-web-server; \
    strip "target/release/gha-indie-worker-web-server"; \
    cp "target/release/gha-indie-worker-web-server" "/usr/local/bin/gha-indie-worker-web-server"

############################
# Stage 2 — slim runtime + sops
############################
FROM debian:bookworm-slim AS runtime
# ca-certificates: the system trust store is needed to reach Postgres, upstream
# HTTP and the OTLP collector over TLS. libssl3 covers a transitive
# openssl-sys that links dynamically.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates libssl3 \
    && apt-get clean \
    && find /var/lib/apt/lists -mindepth 1 -delete \
    && useradd --system --uid 65532 --no-create-home --shell /usr/sbin/nologin app

# sops is copied from its own published multi-arch image rather than curled and
# checksummed: the right architecture is selected automatically for whatever
# --platform this image is built for, and there is no per-arch SHA table to
# keep current. sops is a static Go binary, so the alpine-built one runs here.
COPY --from=build "/usr/local/bin/gha-indie-worker-web-server" "/usr/local/bin/gha-indie-worker-web-server"
COPY --from=ghcr.io/getsops/sops:v3.10.2-alpine --chmod=0755 /usr/local/bin/sops /usr/local/bin/sops
COPY --chmod=0755 scripts/sops-entrypoint.sh /usr/local/bin/sops-entrypoint.sh

# The bind address is set explicitly because the service's own default is a
# LOOPBACK address, which inside a container accepts no traffic from outside
# the network namespace: the pod would pass its own liveness probe and refuse
# every request arriving through the Service.
ENV GHA_INDIE_WORKER_WEB_BIND=0.0.0.0:8080 \
    SOPS_SECRETS_FILE=/run/secrets/app.env \
    HOME=/tmp \
    OTEL_SERVICE_NAME=gha-indie-worker-web-server \
    OTEL_EXPORTER_OTLP_ENDPOINT=http://dd-otel-collector.observability.svc.cluster.local:4318 \
    RUST_LOG=info
EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/sops-entrypoint.sh"]
CMD ["/usr/local/bin/gha-indie-worker-web-server"]
