#![forbid(unsafe_code)]
//! Static assets, embedded in the binary, with their integrity computed from the bytes served.
//!
//! **Why not a CDN.** Loading htmx from `unpkg.com` makes every page in this product depend on a
//! third party's ability to keep an account safe, and on a network path we do not control. An
//! `integrity` attribute would narrow that to a denial of service rather than a compromise, but
//! the version we are pinned to is still whatever the registry decides to serve. Vendoring the
//! file and serving it from our own origin removes the party entirely; the `integrity` attribute
//! is then belt-and-braces, and `connect-src`/`script-src 'self'` in the CSP mean a page could not
//! reach a CDN even if a template tried.
//!
//! **Why the digest is computed at boot.** A digest transcribed into a template is a digest that
//! goes stale the next time the file changes, and a stale one fails closed in a way that looks
//! like an outage. [`crate::sri`] hashes the exact bytes this process will serve, once, at start
//! up. The *supply-chain* check — that those bytes are the htmx release we intended — is a
//! separate, committed pin, verified by `scripts/vendor-htmx.py` and by CI.

use std::sync::OnceLock;

use crate::sri;

/// The stylesheet. Committed, so it is always embedded.
const APP_CSS: &str = include_str!("../assets/app.css");
/// The live-log-tail client. Committed; it is ours.
const TAIL_JS: &str = include_str!("../assets/tail.js");

#[cfg(htmx_vendored)]
const HTMX_JS: Option<&[u8]> = Some(include_bytes!("../assets/htmx.min.js"));
#[cfg(not(htmx_vendored))]
const HTMX_JS: Option<&[u8]> = None;

#[cfg(htmx_vendored)]
const HTMX_PIN: Option<&str> = Some(include_str!("../assets/htmx.min.js.sha384"));
#[cfg(not(htmx_vendored))]
const HTMX_PIN: Option<&str> = None;

/// One served file.
#[derive(Clone, Copy, Debug)]
pub struct Asset {
    /// The path it is served at, which is also the `src`/`href` written into the page.
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

/// The registry: every asset, with its `integrity` value.
#[derive(Debug)]
pub struct Assets {
    pub app_css: (Asset, String),
    pub tail_js: (Asset, String),
    /// `None` when htmx was not vendored at build time.
    pub htmx_js: Option<(Asset, String)>,
}

impl Assets {
    /// Look an asset up by request path.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<(&Asset, &str)> {
        for entry in [
            Some(&self.app_css),
            Some(&self.tail_js),
            self.htmx_js.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if entry.0.path == path {
                return Some((&entry.0, entry.1.as_str()));
            }
        }
        None
    }

    /// Whether htmx is available, and therefore whether pages should render fragment-swapping
    /// attributes at all.
    #[must_use]
    pub const fn htmx_available(&self) -> bool {
        self.htmx_js.is_some()
    }

    /// Problems worth logging at boot. An empty list means the vendored file is the pinned one.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        match (HTMX_JS, HTMX_PIN) {
            (None, _) => out.push(
                "htmx is not vendored; pages will render without fragment swaps. Run \
                 `python3 scripts/vendor-htmx.py`."
                    .to_owned(),
            ),
            (Some(bytes), Some(pin)) if !sri::matches_pin(bytes, pin) => out.push(
                "assets/htmx.min.js does not match assets/htmx.min.js.sha384 — the vendored file \
                 is not the release that was reviewed. Refusing to serve it."
                    .to_owned(),
            ),
            _ => {}
        }
        out
    }
}

/// The registry, built once.
///
/// If the vendored htmx does not match its committed pin it is **dropped**, not served: an
/// unexpected script on our own origin is worse than no script, and the pages work without it.
#[must_use]
pub fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let app_css = Asset {
            path: "/assets/app.css",
            content_type: "text/css; charset=utf-8",
            bytes: APP_CSS.as_bytes(),
        };
        let tail_js = Asset {
            path: "/assets/tail.js",
            content_type: "text/javascript; charset=utf-8",
            bytes: TAIL_JS.as_bytes(),
        };
        let htmx_js = match (HTMX_JS, HTMX_PIN) {
            (Some(bytes), Some(pin)) if sri::matches_pin(bytes, pin) => Some((
                Asset {
                    path: "/assets/htmx.min.js",
                    content_type: "text/javascript; charset=utf-8",
                    bytes,
                },
                sri::integrity(bytes),
            )),
            _ => None,
        };
        Assets {
            app_css: (app_css, sri::integrity(app_css.bytes)),
            tail_js: (tail_js, sri::integrity(tail_js.bytes)),
            htmx_js,
        }
    })
}

#[cfg(test)]
mod tests {
    /// The committed pins. Only the tests read them: the bytes themselves are embedded in the
    /// binary, so at run time there is nothing to verify them against -- the only thing that can
    /// drift is the pin file, and that is a build-time fact, not a run-time one.
    const APP_CSS_PIN: &str = include_str!("../assets/app.css.sha384");
    const TAIL_JS_PIN: &str = include_str!("../assets/tail.js.sha384");

    use super::*;

    /// The one pin the repository can enforce on its own: `assets/app.css.sha384` must be the
    /// digest of `assets/app.css`. If somebody edits the stylesheet without re-running
    /// `scripts/pin-assets.py`, this fails here rather than in a browser.
    #[test]
    fn the_files_this_repository_owns_match_their_committed_pins() {
        for (name, bytes, pin) in [
            ("assets/app.css", APP_CSS.as_bytes(), APP_CSS_PIN),
            ("assets/tail.js", TAIL_JS.as_bytes(), TAIL_JS_PIN),
        ] {
            assert!(
                sri::matches_pin(bytes, pin),
                "{name}.sha384 is stale; run `python3 scripts/pin-assets.py`. Expected {}",
                sri::integrity(bytes)
            );
        }
    }

    #[test]
    fn every_asset_has_a_distinct_path_and_an_integrity_value() {
        let assets = assets();
        let mut paths = vec![assets.app_css.0.path, assets.tail_js.0.path];
        if let Some((asset, _)) = &assets.htmx_js {
            paths.push(asset.path);
        }
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), paths.len(), "two assets share a path");

        for path in paths {
            let (asset, integrity) = assets.get(path).expect("registered asset");
            assert!(!asset.bytes.is_empty(), "{path} is empty");
            assert!(
                integrity.starts_with("sha384-"),
                "{path} has no integrity value"
            );
            assert_eq!(
                integrity,
                sri::integrity(asset.bytes),
                "{path} integrity is not its bytes"
            );
        }
        assert!(assets.get("/assets/../etc/passwd").is_none());
        assert!(assets.get("/assets/htmx.js").is_none());
    }

    #[test]
    fn htmx_is_never_served_unless_it_matches_its_pin() {
        // Whatever the build produced, the invariant is the same: the registry offers htmx only
        // when the vendored bytes and the committed pin agree.
        let available = assets().htmx_available();
        let agrees =
            matches!((HTMX_JS, HTMX_PIN), (Some(bytes), Some(pin)) if sri::matches_pin(bytes, pin));
        assert_eq!(available, agrees);
        if !available {
            assert!(
                !assets().warnings().is_empty(),
                "a missing htmx must be reported at boot"
            );
        }
    }
}
