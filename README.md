# gha-indie-worker-web-server.rs

The **MASH** web server for [indiebuild.dev](https://indiebuild.dev) — Maud +
Axum + Supabase/SeaORM + HTMX. One process, four public hosts, no React, no JSX,
no bundler, and exactly one self-hosted script.

```
                       Cloudflare (the only trusted proxy)
                                    │
      ┌──────────────┬──────────────┼──────────────┬──────────────┐
 app.indiebuild  user.indiebuild  org.indiebuild  m.indiebuild   (anything else)
      │              │              │              │              │
      └──────────────┴──────────────┴──────────────┴──────────────┘
                                    │
                    gha-indie-worker-web-server  (one Cloud Run service)
                                    │
        ┌───────────────┬───────────┴────────┬──────────────────┐
   SeaORM (read)   api-server (write)    /ws relay          NATS subjects
  DATABASE_URL_     GHA_INDIE_WORKER_    → api /v1/ws     giw.<env>.<d>.<e>
    CANONICAL          API_HTTP_BASE
```

## Surfaces

| host | surface | what it is |
|---|---|---|
| `app.indiebuild.dev` | `App` | marketing-continuous home; the product dashboard once signed in |
| `user.indiebuild.dev` | `User` | B2C: sign up, sign in, personal workspace, API tokens, TOTP/passkeys |
| `org.indiebuild.dev` | `Org` | B2B: organization sign-in, the onboarding wizard, members/roles/seats/audit/SSO |
| `m.indiebuild.dev` | `Mobile` | the same pages in a compact shell with a bottom navigation bar |
| anything else | `Unknown` | a 404 page with no product chrome, on every path |

The signed-out `app.` home has exactly two entry points, stated plainly:
**Sign in to your organization** → `https://org.indiebuild.dev/login`, and
**Personal account** → `https://user.indiebuild.dev/login`.

`Host` is resolved once per request in `src/middleware.rs`; `X-Forwarded-Host` is
honoured **only** when the peer address is inside a trusted-proxy CIDR
(Cloudflare by default). An unknown host is never quietly treated as `app.`.

## The four avenues

1. **Direct read-only SeaORM** (`DATABASE_URL_CANONICAL`, feature `db`) —
   `src/data/db.rs`. Reads only: the module refuses any statement that is not a
   `SELECT`, and this service never runs DDL.
2. **Stateless HTTP** (`GHA_INDIE_WORKER_API_HTTP_BASE`) — `src/data/api_client.rs`.
   Every write, with the actor's bearer forwarded so authorization is decided
   once, in the api-server.
3. **Stateful TCP** (`GHA_INDIE_WORKER_WEB_TCP_BIND`, feature `tcp-transport`) —
   length-prefixed JSON frames for operator tooling — **and WebSocket** on the
   HTTP port: `/ws` relays to the api-server's `/v1/ws` for live run logs and
   chat (`src/ws/`, feature `ws-relay`).
4. **Async NATS/JetStream** (`GHA_INDIE_WORKER_NATS_URL`, feature
   `nats-transport`) — subject algebra `giw.<env>.<domain>.<event>`.

`src/transport/mod.rs::choose` is the whole policy, as an exhaustive match.

## Auth

Two credentials, one `Actor`:

* the **host-scoped cookie session** minted by the shared-auth code exchange
  (`src/session.rs`), and
* an **`Authorization: Bearer …`** on API-ish htmx calls — either a shared-auth
  token (introspected) or a Supabase/Neon session JWT verified against cached
  JWKS (`src/auth.rs`, `src/auth/jwks.rs`).

The session cookie carries **no `Domain` attribute**. `app.`, `user.` and `org.`
are separate origins, and the org surface holds seat, billing and role
authority — a session minted on `user.` must not be replayable there. Signing in
on two surfaces means completing the exchange twice, once per origin.

Admin-instance credentials are rejected outright: this process holds the product
audience and the product introspection secret only.

## Security posture

* **CSP per request**: `script-src 'self' 'nonce-…'`, no `unsafe-inline`, no
  `unsafe-eval`, no CDN. The nonce is minted per request and appears on exactly
  two tags — the inline stylesheet and the self-hosted htmx script.
* **CSRF**: double-submit, HMAC-bound to the session subject. The shell puts the
  token on `<body hx-headers>` so htmx sends it automatically; forms carry a
  hidden field for the no-JavaScript path. Unsafe methods without a valid token
  are rejected in middleware, before any handler sees the body.
* **Rate limiting**: `ores-rl-lib-core`'s deterministic state machine over
  opaque HMAC keys — no raw IP, subject, email or token ever reaches limiter
  state (`src/rate_limit.rs`).
* **Cache-Control**: `private, no-store` for pages (they carry a per-user token);
  `public, max-age=31536000, immutable` for `/assets/releases/{hash}/…`.

## Assets

Nothing is loaded from a CDN. `scripts/vendor-assets.sh` fetches htmx into
`assets/vendor/htmx.min.js` (idempotent; safe to run in a Docker layer or in
CI). `ores-web-loader` is served from `/assets/loader/`, and content-addressed
release bundles from `/assets/releases/{hash}/…` out of
`GHA_INDIE_WORKER_ASSETS_DIR`.

Leptos/Dioxus **islands** are behind the `islands` cargo feature, **off by
default**, so the server builds and tests without a wasm toolchain. Island mounts
always contain server-rendered fallback content. Marketing pages emit
`<link rel="prefetch">` for the release **only on intent** — an htmx
`hx-trigger="mouseenter once, focus once"` on the "Open app" control, so hover
and keyboard focus get the same head start and a reader who never clicks
downloads nothing.

## Running it

```console
cp .env.example .env            # development values only; nothing secret
scripts/vendor-assets.sh        # fetch htmx into assets/vendor/
cargo run                       # http://127.0.0.1:8081
```

`GHA_INDIE_WORKER_DEV_SURFACE` decides which surface `localhost` serves, so one
local process can be any of the four. To exercise host routing locally, send the
header directly:

```console
curl -s -H 'Host: org.indiebuild.dev' http://127.0.0.1:8081/login
curl -s -H 'Host: m.indiebuild.dev'   http://127.0.0.1:8081/ | grep 'body class'
```

## Tests

```console
cargo test                                  # unit + HTTP surface
cargo check --no-default-features           # no database, no relay
cargo check --features islands              # island pages
```

`tests/` drives the real router with `tower::ServiceExt::oneshot` and needs no
network, no database and no shared-auth: `tests/host_routing.rs` (surfaces, CSP,
security headers), `tests/csrf.rs`, `tests/onboarding_snapshots.rs` (every wizard
step) and `tests/mobile_layout.rs`.

## Layout

```
src/
  hosts/      mod.rs (Host → Surface, dispatch) + app|user|org|mobile|unknown|common
  ui/         layout, nav, components, forms, tables, loader, chat_widget, styles
  data/       api_client (writes), db (reads, feature `db`), models, rows
  ws/         relay (feature `ws-relay`)
  transport/  the four avenues, and the rule for choosing between them
  auth.rs  session.rs  csrf.rs  middleware.rs  rate_limit.rs  telemetry.rs  chat.rs
```
