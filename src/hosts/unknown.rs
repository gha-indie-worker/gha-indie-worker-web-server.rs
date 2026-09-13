#![forbid(unsafe_code)]

//! The surface for a `Host` this deployment does not serve.
//!
//! Everything on an unrecognised host answers **404**, including `/`, and the
//! page carries no product navigation, no sign-in control and no organization
//! or account detail. That matters for more than tidiness:
//!
//! * a dangling DNS record pointed at this service cannot be used to phish with
//!   real-looking product chrome;
//! * `Host`-header probing cannot enumerate which surfaces exist;
//! * a misconfigured CDN origin fails loudly during a deploy instead of quietly
//!   serving the wrong host's pages.
//!
//! `/healthz` is deliberately **not** mounted here: a load balancer must not be
//! able to call this deployment healthy through a host it does not serve.

use axum::response::{IntoResponse, Response};
use axum::Extension;
use axum::Router;
use maud::{html, Markup};

use crate::error::WebError;
use crate::hosts::common;
use crate::middleware::RequestCtx;
use crate::state::AppState;
use crate::ui::layout::PageMeta;

#[must_use]
pub fn routes() -> Router<AppState> {
    Router::new().fallback(not_found)
}

#[must_use]
pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

async fn not_found(Extension(context): Extension<RequestCtx>) -> Response {
    let mut response = common::render(&context, &meta(), body(&context)).into_response();
    *response.status_mut() = WebError::NotFound.status();
    response
}

fn meta() -> PageMeta {
    PageMeta::new("Not found", "This host is not served by GHA Indie Worker")
}

/// Names the host that was asked for, and nothing else. The host came from a
/// header, so it is rendered through maud's escaping like any other input.
#[must_use]
pub fn body(context: &RequestCtx) -> Markup {
    html! {
        section class="section" {
            p class="eyebrow" { "404" }
            h1 { "Nothing here" }
            p class="lede" {
                "This address is not one of the hosts GHA Indie Worker serves. If you followed a link, \
                 it is out of date."
            }
            @if !context.host.is_empty() {
                p { "Requested host: " code { (context.host) } }
            }
            p { a class="text-link" href="https://github.com/gha-indie-worker" { "github.com/gha-indie-worker" } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::Surface;

    fn context(host: &str) -> RequestCtx {
        RequestCtx {
            surface: Surface::Unknown,
            host: host.into(),
            base_domain: "indiebuild.dev".into(),
            path: "/".into(),
            nonce: "n0nce".into(),
            csrf_token: "tok3n".into(),
            csrf: crate::csrf::CsrfCheck::NotRequired,
            actor: None,
            is_htmx: false,
            subject: "anonymous".into(),
            release_manifest_url: None,
            chat_enabled: true,
        }
    }

    #[test]
    fn the_page_names_the_host_without_trusting_it() {
        let html = body(&context("<script>alert(1)</script>.example")).into_string();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn an_unknown_host_gets_no_product_navigation() {
        use crate::ui::layout::page;
        let markup = page(&context("evil.example").page(), &meta(), body(&context("evil.example"))).into_string();
        assert!(!markup.contains(r#"class="primary-nav""#));
        assert!(!markup.contains(r#"class="bottom-nav""#));
    }

    #[test]
    fn a_missing_host_still_renders_a_page() {
        let html = body(&context("")).into_string();
        assert!(html.contains("Nothing here"));
        assert!(!html.contains("Requested host"));
    }
}
