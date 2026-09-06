#![forbid(unsafe_code)]
//! The one error type every handler returns, and the only place a status code is chosen.
//!
//! Refusals are deliberately coarse to the client and precise to the log. `Unauthenticated` and
//! `Forbidden` both render the same page on a surface a stranger can reach, because "this exists
//! but you may not see it" is itself an answer.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use maud::html;
use thiserror::Error;

use crate::csrf::CsrfError;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("csrf: {0:?}")]
    Csrf(CsrfError),
    /// A form that could not be parsed at all — not a validation failure, which is rendered next
    /// to the field instead.
    #[error("bad request: {0}")]
    BadRequest(&'static str),
    #[error("a dependency is unavailable: {0}")]
    Unavailable(&'static str),
    #[error("internal error")]
    Internal,
}

impl WebError {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            // An unauthenticated browser is redirected before it reaches a handler; reaching one
            // means an API-shaped request, so it gets a status rather than a login page.
            WebError::Unauthenticated => StatusCode::UNAUTHORIZED,
            WebError::Forbidden | WebError::Csrf(_) => StatusCode::FORBIDDEN,
            WebError::NotFound => StatusCode::NOT_FOUND,
            WebError::BadRequest(_) => StatusCode::BAD_REQUEST,
            WebError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            WebError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// What the person reads. Never the `Display` form: that one is for the log.
    #[must_use]
    pub const fn public_message(&self) -> &'static str {
        match self {
            WebError::Unauthenticated => "Sign in to see this.",
            WebError::Forbidden => "Your account does not have access to this.",
            WebError::NotFound => "There is nothing at this address.",
            WebError::Csrf(error) => error.public_message(),
            WebError::BadRequest(_) => "That request could not be read.",
            WebError::Unavailable(_) => "Something this page needs is temporarily unavailable.",
            WebError::Internal => "Something went wrong on our side. It has been recorded.",
        }
    }
}

impl From<CsrfError> for WebError {
    fn from(error: CsrfError) -> Self {
        WebError::Csrf(error)
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let status = self.status();
        if matches!(self, WebError::Internal | WebError::Unavailable(_)) {
            tracing::error!(error = %self, "request failed");
        } else {
            tracing::debug!(error = %self, "request refused");
        }
        let body = html! {
            (maud::DOCTYPE)
            html lang="en" {
                head {
                    meta charset="utf-8";
                    meta name="viewport" content="width=device-width, initial-scale=1";
                    meta name="color-scheme" content="light dark";
                    title { (status.as_u16()) " · GHA Indie Worker" }
                    link rel="stylesheet" href="/assets/app.css";
                }
                body class="plain" {
                    main class="stack narrow" {
                        p class="eyebrow" { (status.as_u16()) }
                        h1 { (status.canonical_reason().unwrap_or("Error")) }
                        p { (self.public_message()) }
                        p { a class="button" href="/" { "Back to the start" } }
                    }
                }
            }
        };
        let mut response = (status, body).into_response();
        // An error page is never a cache entry.
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}
