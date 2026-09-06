#![forbid(unsafe_code)]
//! The page shell: `<head>`, masthead, navigation, footer — and the cross-surface links that make
//! the two doors reachable from each other.
//!
//! Accessibility decisions that are structural rather than cosmetic, and so belong here:
//!
//! * A skip link is the first focusable thing on every page, and `<main id="main">` is its target.
//! * Navigation is a `<nav>` with an accessible name, and the current page is marked with
//!   `aria-current="page"` rather than by colour alone.
//! * The mobile shell's bottom navigation is the *same* list in the same order as the desktop
//!   masthead, so the two surfaces do not disagree about where things are.
//! * Nothing in the page depends on script. htmx, when it is vendored, upgrades forms to fragment
//!   swaps; without it every form still posts and every link still navigates.

use maud::{html, Markup, PreEscaped, DOCTYPE};

use crate::assets;
use crate::present::{Face, Page};
use crate::state::Ctx;

/// Where a nav item points and what it is called.
pub struct NavItem {
    pub href: &'static str,
    pub label: &'static str,
    /// A text glyph for the mobile bottom bar. Decorative: the label is always rendered too.
    pub glyph: &'static str,
    pub matches: fn(&Page) -> bool,
}

/// `https://org.indiebuild.dev/members` — an absolute link to another surface.
///
/// Cross-surface links are absolute because the surfaces are different hosts; a relative `/members`
/// from `user.` would stay on `user.` and 404. The scheme follows the deployment rather than the
/// request, so a developer on loopback is not bounced to https.
#[must_use]
pub fn surface_url(apex: &str, secure: bool, label: &str, path: &str) -> String {
    let scheme = if secure { "https" } else { "http" };
    if apex.contains("://") || apex.starts_with("127.") || apex == "localhost" {
        // A loopback development origin has no subdomains to speak of; stay where we are and let
        // the developer set a `Host` header if they want to exercise another surface.
        return path.to_owned();
    }
    format!("{scheme}://{label}.{apex}{path}")
}

/// The application navigation, shared by `app.` and `m.`.
#[must_use]
pub fn application_nav() -> Vec<NavItem> {
    vec![
        NavItem {
            href: "/",
            label: "Overview",
            glyph: "◈",
            matches: |page| matches!(page, Page::AppDashboard),
        },
        NavItem {
            href: "/runs",
            label: "Runs",
            glyph: "▤",
            matches: |page| matches!(page, Page::AppRuns | Page::AppRunDetail(_)),
        },
        NavItem {
            href: "/runners",
            label: "Runners",
            glyph: "⬡",
            matches: |page| matches!(page, Page::AppRunners),
        },
        NavItem {
            href: "/caches",
            label: "Caches",
            glyph: "▦",
            matches: |page| matches!(page, Page::AppCaches),
        },
        NavItem {
            href: "/settings",
            label: "Settings",
            glyph: "⚙",
            matches: |page| matches!(page, Page::AppSettings),
        },
    ]
}

/// The organization navigation.
#[must_use]
pub fn organization_nav() -> Vec<NavItem> {
    vec![
        NavItem {
            href: "/members",
            label: "Members",
            glyph: "☰",
            matches: |page| matches!(page, Page::OrgMembers),
        },
        NavItem {
            href: "/invitations",
            label: "Seats & invitations",
            glyph: "✉",
            matches: |page| matches!(page, Page::OrgInvitations),
        },
        NavItem {
            href: "/domains",
            label: "Domains",
            glyph: "◎",
            matches: |page| matches!(page, Page::OrgDomains),
        },
        NavItem {
            href: "/sso",
            label: "Single sign-on",
            glyph: "⚿",
            matches: |page| matches!(page, Page::OrgSso),
        },
        NavItem {
            href: "/settings",
            label: "Settings",
            glyph: "⚙",
            matches: |page| matches!(page, Page::OrgSettings),
        },
    ]
}

/// Everything a template needs that is not the page's own content.
pub struct Chrome<'a> {
    pub ctx: &'a Ctx,
    pub page: &'a Page,
    pub nonce: &'a str,
    pub apex: &'a str,
    pub secure: bool,
    /// An inline `<script>` body, run under the page's nonce. Used only to hand the log tail its
    /// stream name; there is no other inline script in the product.
    pub inline_script: Option<String>,
}

/// Render a complete document.
#[must_use]
pub fn shell(chrome: &Chrome<'_>, body: Markup) -> Markup {
    let assets = assets::assets();
    let face = chrome.ctx.face;
    let body_class = match face {
        Face::Marketing => "marketing",
        Face::User => "user",
        Face::Org => "org",
        Face::App => "app",
        Face::Mobile => "m",
    };
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="color-scheme" content="light dark";
                meta name="referrer" content="strict-origin-when-cross-origin";
                title { (chrome.page.title()) " · GHA Indie Worker" }
                // Same-origin subresources do not need CORS for integrity to be checked, so there
                // is deliberately no `crossorigin` attribute here: adding one would turn a plain
                // same-origin fetch into a CORS one for no benefit.
                link rel="stylesheet" href=(assets.app_css.0.path) integrity=(assets.app_css.1);
                @if let Some((asset, integrity)) = &assets.htmx_js {
                    script src=(asset.path) integrity=(integrity) defer {}
                }
                @if matches!(face, Face::App | Face::Mobile) {
                    script src=(assets.tail_js.0.path) integrity=(assets.tail_js.1) defer {}
                }
                @if let Some(script) = &chrome.inline_script {
                    script nonce=(chrome.nonce) { (PreEscaped(script.clone())) }
                }
            }
            body class=(body_class) {
                a class="skip-link" href="#main" { "Skip to content" }
                (masthead(chrome))
                main id="main" tabindex="-1" {
                    div class="wrap" { (body) }
                }
                @if face.is_small_screen() && chrome.ctx.authenticated() {
                    (bottom_nav(chrome))
                } @else {
                    (footer(chrome))
                }
            }
        }
    }
}

