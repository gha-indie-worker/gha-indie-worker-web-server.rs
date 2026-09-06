//! Detect whether htmx has been vendored into `assets/`.
//!
//! htmx is **not** committed to this repository and is **not** loaded from a CDN. `just vendor`
//! (or `python3 scripts/vendor-htmx.py`) downloads a pinned version, verifies its SHA-384 against
//! `assets/htmx.min.js.sha384`, and writes both files; CI runs it before `cargo test`.
//!
//! When the pair is present the binary embeds it and serves it from our own origin. When it is
//! absent the binary still builds, still runs, and still works — every form on every page is a
//! real `<form method="post">` that posts and re-renders without any script at all. The pages
//! simply lose the fragment swaps. That is the whole reason the forms were written that way.

fn main() {
    println!("cargo:rerun-if-changed=assets/htmx.min.js");
    println!("cargo:rerun-if-changed=assets/htmx.min.js.sha384");
    println!("cargo:rustc-check-cfg=cfg(htmx_vendored)");
    let script = std::path::Path::new("assets/htmx.min.js");
    let pin = std::path::Path::new("assets/htmx.min.js.sha384");
    if script.is_file() && pin.is_file() {
        println!("cargo:rustc-cfg=htmx_vendored");
    } else {
        println!(
            "cargo:warning=htmx is not vendored (assets/htmx.min.js[.sha384]); run \
             `python3 scripts/vendor-htmx.py`. The server will serve pages without fragment swaps."
        );
    }
}
