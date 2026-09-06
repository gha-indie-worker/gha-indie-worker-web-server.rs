#![forbid(unsafe_code)]

//! The shared shell every page renders into.
//!
//! Responsibilities, in order:
//!
//! 1. carry the per-request **CSP nonce** onto the only two tags allowed to
//!    execute or style: the inline `<style>` and the self-hosted htmx `<script>`;
//! 2. put the **CSRF token** on `<body hx-headers>` so every htmx request echoes
//!    it automatically, and expose it to [`crate::ui::forms`] for the
//!    no-JavaScript fallback;
//! 3. switch layout by [`Surface`] — `m.` renders `body.compact` with a bottom
//!    navigation bar and no hover-dependent affordance;
//! 4. keep the marketing site's identity (`GHA/IW`, the accent, the mono
//!    eyebrows) so `app.indiebuild.dev` reads as the same product as the
//!    Astro home a visitor just came from.

use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::hosts::Surface;
use crate::session::Actor;
use crate::ui::{chat_widget, loader, nav, styles};

/// The product name, matching `gha-indie-worker.github.io`.
pub const PRODUCT_NAME: &str = "GHA Indie Worker";
/// The mono mark in the masthead, matching the marketing site.
pub const PRODUCT_MARK: &str = "GHA/IW";
/// Self-hosted htmx. Fetched by `scripts/vendor-assets.sh`; never a CDN.
pub const HTMX_PATH: &str = "/assets/vendor/htmx.min.js";

/// Everything a page needs from the request, with nothing a page could misuse.
#[derive(Clone, Debug)]
pub struct PageContext {
    pub surface: Surface,
    pub base_domain: String,
    /// Per-request CSP nonce. Never reused across requests.
    pub nonce: String,
    /// Double-submit CSRF token for this request's subject.
    pub csrf_token: String,
    pub actor: Option<Actor>,
    pub path: String,
    pub release_manifest_url: Option<String>,
    pub chat_enabled: bool,
}

impl PageContext {
    #[must_use]
    pub fn is_signed_in(&self) -> bool {
        self.actor.is_some()
    }

    #[must_use]
    pub fn origin(&self, surface: Surface) -> String {
        surface.origin(&self.base_domain)
    }

    /// A deterministic context for unit tests and markup snapshots.
    #[must_use]
    pub fn for_tests(surface: Surface) -> Self {
        Self {
            surface,
            base_domain: crate::config::DEFAULT_BASE_DOMAIN.to_owned(),
            nonce: "n0nce".to_owned(),
            csrf_token: "tok3n".to_owned(),
            actor: None,
            path: "/".to_owned(),
            release_manifest_url: None,
            chat_enabled: true,
        }
    }
}

/// Per-page switches.
#[derive(Clone, Debug)]
pub struct PageMeta {
    pub title: String,
    pub description: String,
    /// Which nav entry to mark `aria-current="page"`.
    pub active: &'static str,
    /// Page carries a Leptos/Dioxus island and needs ores-web-loader.
    pub islands: bool,
    /// Emit `<link rel="prefetch">` for the app release — on intent only.
    pub prefetch_app_release: bool,
    /// Which ores-chat widget, if any, belongs on this page.
    pub chat: Option<chat_widget::ChatAudience>,
}

impl PageMeta {
    #[must_use]
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            active: "",
            islands: false,
            prefetch_app_release: false,
            chat: None,
        }
    }

    #[must_use]
    pub fn active(mut self, active: &'static str) -> Self {
        self.active = active;
        self
    }

    #[must_use]
    pub fn with_islands(mut self) -> Self {
        self.islands = true;
        self
    }

    #[must_use]
    pub fn with_release_prefetch(mut self) -> Self {
        self.prefetch_app_release = true;
        self
    }

    #[must_use]
    pub fn with_chat(mut self, audience: chat_widget::ChatAudience) -> Self {
        self.chat = Some(audience);
        self
    }
}

/// Renders one full page.
#[must_use]
pub fn page(context: &PageContext, meta: &PageMeta, body: Markup) -> Markup {
    let body_class = if context.surface.is_compact() {
        "compact"
    } else {
        "wide"
    };
    let hx_headers = format!("{{\"X-CSRF-Token\": \"{}\"}}", context.csrf_token);
    let full_title = format!("{} — {PRODUCT_NAME}", meta.title);

    // An unrecognised Host gets the page and nothing else: no masthead, no navigation,

    // no footer, no chat. Product chrome on a host we do not serve is a phishing

    // surface and a way to enumerate which surfaces exist.

    let chrome = context.surface != Surface::Unknown;

    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="description" content=(meta.description);
                meta name="color-scheme" content="dark light";
                meta name="referrer" content="strict-origin-when-cross-origin";
                title { (full_title) }
                style nonce=(context.nonce) { (PreEscaped(styles::CSS)) }
                // The only script on a non-island page, self-hosted, nonce-tagged.
                script src=(HTMX_PATH) nonce=(context.nonce) defer="defer" {}
                @if meta.islands {
                    (loader::island_head_tags(context))
                }
                @if meta.prefetch_app_release {
                    (loader::release_prefetch_hint(context))
                }
            }
            body class=(body_class) data-surface=(context.surface.label()) hx-boost="true" hx-headers=(hx_headers) {
                a class="skip-link" href="#main" { "Skip to content" }
                @if chrome { (masthead(context, meta)) }
                main id="main" { (body) }
                @if chrome { (site_footer(context)) }
                @if chrome && context.surface.is_compact() {
                    (nav::bottom_nav(context, meta.active))
                }
                @if let Some(audience) = meta.chat {
                    @if chrome && context.chat_enabled {
                        (chat_widget::widget(context, audience))
                    }
                }
            }
        }
    }
}

