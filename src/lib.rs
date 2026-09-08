#![forbid(unsafe_code)]

//! `gha-indie-worker-web-server` — the MASH surface for **indiebuild.dev**.
//!
//! MASH = **M**aud + **A**xum + **S**upabase/SeaORM + **H**TMX. There is no
//! React, no JSX, no bundler and no third-party inline script: every page is
//! rendered server-side into [`maud::Markup`] and progressively enhanced by one
//! self-hosted copy of htmx at `/assets/vendor/htmx.min.js`.
//!
//! One process serves four public hosts and routes on the `Host` header
//! (Cloudflare is the only trusted proxy):
//!
//! | host | [`Surface`] | audience |
//! |---|---|---|
//! | `app.indiebuild.dev` | [`Surface::App`] | marketing-continuous home + product dashboard |
//! | `user.indiebuild.dev` | [`Surface::User`] | B2C signup/login, personal workspace |
//! | `org.indiebuild.dev` | [`Surface::Org`] | B2B org login, onboarding wizard, org admin |
//! | `m.indiebuild.dev` | [`Surface::Mobile`] | compact layouts, bottom navigation |
//!
//! Anything else resolves to [`Surface::Unknown`] and is answered with a 404
//! page — an unrecognised `Host` never falls through to a product surface.
//!
//! Four interaction avenues, exactly as the fleet contract requires:
//!
//! 1. read-only SeaORM against `DATABASE_URL_CANONICAL` ([`data::db`], feature `db`);
//! 2. stateless HTTP to the api-server for every write ([`data::api_client`]);
//! 3. stateful TCP ([`transport::tcp`]) plus WebSocket relay on the HTTP port ([`ws`]);
//! 4. async NATS/JetStream subjects ([`transport::nats`]).

pub mod auth;
pub mod chat;
pub mod config;
pub mod csrf;
pub mod data;
pub mod error;
pub mod hosts;
pub mod middleware;
pub mod pages;
pub mod persistence;
pub mod rate_limit;
pub mod releases;
pub mod server;
pub mod session;
pub mod state;
pub mod telemetry;
pub mod transport;
pub mod ui;
pub mod ws;

pub use config::WebConfig;
pub use error::WebError;
pub use hosts::Surface;
pub use state::AppState;

/// Service name reported to ores-middleware, ores-otel and Cloud Run.
pub const SERVICE_NAME: &str = "gha-indie-worker-web-server";
/// Telemetry namespace shared by every server in the org.
pub const SERVICE_NAMESPACE: &str = "gha-indie-worker";
