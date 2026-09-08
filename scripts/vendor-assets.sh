#!/usr/bin/env bash
# Vendors the third-party browser assets this server self-hosts.
#
# Nothing in this product loads a script from a CDN: the Content-Security-Policy
# is `script-src 'self' 'nonce-…'`, so every executable byte has to be served
# from our own origin. This script is what puts it there.
#
# It is idempotent: a file whose SHA-256 already matches is left alone, so
# running it twice writes nothing and running it in a Docker layer stays cached.
#
#   scripts/vendor-assets.sh              # into ./assets
#   ASSETS_DIR=/srv/assets scripts/vendor-assets.sh
#   scripts/vendor-assets.sh --check      # verify only, non-zero if stale
set -euo pipefail

ASSETS_DIR="${ASSETS_DIR:-assets}"
HTMX_VERSION="${HTMX_VERSION:-2.0.4}"
HTMX_URL="https://unpkg.com/htmx.org@${HTMX_VERSION}/dist/htmx.min.js"
HTMX_PATH="${ASSETS_DIR}/vendor/htmx.min.js"

CHECK_ONLY=0
if [[ "${1:-}" == "--check" ]]; then
  CHECK_ONLY=1
fi

log() { printf '[vendor-assets] %s\n' "$*"; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

fetch() {
  local url="$1" destination="$2"
  local directory temporary
  directory="$(dirname "$destination")"
  mkdir -p "$directory"
  temporary="$(mktemp "${directory}/.vendor.XXXXXX")"
  # -f: fail on HTTP error rather than writing an error page to disk.
  if ! curl -fsSL "$url" -o "$temporary"; then
    rm -f "$temporary"
    log "ERROR: could not fetch $url"
    return 1
  fi
  if [[ ! -s "$temporary" ]]; then
    rm -f "$temporary"
    log "ERROR: $url returned an empty body"
    return 1
  fi
  if [[ -f "$destination" ]] && [[ "$(sha256_of "$temporary")" == "$(sha256_of "$destination")" ]]; then
    rm -f "$temporary"
    log "unchanged  $destination"
    return 0
  fi
  if [[ "$CHECK_ONLY" == "1" ]]; then
    rm -f "$temporary"
    log "STALE      $destination"
    return 1
  fi
  mv "$temporary" "$destination"
  chmod 0644 "$destination"
  log "wrote      $destination ($(sha256_of "$destination"))"
}

mkdir -p "${ASSETS_DIR}/vendor" "${ASSETS_DIR}/loader" "${ASSETS_DIR}/releases"

log "htmx ${HTMX_VERSION} -> ${HTMX_PATH}"
fetch "$HTMX_URL" "$HTMX_PATH"

# ores-web-loader is a fleet artifact, not a public package. It is published into
# the release bundle by the ores-web-loader pipeline and copied in by the deploy;
# this script only makes sure the directory exists so the static mount has
# somewhere to serve from and the page's <link rel="modulepreload"> resolves.
if [[ ! -f "${ASSETS_DIR}/loader/ores-web-loader.js" ]]; then
  log "note       ${ASSETS_DIR}/loader/ores-web-loader.js is absent (islands will not hydrate)"
fi

log "done"
