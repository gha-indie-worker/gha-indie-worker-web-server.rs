#![forbid(unsafe_code)]
//! `gha-indie-worker-web-server` — one binary, five host-routed surfaces.
//!
//! | host | surface | what it is |
//! |---|---|---|
//! | `www.` / apex | marketing | two equal doors: for your team, for yourself |
//! | `user.` | B2C | individual sign-up, magic-link sign-in, personal settings |
//! | `org.` | B2B | organization sign-in, seats, invitations, members, domains |
//! | `app.` | application | runs, run detail with a live log tail, runners, settings |
//! | `m.` | mobile | the same application, small-screen chrome, no live island |
//!
//! `admin.` and `admin-api.` resolve to surfaces this binary answers **404** for. That decision is
//! `lib_core::runtime::surface::dispose`'s, made once, and honoured in exactly two places:
//! [`pages::show`] and [`pages::accept_post`].
//!
//! ## How the crate is arranged
//!
//! Modules split along one line: **pure** modules import nothing outside `std` and are compiled
//! and tested standalone with `rustc --edition 2021 --test`; **bound** modules are the ones that
//! touch axum, the clock, the network or randomness.
//!
//! | pure | what it decides |
//! |---|---|
//! | [`csrf`] | the CSRF check, cookie parsing, form decoding, constant-time comparison |
//! | [`present`] | which page a request means, and what every refusal says to a human |
//! | [`policy`] | the CSP and every other security header |
//! | [`wire`] | the WebSocket JSON codec |
//! | [`sri`] | SHA-384 and base64, for subresource integrity |
//!
//! [`bridge`] is the seam: it converts lib-core's enums into the mirrors the pure modules speak,
//! with a total `match` in every direction, so a new variant upstream is a compile error here
//! rather than a missing message in a browser.

pub mod assets;
pub mod auth;
pub mod bridge;
pub mod config;
pub mod csrf;
pub mod error;
pub mod pages;
pub mod persistence;
pub mod policy;
pub mod present;
pub mod server;
pub mod sri;
pub mod state;
pub mod transport;
pub mod wire;
pub mod ws;

pub use error::WebError;
pub use server::run;
pub use state::AppState;

/// The service name reported to `ores-middleware` and `ores-otel`. It must match
/// `ORES_OTEL_SERVICE_NAME` in `gcp/cloudrun/services.tf`.
pub const SERVICE: &str = "gha-indie-worker-web-server";
