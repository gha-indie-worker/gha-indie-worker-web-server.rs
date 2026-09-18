# syntax=docker/dockerfile:1
#
# Multi-stage image for gha-indie-worker-web-server.
# Prefer linux/arm64:
#   docker buildx build --platform linux/arm64 -t gha-indie-worker-web-server:dev .
#   docker run --rm --platform linux/arm64 \
#     -e SOPS_AGE_KEY="$(cat ~/.config/sops/age/keys.txt)" gha-indie-worker-web-server:dev
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
WORKDIR /src
COPY . .
# /src/target is a BuildKit cache mount, so nothing under it survives this RUN:
# the stripped binary is copied to /usr/local/bin, and that is what stage 2 takes.
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git,sharing=locked \
    --mount=type=cache,target=/src/target,id=gha-indie-worker-web-server-target-${TARGETARCH},sharing=locked \
    cargo build --release --bin gha-indie-worker-web-server \
    && strip "target/release/gha-indie-worker-web-server" \
    && cp "target/release/gha-indie-worker-web-server" "/usr/local/bin/gha-indie-worker-web-server"

############################
# Stage 2 — slim runtime + sops
############################
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && apt-get clean \
    && find /var/lib/apt/lists -mindepth 1 -delete \
    && useradd --system --uid 65532 --no-create-home --shell /usr/sbin/nologin app

COPY --from=build "/usr/local/bin/gha-indie-worker-web-server" "/usr/local/bin/gha-indie-worker-web-server"
COPY --from=ghcr.io/getsops/sops:v3.10.2-alpine --chmod=0755 /usr/local/bin/sops /usr/local/bin/sops
COPY --chmod=0755 scripts/sops-entrypoint.sh /usr/local/bin/sops-entrypoint.sh

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
