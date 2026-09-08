#![forbid(unsafe_code)]

//! Navigation, per surface.
//!
//! Desktop surfaces get the marketing site's mono top nav. `m.` gets a bottom
//! bar instead: fixed, thumb-reachable, three-to-five destinations, each a real
//! link with a 3rem minimum target. Nothing anywhere depends on `:hover` to be
//! reachable — hover only changes colour.

use maud::{html, Markup};

use crate::hosts::Surface;
use crate::ui::layout::PageContext;

/// One navigation destination.
#[derive(Clone, Copy, Debug)]
pub struct NavItem {
    /// Stable key, matched against `PageMeta::active`.
    pub key: &'static str,
    pub href: &'static str,
    pub label: &'static str,
    /// Single character shown above the label in the bottom bar.
    pub glyph: &'static str,
}

const APP_SIGNED_IN: &[NavItem] = &[
    NavItem {
        key: "runs",
        href: "/runs",
        label: "Runs",
        glyph: "▸",
    },
    NavItem {
        key: "workers",
        href: "/workers",
        label: "Workers",
        glyph: "▤",
    },
    NavItem {
        key: "plans",
        href: "/plans",
        label: "Plans",
        glyph: "◇",
    },
    NavItem {
        key: "settings",
        href: "/settings",
        label: "Settings",
        glyph: "⚙",
    },
];

const APP_SIGNED_OUT: &[NavItem] = &[
    NavItem {
        key: "principles",
        href: "/#principles",
        label: "Principles",
        glyph: "◈",
    },
    NavItem {
        key: "method",
        href: "/#method",
        label: "Method",
        glyph: "≡",
    },
    NavItem {
        key: "access",
        href: "/#access",
        label: "Access",
        glyph: "→",
    },
];

const ORG_NAV: &[NavItem] = &[
    NavItem {
        key: "overview",
        href: "/",
        label: "Overview",
        glyph: "◈",
    },
    NavItem {
        key: "members",
        href: "/members",
        label: "Members",
        glyph: "☰",
    },
    NavItem {
        key: "seats",
        href: "/seats",
        label: "Seats",
        glyph: "▦",
    },
    NavItem {
        key: "audit",
        href: "/audit",
        label: "Audit",
        glyph: "✓",
    },
    NavItem {
        key: "sso",
        href: "/sso",
        label: "SSO",
        glyph: "⛨",
    },
];

const USER_NAV: &[NavItem] = &[
    NavItem {
        key: "workspace",
        href: "/workspace",
        label: "Workspace",
        glyph: "◈",
    },
    NavItem {
        key: "tokens",
        href: "/tokens",
        label: "Tokens",
        glyph: "⚿",
    },
    NavItem {
        key: "security",
        href: "/security",
        label: "Security",
        glyph: "⛨",
    },
];

/// The destinations this surface offers, given who is looking.
#[must_use]
pub fn items(context: &PageContext) -> &'static [NavItem] {
    match context.surface {
        Surface::App | Surface::Mobile => {
            if context.is_signed_in() {
                APP_SIGNED_IN
            } else {
                APP_SIGNED_OUT
            }
        }
        Surface::Org => ORG_NAV,
        Surface::User => USER_NAV,
        Surface::Unknown => &[],
    }
}

/// The masthead navigation. Hidden on compact layouts, where the bottom bar
/// takes over, so the same markup serves both without duplication.
#[must_use]
pub fn primary_nav(context: &PageContext, active: &str) -> Markup {
    let items = items(context);
    html! {
        @if !items.is_empty() {
            nav class="primary-nav" aria-label="Primary" {
                @for item in items {
                    @if item.key == active {
                        a href=(item.href) aria-current="page" { (item.label) }
                    } @else {
                        a href=(item.href) { (item.label) }
                    }
                }
            }
        }
    }
}

/// The `m.` bottom bar.
#[must_use]
pub fn bottom_nav(context: &PageContext, active: &str) -> Markup {
    let items = items(context);
    html! {
        @if !items.is_empty() {
            nav class="bottom-nav" aria-label="Primary" {
                @for item in items {
                    @if item.key == active {
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
}

/// A breadcrumb trail. `(label, href)`; the last entry is the current page.
#[must_use]
pub fn breadcrumbs(trail: &[(&str, &str)]) -> Markup {
    html! {
        nav class="row" aria-label="Breadcrumb" {
            @for (index, (label, href)) in trail.iter().enumerate() {
                @if index + 1 == trail.len() {
                    span class="text-link" aria-current="page" { (label) }
                } @else {
                    a class="text-link" href=(href) { (label) }
                    span class="text-link" aria-hidden="true" { "/" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_surface_swaps_navigation_when_signed_in() {
        let mut context = PageContext::for_tests(Surface::App);
        assert_eq!(items(&context).len(), APP_SIGNED_OUT.len());
        context.actor = Some(crate::session::Actor {
            sub: "u".into(),
            email: None,
            org_id: None,
            roles: vec![],
            scopes: vec![],
            acr: None,
            source: crate::session::ActorSource::Session,
            access_token: String::new(),
        });
        assert_eq!(items(&context).len(), APP_SIGNED_IN.len());
    }

    #[test]
    fn an_unknown_host_gets_no_navigation_at_all() {
        let context = PageContext::for_tests(Surface::Unknown);
        assert!(items(&context).is_empty());
        assert_eq!(primary_nav(&context, "").into_string(), "");
        assert_eq!(bottom_nav(&context, "").into_string(), "");
    }

    #[test]
    fn the_active_item_is_marked_once() {
        let context = PageContext::for_tests(Surface::Org);
        let html = primary_nav(&context, "members").into_string();
        assert_eq!(html.matches(r#"aria-current="page""#).count(), 1);
        assert!(html.contains(r#"<a href="/members" aria-current="page">Members</a>"#));
    }

    #[test]
    fn the_bottom_bar_labels_every_destination() {
        let context = PageContext::for_tests(Surface::Org);
        let html = bottom_nav(&context, "seats").into_string();
        for item in ORG_NAV {
            assert!(html.contains(item.label), "missing {}", item.label);
        }
        assert!(html.contains(r#"aria-hidden="true""#));
    }

    #[test]
    fn breadcrumbs_mark_only_the_last_entry_as_current() {
        let html = breadcrumbs(&[("Runs", "/runs"), ("Run 42", "/runs/42")]).into_string();
        assert_eq!(html.matches(r#"aria-current="page""#).count(), 1);
        assert!(html.contains("Run 42"));
    }
}
