# gha-indie-worker-web-server.rs

Rust web server (Axum/Maud/HTMX). Four API avenues live in `src/transport`.

## Surfaces

One binary, five host-routed surfaces off `GHA_INDIE_WORKER_APEX` (default `indiebuild.dev`):

| host | what it is |
|---|---|
| `www.` / apex | the landing page: two equal doors, "for your team" and "for yourself" |
| `user.` | B2C — individual sign-up, magic-link sign-in, personal settings |
| `org.` | B2B — organization sign-in, seats, invitations, members, domain verification |
| `app.` | the signed-in shell — runs, run detail with a live log tail, runners, settings |
| `m.` | the mobile shell — same data, bottom navigation, no live island |

`admin.` and `admin-api.` answer **404** here. That is `lib_core::runtime::surface::dispose`'s
decision, honoured in `pages::show` and `pages::accept_post` and nowhere else.

Maud templates, htmx for fragment swaps, no React and no JSX. htmx is vendored and served from our
own origin — never a CDN — by `scripts/vendor-htmx.py`, and its SHA-384 pin is committed and
checked in CI. Every form works without it.

```
python3 scripts/vendor-htmx.py     # once, then commit assets/htmx.min.js{,.sha384}
cargo test --all-targets
cargo run
```
