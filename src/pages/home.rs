#![forbid(unsafe_code)]

//! The home page.
//!
//! The real thing lives in [`crate::hosts::app::marketing_home`], because what
//! `/` renders depends on which host asked and whether anyone is signed in.
//! This module stays as the stable entry point it was before the surface split.

use maud::Markup;

use crate::hosts::app;
use crate::middleware::RequestCtx;

/// The signed-out home body for the given request.
#[must_use]
pub fn markup(context: &RequestCtx) -> Markup {
    app::marketing_home(context)
}

/// The same body as a string, for tooling that wants the HTML directly.
#[must_use]
pub fn render(context: &RequestCtx) -> String {
    markup(context).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::Surface;

    fn context() -> RequestCtx {
        RequestCtx {
            surface: Surface::App,
            host: "app.indiebuild.dev".into(),
            base_domain: "indiebuild.dev".into(),
            path: "/".into(),
            nonce: "n".into(),
            csrf_token: "t".into(),
            csrf: crate::csrf::CsrfCheck::NotRequired,
            actor: None,
            is_htmx: false,
            subject: "anonymous".into(),
            release_manifest_url: None,
            chat_enabled: true,
        }
    }

    #[test]
    fn the_alias_renders_the_same_two_entry_points() {
        let html = render(&context());
        assert!(html.contains("https://org.indiebuild.dev/login"));
        assert!(html.contains("https://user.indiebuild.dev/login"));
    }
}