/// A page fragment: an htmx swap target, rendered without the shell.
///
/// htmx replaces the target element's inner HTML, so a fragment must never
/// carry `<html>`, `<head>` or the stylesheet — that is what this exists for.
#[must_use]
pub fn fragment(body: Markup) -> Markup {
    body
}

fn masthead(context: &PageContext, meta: &PageMeta) -> Markup {
    let home = context.origin(Surface::App);
    html! {
        header class="masthead" {
            a class="identity" href=(home) aria-label=(format!("{PRODUCT_NAME} home")) {
                span class="mark" { (PRODUCT_MARK) }
                span { (PRODUCT_NAME) }
            }
            (nav::primary_nav(context, meta.active))
            div class="identity-actions" {
                span class="surface-tag" { (context.surface.label()) }
                @match &context.actor {
                    Some(actor) => {
                        span { (actor.display_name()) }
                        form method="post" action="/logout" hx-post="/logout" hx-target="body" {
                            input type="hidden" name="csrf_token" value=(context.csrf_token);
                            button type="submit" class="button secondary" { "Sign out" }
                        }
                    }
                    None => {
                        a class="text-link" href=(format!("{}/login", context.origin(Surface::User))) { "Personal sign in" }
                        a class="button" href=(format!("{}/login", context.origin(Surface::Org))) { "Organization" }
                    }
                }
            }
        }
    }
}

fn site_footer(_context: &PageContext) -> Markup {
    html! {
        footer class="site" {
            span { (PRODUCT_MARK) }
            span { "Run Actions on your own machines. Keep the contract native." }
            span { a href="https://github.com/gha-indie-worker" { "github.com/gha-indie-worker" } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(surface: Surface) -> PageContext {
        PageContext::for_tests(surface)
    }

    #[test]
    fn the_shell_carries_the_nonce_on_every_executable_tag() {
        let markup = page(
            &context(Surface::App),
            &PageMeta::new("Home", "d"),
            html! { p { "hi" } },
        );
        let html = markup.into_string();
        assert!(html.contains(r#"<style nonce="n0nce">"#));
        assert!(html.contains(r#"src="/assets/vendor/htmx.min.js" nonce="n0nce""#));
        // No third-party origin may appear in a script or link tag.
        assert!(!html.contains("unpkg.com"));
        assert!(!html.contains("cdn.jsdelivr.net"));
    }

    #[test]
    fn the_csrf_token_reaches_every_htmx_request() {
        let html = page(&context(Surface::App), &PageMeta::new("Home", "d"), html! {}).into_string();
        assert!(html.contains("hx-headers"));
        assert!(html.contains("X-CSRF-Token"));
        assert!(html.contains("tok3n"));
        assert!(html.contains(r#"hx-boost="true""#));
    }

    #[test]
    fn mobile_selects_the_compact_layout_and_a_bottom_nav() {
        let html = page(&context(Surface::Mobile), &PageMeta::new("Home", "d"), html! {}).into_string();
        assert!(html.contains(r#"<body class="compact""#));
        assert!(html.contains(r#"class="bottom-nav""#));

        let desktop = page(&context(Surface::App), &PageMeta::new("Home", "d"), html! {}).into_string();
        assert!(desktop.contains(r#"<body class="wide""#));
        assert!(!desktop.contains(r#"class="bottom-nav""#));
    }

    #[test]
    fn a_signed_out_shell_offers_both_entry_points() {
        let html = page(&context(Surface::App), &PageMeta::new("Home", "d"), html! {}).into_string();
        assert!(html.contains("https://user.indiebuild.dev/login"));
        assert!(html.contains("https://org.indiebuild.dev/login"));
    }

    #[test]
    fn island_tags_appear_only_when_the_page_asks_for_them() {
        let plain = page(&context(Surface::App), &PageMeta::new("Home", "d"), html! {}).into_string();
        assert!(!plain.contains("ores-web-loader.js"));
        let island = page(
            &context(Surface::App),
            &PageMeta::new("Home", "d").with_islands(),
            html! {},
        )
        .into_string();
        assert!(island.contains("ores-web-loader.js"));
    }

    #[test]
    fn the_page_title_and_description_are_escaped() {
        let meta = PageMeta::new("<script>x</script>", "\"quoted\"");
        let html = page(&context(Surface::App), &meta, html! {}).into_string();
        assert!(!html.contains("<script>x</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
