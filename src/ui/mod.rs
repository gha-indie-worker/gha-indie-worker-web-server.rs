#![forbid(unsafe_code)]

//! The maud layout system.
//!
//! Every page in this crate is one [`maud::Markup`] tree. There is no React, no
//! JSX, no build step and no third-party inline script. The only JavaScript the
//! browser loads is a self-hosted copy of htmx at `/assets/vendor/htmx.min.js`
//! plus, on island pages, the ores-web-loader module — both same-origin, both
//! nonce-tagged, both allowed by the per-request Content-Security-Policy.
//!
//! * [`layout`] — the shared shell (head, CSP nonce, htmx wiring, theme).
//! * [`nav`] — per-surface navigation; mobile gets a bottom bar, never hover.
//! * [`components`] — cards, badges, alerts, step indicators, log streams.
//! * [`forms`] — form scaffolding that always carries the CSRF token.
//! * [`tables`] — the one table shape used by runs, workers, members and audit.
//! * [`loader`] — ores-web-loader script/preload/prefetch tags.
//! * [`chat_widget`] — the ores-chat visitor and customer widgets.

pub mod chat_widget;
pub mod components;
pub mod forms;
pub mod layout;
pub mod loader;
pub mod nav;
pub mod styles;
pub mod tables;

pub use layout::{page, PageContext};
