#![forbid(unsafe_code)]

//! ores-web-loader adoption (pilot).
//!
//! Most of this product is server-rendered maud; a few pages want a real client
//! island (a live run graph, a log scrubber). Those pages — and only those —
//! emit:
//!
//! * `<link rel="modulepreload" href="/assets/loader/ores-web-loader.js">`
//! * `<script type="module" src="/assets/loader/ores-web-loader.js" nonce=…>`
//! * a mount element `<div data-ores-app="gha-indie-worker-web" data-release=…>`
//!
//! The release identifier comes from `GHA_INDIE_WORKER_RELEASE_MANIFEST_URL`;
//! release assets are served **same-origin** from `/assets/releases/{hash}/…`
//! out of `GHA_INDIE_WORKER_ASSETS_DIR` with immutable cache headers, so the
//! loader never contacts a third-party origin and the CSP stays `'self'`.
//!
//! ## Prefetch is on intent, never on load
//!
//! A marketing page must not pull a megabyte of wasm for a visitor who is only
//! reading. [`open_app_link`] attaches an htmx `hx-trigger` on
//! `mouseenter`/`focus` of the "Open app" control, which fetches a tiny endpoint
//! that returns the `<link rel="prefetch">` tags. Hover **and** focus, so a
//! keyboard user gets the same head start as a mouse user.
//!
//! Islands themselves are behind the `islands` cargo feature (default OFF) so
//! this crate builds and tests without a wasm toolchain; the tags are emitted
//! either way, and with the feature off the mount is inert and its server-side
//! fallback content is what the visitor sees.

use maud::{html, Markup};

use crate::ui::layout::PageContext;

/// The loader module, served same-origin.
pub const LOADER_PATH: &str = "/assets/loader/ores-web-loader.js";
/// Application id the loader looks up in the release manifest.
pub const APP_ID: &str = "gha-indie-worker-web";
/// Immutable, content-addressed release assets.
pub const RELEASE_PREFIX: &str = "/assets/releases";
/// Endpoint that returns prefetch hints, fetched on hover/focus only.
///
/// Deliberately outside `/assets`, which is a static-file mount.
pub const PREFETCH_PATH: &str = "/_prefetch/release";

/// True when islands are compiled in. The markup is identical either way; this
/// only tells the loader whether a client bundle is expected to exist.
#[must_use]
pub const fn islands_enabled() -> bool {
    cfg!(feature = "islands")
}

