#![forbid(unsafe_code)]

//! `m.indiebuild.dev` — the compact surface.
//!
//! Not a different product and not a different set of handlers: the **same**
//! routes as [`super::app`], plus the personal pages people reach for on a
//! phone. What differs is entirely in the shell, and it differs because
//! [`Surface::Mobile`] is compact:
//!
//! * `body.compact` — single-column grids, tighter sections, larger base type;
//! * a fixed **bottom navigation bar** instead of the masthead's inline nav;
//! * **no hover-dependent affordance anywhere** — every control is reachable by
//!   tap and by keyboard, and `:hover` only ever changes colour;
//! * touch targets of at least 3rem, with the safe-area inset respected.
//!
//! Reusing [`super::app::routes`] rather than copying it is what keeps the two
//! surfaces from drifting: a route added to the product exists on mobile the
//! same day.

use axum::Router;

use crate::hosts::{app, user, Surface};
use crate::state::AppState;

/// The `m.` router: the product routes, plus the personal workspace and
/// security pages, which is what the bottom bar links to on a phone.
#[must_use]
pub fn routes() -> Router<AppState> {
    // `merge` would collide on the shared routes that both surfaces mount
    // (`/healthz`, `/ws`, `/logout`, chat), so mobile takes the app routes —
    // which already include the shared set — and nests the personal pages under
    // their own prefix rather than re-merging the user surface wholesale.
    app::routes().nest("/me", user_pages())
}

/// The subset of the personal surface that makes sense on a phone.
fn user_pages() -> Router<AppState> {
    // `user::routes()` also carries the shared routes; nesting keeps them at
    // `/me/healthz` and so on, which is harmless and avoids a route conflict.
    user::routes()
}

#[must_use]
pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

/// Whether this surface should render the compact shell. Kept as a function so
/// the rule has one home and the tests can assert on it directly.
#[must_use]
pub const fn is_compact(surface: Surface) -> bool {
    surface.is_compact()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_mobile_surface_is_compact() {
        assert!(is_compact(Surface::Mobile));
        for surface in [Surface::App, Surface::User, Surface::Org, Surface::Unknown] {
            assert!(!is_compact(surface), "{surface:?} must not use the compact shell");
        }
    }

    #[test]
    fn the_compact_shell_replaces_the_inline_nav_with_a_bottom_bar() {
        use crate::ui::layout::{page, PageContext, PageMeta};

        let mut context = PageContext::for_tests(Surface::Mobile);
        context.actor = Some(crate::session::Actor {
            sub: "u1".into(),
            email: Some("a@example.com".into()),
            org_id: None,
            roles: vec![],
            scopes: vec![],
            acr: None,
            source: crate::session::ActorSource::Session,
            access_token: String::new(),
        });
        let html = page(&context, &PageMeta::new("Runs", "d").active("runs"), maud::html! {}).into_string();

        assert!(html.contains(r#"<body class="compact""#));
        assert!(html.contains(r#"class="bottom-nav""#));
        assert!(html.contains(r#"data-surface="m""#));
        // The bottom bar is where the current page is marked on mobile.
        assert!(html.contains(r#"aria-current="page""#));
    }

    #[test]
    fn the_stylesheet_makes_the_bottom_bar_mobile_only() {
        let css = crate::ui::styles::CSS;
        assert!(css.contains(".bottom-nav { position: fixed"), "the bar must be pinned");
        assert!(
            css.contains("body.compact .bottom-nav { display: grid"),
            "only compact shows it"
        );
        assert!(
            css.contains("body.compact .primary-nav { display: none"),
            "compact hides the inline nav"
        );
        assert!(css.contains("min-height: 3rem"), "touch targets must be at least 3rem");
        assert!(
            css.contains("env(safe-area-inset-bottom)"),
            "the bar must clear the home indicator"
        );
    }
}