fn masthead(chrome: &Chrome<'_>) -> Markup {
    let face = chrome.ctx.face;
    html! {
        header class="masthead" {
            div class="wrap" {
                a class="brand" href="/" {
                    span class="mark" aria-hidden="true" { "gi" }
                    span { "GHA Indie Worker" }
                    @if !matches!(face, Face::Marketing) {
                        span class="surface-tag" { (face.label()) }
                    }
                }
                span class="grow" {}
                @if matches!(face, Face::Marketing) {
                    (marketing_nav(chrome))
                } @else if matches!(face, Face::App) {
                    (nav_list("Application", &application_nav(), chrome))
                } @else if matches!(face, Face::Org) && chrome.ctx.authenticated() {
                    (nav_list("Organization", &organization_nav(), chrome))
                }
                @if let Some(session) = &chrome.ctx.session {
                    form method="post" action="/auth/signout" class="row" {
                        input type="hidden" name="csrf_token" value=(chrome.ctx.csrf_token);
                        span class="faint" { (session.email) }
                        button type="submit" class="button button-quiet" { "Sign out" }
                    }
                }
            }
        }
    }
}

fn nav_list(name: &str, items: &[NavItem], chrome: &Chrome<'_>) -> Markup {
    html! {
        nav class="mainnav" aria-label=(name) {
            @for item in items {
                @if (item.matches)(chrome.page) {
                    a href=(item.href) aria-current="page" { (item.label) }
                } @else {
                    a href=(item.href) { (item.label) }
                }
            }
        }
    }
}

fn marketing_nav(chrome: &Chrome<'_>) -> Markup {
    let org = surface_url(chrome.apex, chrome.secure, "org", "/");
    let user = surface_url(chrome.apex, chrome.secure, "user", "/");
    html! {
        nav class="mainnav" aria-label="Marketing" {
            a href="/pricing" { "Pricing" }
        }
        div class="row" {
            a class="button" href=(org) { "For your team" }
            a class="button button-primary" href=(user) { "For yourself" }
        }
    }
}

fn bottom_nav(chrome: &Chrome<'_>) -> Markup {
    html! {
        nav class="bottomnav" aria-label="Sections" {
            @for item in application_nav() {
                @if (item.matches)(chrome.page) {
                    a href=(item.href) aria-current="page" {
                        span class="glyph" aria-hidden="true" { (item.glyph) }
                        span { (item.label) }
                    }
                } @else {
                    a href=(item.href) {
                        span class="glyph" aria-hidden="true" { (item.glyph) }
                        span { (item.label) }
                    }
                }
            }
        }
    }
}

fn footer(chrome: &Chrome<'_>) -> Markup {
    let org = surface_url(chrome.apex, chrome.secure, "org", "/");
    let user = surface_url(chrome.apex, chrome.secure, "user", "/");
    html! {
        footer class="sitefoot" {
            div class="wrap" {
                p { "GHA Indie Worker — self-hosted GitHub Actions runners you do not have to babysit." }
                nav aria-label="Other ways in" {
                    div class="row" {
                        a href=(org) { "For your team" }
                        span class="faint" aria-hidden="true" { "·" }
                        a href=(user) { "For yourself" }
                        span class="faint" aria-hidden="true" { "·" }
                        a href="/pricing" { "Pricing" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_surface_links_name_the_other_host_absolutely() {
        assert_eq!(
            surface_url("indiebuild.dev", true, "org", "/new"),
            "https://org.indiebuild.dev/new"
        );
        assert_eq!(
            surface_url("indiebuild.dev", false, "user", "/"),
            "http://user.indiebuild.dev/"
        );
        // On a laptop there are no surface subdomains, so the link stays relative rather than
        // pointing at a host that does not resolve.
        assert_eq!(surface_url("127.0.0.1:8081", false, "org", "/new"), "/new");
        assert_eq!(surface_url("localhost", false, "org", "/new"), "/new");
    }

    #[test]
    fn the_two_navigations_agree_about_what_exists_and_in_what_order() {
        let desktop: Vec<&str> = application_nav().iter().map(|item| item.href).collect();
        let mobile: Vec<&str> = application_nav().iter().map(|item| item.href).collect();
        assert_eq!(desktop, mobile);
        assert_eq!(
            desktop,
            vec!["/", "/runs", "/runners", "/caches", "/settings"]
        );
        for item in application_nav().iter().chain(organization_nav().iter()) {
            assert!(!item.label.is_empty(), "{} has no label", item.href);
            assert!(!item.glyph.is_empty(), "{} has no glyph", item.href);
        }
    }

    #[test]
    fn run_detail_keeps_the_runs_tab_marked_current() {
        let runs = application_nav()
            .into_iter()
            .find(|item| item.href == "/runs")
            .unwrap();
        assert!((runs.matches)(&Page::AppRuns));
        assert!((runs.matches)(&Page::AppRunDetail("01HZ".into())));
        assert!(!(runs.matches)(&Page::AppRunners));
    }
}
