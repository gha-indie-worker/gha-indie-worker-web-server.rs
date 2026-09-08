# GHA Indie Worker — web-server.rs

Canonical `web-server.rs` repository for [`gha-indie-worker`](https://github.com/gha-indie-worker).

- Internal runtimes: Rust, TypeScript, Dart.
- Contracts: JSON Schema in `gha-indie-worker-interfaces`.
- Auth: github.com/shared-auth.
- Sync: github.com/opto-sync.
- Telemetry: github.com/ores-otel.
- Flags: github.com/flags-2-env.
- Packages: github.com/zed-pkg.
- Never use React/JSX or webviews.
- Resolve git conflicts semantically; never rebase, stash, or reset.

## What this server is

The **MASH** surface for indiebuild.dev: Maud + Axum + Supabase/SeaORM + HTMX.
One process serves four hosts and routes on the `Host` header. See `README.md`
for the map; the rules below are the ones an agent must not break.

## Rules

### Rendering

- Every page is `maud::Markup`. No React, no JSX, no template language, no
  bundler, no server-side string concatenation of HTML.
- The **only** script the browser loads is the self-hosted
  `/assets/vendor/htmx.min.js`, plus `/assets/loader/ores-web-loader.js` on
  island pages. Never add a CDN reference, an inline event handler, or a
  third-party tag. CI fails the build if a page references a foreign origin.
- Everything goes through `ui::layout::page` (or `hosts::common::render`), which
  is what guarantees the CSP nonce and the CSRF token are present.
- Escape by construction: splice values through maud, never `PreEscaped` on
  anything that came from a request, a database row, or an upstream.

### Host routing

- `Host` is resolved once, in `middleware::request_context`, and travels in
  `RequestCtx`. Do not re-read the header in a handler.
- `X-Forwarded-Host` is honoured **only** behind a trusted-proxy CIDR.
- An unrecognised host is `Surface::Unknown` → 404 on every path, with no
  product chrome and no `/healthz`. Never widen this to "probably app.".

### Auth and sessions

- The session cookie is **host-scoped**: no `Domain` attribute, ever. `app.`,
  `user.` and `org.` are separate origins on purpose.
- `HttpOnly`, `SameSite=Lax`, `Secure` outside development.
- Admin roles and admin-instance audiences are rejected here. This server holds
  `SHARED_AUTH_BASE`/`SHARED_AUTH_AUDIENCE`/`SHARED_AUTH_INTROSPECT_SECRET` only;
  it must never learn `SHARED_AUTH_ADMIN_*`.
- Credentials never reach this server: sign-in hands off to shared-auth and
  returns through `/auth/callback`.

### CSRF

- Every state-changing route is covered. The middleware rejects a missing or
  forged token before a handler runs; a form fallback is re-checked in the
  handler with `RequestCtx::verify_form_csrf`.
- Open a `<form>` only through `ui::forms::form`, which carries the token.

### Data

- **Reads** may use the read-only SeaORM pool; **writes** always go to the
  api-server with the actor's bearer forwarded. This server never mints a
  privileged token of its own.
- `data::db` refuses any statement that is not a `SELECT`. Never run DDL, never
  add a migration here — migrations live in `gha-indie-worker-lib-core`.
- Direct `sqlx`/`tokio-postgres` are forbidden fleet-wide. SeaORM only, and all
  of its API surface stays inside `src/data/db.rs`.
- `src/data/rows.rs` mirrors what `gha-indie-worker-orm-core` will generate.
  When that repository exists, replace the row structs and the SQL constants
  with its entities — two files, no template changes.

### Configuration

- Env vars only, every one declared in `.cli-flags.toml`, loaded in
  `src/config.rs`, mirrored into `generated/` by flags-2-env. CI fails on an
  undeclared key.
- Never log a secret, a DSN, a token, a cookie, or a full URL with a query
  string. `Debug` impls for anything holding a secret are `finish_non_exhaustive`.
- Secrets come from `env/enc` → `env/dec` via ores-sops.

### Middleware

- ores-middleware is installed with this service's `AuthVerifier` and
  `RateLimiter` (`src/middleware.rs`). Keep the module names identical to the
  api-server: `middleware.rs`, `telemetry.rs`, `rate_limit.rs`, `auth.rs`.
- Rate-limit keys are opaque HMAC digests. A raw IP, subject, email or token must
  never reach limiter state, a log line, or a metric label.

### Islands and assets

- Islands are behind the `islands` feature, **default off**: the crate must build
  and test without a wasm toolchain. Every island mount carries server-rendered
  fallback content.
- Release prefetch happens **on intent only** (hover/focus of "Open app"), never
  on page load.
- Vendored assets are fetched by `scripts/vendor-assets.sh`, not committed.

### Tests

- Unit tests next to the module (`#[cfg(test)]`); HTTP surfaces in `tests/` with
  `tower::ServiceExt::oneshot`.
- Tests must not need the network, a database, or shared-auth. Use
  `AppState::for_tests` and `server::build_router`.
- A new surface, wizard step or route needs a test that pins its host, its status
  and the security markers on the page.

### Style

- Modular, no god-files. Functional core, effects at the edges. Typed errors with
  `thiserror`. Exhaustive matches. Explicit state machines for onboarding and run
  lifecycle. Illegal states unrepresentable.
- `#![forbid(unsafe_code)]` at the top of every module.
- Comments explain *why*, not *what*.

## Commit trailer

```
Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01MH6JaV2AGqtMgqrUmdCSJh
```
