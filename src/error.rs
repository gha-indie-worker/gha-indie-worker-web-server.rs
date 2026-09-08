#![forbid(unsafe_code)]

//! One typed error for every HTML surface. Rendering is deliberately terse: a
//! visitor sees a title and a short sentence, never an upstream message, a query
//! string, a token, or a database error.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("step-up authentication required")]
    StepUpRequired,
    #[error("csrf token missing or invalid")]
    CsrfRejected,
    #[error("not found")]
    NotFound,
    #[error("bad request")]
    BadRequest(&'static str),
    #[error("upstream unavailable")]
    Unavailable,
    #[error("rate limited")]
    RateLimited,
    #[error("internal error")]
    Internal,
}

impl WebError {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::StepUpRequired => StatusCode::FORBIDDEN,
            Self::CsrfRejected => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable machine code, safe to put in markup and logs.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden => "forbidden",
            Self::StepUpRequired => "step_up_required",
            Self::CsrfRejected => "csrf_rejected",
            Self::NotFound => "not_found",
            Self::BadRequest(_) => "bad_request",
            Self::Unavailable => "upstream_unavailable",
            Self::RateLimited => "rate_limited",
            Self::Internal => "internal_error",
        }
    }

    #[must_use]
    pub const fn headline(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "Sign in to continue",
            Self::Forbidden => "You do not have access to this",
            Self::StepUpRequired => "Confirm it is you",
            Self::CsrfRejected => "That form expired",
            Self::NotFound => "Nothing here",
            Self::BadRequest(_) => "That request was not understood",
            Self::Unavailable => "Temporarily unavailable",
            Self::RateLimited => "Too many requests",
            Self::Internal => "Something went wrong",
        }
    }

    #[must_use]
    pub const fn detail(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "This page needs a signed-in account.",
            Self::Forbidden => "Your role does not include this page. Ask an organization owner for access.",
            Self::StepUpRequired => "This action needs a second factor. Confirm with your authenticator or passkey.",
            Self::CsrfRejected => "Reload the page and try again — the security token no longer matched.",
            Self::NotFound => "The page you asked for does not exist on this host.",
            // `&&'static str` from the match ergonomics; the target is the inner one.
            Self::BadRequest(reason) => *reason,
            Self::Unavailable => "A service this page depends on did not answer. Nothing was changed.",
            Self::RateLimited => "Slow down for a moment and try again.",
            Self::Internal => "The request failed before it finished. Nothing was changed.",
        }
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        // The chrome-less body keeps this usable both as a full page and as an
        // htmx swap target; hosts::*::error_page wraps it in the shell.
        let body = crate::ui::components::error_block(&self);
        (self.status(), body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_codes_are_distinct_and_stable() {
        assert_eq!(WebError::NotFound.status(), StatusCode::NOT_FOUND);
        assert_eq!(WebError::CsrfRejected.status(), StatusCode::FORBIDDEN);
        assert_eq!(WebError::CsrfRejected.code(), "csrf_rejected");
        assert_eq!(WebError::RateLimited.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn details_never_leak_an_upstream_message() {
        for error in [
            WebError::Unavailable,
            WebError::Internal,
            WebError::Forbidden,
            WebError::Unauthenticated,
        ] {
            assert!(!error.detail().is_empty());
            assert!(!error.detail().contains("http"));
        }
    }
}