/// The release identifier put on every mount and asset URL.
///
/// It is derived from the configured manifest URL so a deploy that rolls the
/// manifest also rolls every asset path, which is what makes the immutable
/// cache headers safe.
#[must_use]
pub fn release_id(context: &PageContext) -> String {
    context
        .release_manifest_url
        .as_deref()
        .and_then(|url| {
            url.trim_end_matches('/')
                .rsplit('/')
                .find(|segment| !segment.is_empty() && *segment != "manifest.json")
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "dev".to_owned())
}

/// `<head>` tags for a page that carries an island.
#[must_use]
pub fn island_head_tags(context: &PageContext) -> Markup {
    html! {
        link rel="modulepreload" href=(LOADER_PATH);
        script type="module" src=(LOADER_PATH) nonce=(context.nonce) {}
    }
}

/// The mount point an island attaches to.
///
/// `fallback` is rendered server-side inside the mount. If the island never
/// hydrates — feature off, wasm blocked, script error — the page still shows
/// something real rather than an empty box.
#[must_use]
pub fn island_mount(context: &PageContext, island: &str, fallback: Markup) -> Markup {
    let release = release_id(context);
    let manifest = context.release_manifest_url.clone().unwrap_or_default();
    html! {
        div data-ores-app=(APP_ID)
            data-ores-island=(island)
            data-release=(release)
            data-manifest=(manifest)
            data-assets=(format!("{RELEASE_PREFIX}/{release}/"))
            data-enabled=(if islands_enabled() { "true" } else { "false" }) {
            (fallback)
        }
    }
}

/// The prefetch hints themselves. Returned by [`PREFETCH_PATH`], never inlined
/// into a page that was merely loaded.
#[must_use]
pub fn release_prefetch_hint(context: &PageContext) -> Markup {
    let release = release_id(context);
    html! {
        link rel="prefetch" as="script" href=(format!("{RELEASE_PREFIX}/{release}/app.js"));
        link rel="prefetch" as="fetch" crossorigin="anonymous" href=(format!("{RELEASE_PREFIX}/{release}/app.wasm"));
        link rel="prefetch" as="script" href=(LOADER_PATH);
    }
}

/// The "Open app" control that warms the release on intent.
///
/// `hx-trigger="mouseenter once, focus once"` — one fetch per element, on either
/// pointer or keyboard intent, into an out-of-band `<head>` sink.
#[must_use]
pub fn open_app_link(context: &PageContext, label: &str) -> Markup {
    let href = format!("{}/dashboard", context.origin(crate::hosts::Surface::App));
    html! {
        a class="button" href=(href)
          hx-get=(PREFETCH_PATH)
          hx-trigger="mouseenter once, focus once"
          hx-target="#release-prefetch"
          hx-swap="innerHTML" { (label) }
        span id="release-prefetch" hidden="hidden" {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::Surface;

    fn context_with(manifest: Option<&str>) -> PageContext {
        let mut context = PageContext::for_tests(Surface::App);
        context.release_manifest_url = manifest.map(str::to_owned);
        context
    }

    #[test]
    fn the_release_id_comes_from_the_manifest_url() {
        let context = context_with(Some("https://assets.indiebuild.dev/releases/9f3a1c/manifest.json"));
        assert_eq!(release_id(&context), "9f3a1c");
    }

    #[test]
    fn a_missing_manifest_falls_back_to_a_development_release() {
        assert_eq!(release_id(&context_with(None)), "dev");
    }

    #[test]
    fn head_tags_preload_and_load_the_loader_same_origin() {
        let html = island_head_tags(&context_with(None)).into_string();
        assert!(html.contains(r#"<link rel="modulepreload" href="/assets/loader/ores-web-loader.js">"#));
        assert!(html.contains(r#"type="module""#));
        assert!(html.contains(r#"nonce="n0nce""#));
        assert!(!html.contains("//"), "loader assets must be same-origin paths");
    }

    #[test]
    fn the_mount_declares_the_app_release_and_asset_base() {
        let context = context_with(Some("https://assets.indiebuild.dev/releases/abc123/manifest.json"));
        let html = island_mount(&context, "run-graph", maud::html! { p { "fallback" } }).into_string();
        assert!(html.contains(r#"data-ores-app="gha-indie-worker-web""#));
        assert!(html.contains(r#"data-release="abc123""#));
        assert!(html.contains(r#"data-assets="/assets/releases/abc123/""#));
        assert!(
            html.contains("fallback"),
            "the mount must degrade to server-rendered content"
        );
    }

    #[test]
    fn the_mount_reports_whether_islands_are_compiled_in() {
        let html = island_mount(&context_with(None), "x", maud::html! {}).into_string();
        let expected = if cfg!(feature = "islands") {
            r#"data-enabled="true""#
        } else {
            r#"data-enabled="false""#
        };
        assert!(html.contains(expected));
    }

    #[test]
    fn prefetch_happens_on_intent_not_on_load() {
        let html = open_app_link(&context_with(None), "Open app").into_string();
        assert!(html.contains(r#"hx-trigger="mouseenter once, focus once""#));
        assert!(html.contains(r#"hx-get="/_prefetch/release""#));
        // The hints themselves are not in the page that was merely loaded.
        assert!(!html.contains(r#"rel="prefetch""#));
    }

    #[test]
    fn hints_reference_only_same_origin_release_assets() {
        let context = context_with(Some("https://assets.indiebuild.dev/releases/abc123/manifest.json"));
        let html = release_prefetch_hint(&context).into_string();
        assert!(html.contains(r#"href="/assets/releases/abc123/app.js""#));
        assert!(html.contains(r#"href="/assets/releases/abc123/app.wasm""#));
        assert!(!html.contains("https://assets.indiebuild.dev"));
    }
}
