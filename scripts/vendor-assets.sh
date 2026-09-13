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
# The bytes are pinned. htmx is fetched from unpkg over TLS and refused unless its
# SHA-384 matches HTMX_SHA384, which is committed here next to the version. A
# compromised registry can therefore at worst stop a build, never land a script.
# Bumping htmx is a deliberate two-line diff (HTMX_VERSION + HTMX_SHA384) a
# reviewer can see; overriding the version without also overriding the pin fails.
#
#   scripts/vendor-assets.sh              # into ./assets
#   ASSETS_DIR=/srv/assets scripts/vendor-assets.sh
#   scripts/vendor-assets.sh --check      # verify only, non-zero if stale
set -euo pipefail

ASSETS_DIR="${ASSETS_DIR:-assets}"
PINNED_HTMX_VERSION="2.0.6"
PINNED_HTMX_SHA384="Akqfrbj/HpNVo8k11SXBb6TlBWmXXlYQrCSqEWmyKJe+hDm3Z/B2WVG4smwBkRVm"
HTMX_VERSION="${HTMX_VERSION:-$PINNED_HTMX_VERSION}"
if [[ "$HTMX_VERSION" == "$PINNED_HTMX_VERSION" ]]; then
  HTMX_SHA384="${HTMX_SHA384:-$PINNED_HTMX_SHA384}"
elif [[ -z "${HTMX_SHA384:-}" ]]; then
  printf '[vendor-assets] ERROR: HTMX_VERSION=%s needs a matching HTMX_SHA384 pin\n' "$HTMX_VERSION" >&2
  exit 1
fi
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

sha384_b64_of() {
  openssl dgst -sha384 -binary "$1" | openssl base64 -A
}

fetch() {
  local url="$1" destination="$2" expected_sha384="$3"
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
  if [[ "$(sha384_b64_of "$temporary")" != "$expected_sha384" ]]; then
    rm -f "$temporary"
    log "ERROR: $url does not match its pinned sha384-${expected_sha384}"
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
fetch "$HTMX_URL" "$HTMX_PATH" "$HTMX_SHA384"

# ores-web-loader is a fleet artifact, not a public package. It is published into
# the release bundle by the ores-web-loader pipeline and copied in by the deploy;
# this script only makes sure the directory exists so the static mount has
# somewhere to serve from and the page's <link rel="modulepreload"> resolves.
if [[ ! -f "${ASSETS_DIR}/loader/ores-web-loader.js" ]]; then
  log "note       ${ASSETS_DIR}/loader/ores-web-loader.js is absent (islands will not hydrate)"
fi

log "done"
