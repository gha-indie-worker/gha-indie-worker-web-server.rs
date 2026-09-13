#![forbid(unsafe_code)]

//! The single stylesheet, inlined under the request's CSP nonce.
//!
//! The palette, type scale and rules are the ones from the marketing site
//! (`gha-indie-worker.github.io`) so a visitor moving from the Astro home to
//! `app.indiebuild.dev` sees the same product, not two products: same `GHA/IW`
//! mark, same mono eyebrow labels, same accent `#67c2a3`, same hairline rules.
//!
//! Dark is the marketing site's native tone and stays the default; a light
//! palette is provided for `prefers-color-scheme: light`. Nothing here depends
//! on `:hover` for access to a control — mobile and keyboard users get the same
//! affordances, and every focusable element has a visible focus ring.

/// Inlined into `<style nonce=…>` by [`crate::ui::layout::page`].
pub const CSS: &str = r#"
*, *::before, *::after { box-sizing: border-box; }
:root {
  --paper: #0e1317; --ink: #e8eef2; --muted: #93a0aa; --accent: #67c2a3;
  --rule: #29343c; --raised: #131a20; --danger: #e2725b; --warn: #d9a441;
  --sans: "Avenir Next", Avenir, "Segoe UI", system-ui, sans-serif;
  --serif: "Iowan Old Style", "Palatino Linotype", Palatino, Georgia, serif;
  --mono: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  --gutter: 4vw;
  color-scheme: dark light;
}
@media (prefers-color-scheme: light) {
  :root {
    --paper: #f7f9fa; --ink: #12181d; --muted: #55636d; --accent: #2f7d63;
    --rule: #d7dee3; --raised: #ffffff; --danger: #a8341c; --warn: #8a6208;
  }
}
html { background: var(--paper); }
body {
  margin: 0; background: var(--paper); color: var(--ink); font-family: var(--sans);
  font-size: 16px; line-height: 1.55; -webkit-text-size-adjust: 100%;
}
a { color: inherit; }
::selection { background: var(--accent); color: var(--paper); }
:focus-visible { outline: 3px solid var(--accent); outline-offset: 3px; }
.skip-link { position: fixed; left: 1rem; top: 1rem; z-index: 40; padding: .75rem 1rem; background: var(--ink); color: var(--paper); transform: translateY(-200%); }
.skip-link:focus { transform: none; }

/* masthead ---------------------------------------------------------------- */
.masthead {
  position: sticky; top: 0; z-index: 30; display: flex; align-items: center; justify-content: space-between;
  gap: 1.5rem; min-height: 4.5rem; padding: 0 var(--gutter);
  background: color-mix(in srgb, var(--paper) 94%, transparent);
  backdrop-filter: blur(16px); border-bottom: 1px solid var(--rule);
}
.identity { display: inline-flex; align-items: center; gap: .8rem; text-decoration: none; font-weight: 650; letter-spacing: -.02em; }
.mark { font-family: var(--mono); font-size: .7rem; letter-spacing: .08em; color: var(--accent); }
.surface-tag { font: .62rem/1 var(--mono); letter-spacing: .12em; text-transform: uppercase; color: var(--muted); border: 1px solid var(--rule); padding: .3rem .45rem; }
.primary-nav { display: flex; gap: clamp(.8rem, 2.4vw, 2rem); font-family: var(--mono); font-size: .68rem; text-transform: uppercase; letter-spacing: .08em; }
.primary-nav a { text-decoration: none; padding: .65rem 0; border-bottom: 1px solid transparent; }
.primary-nav a[aria-current="page"] { border-color: var(--accent); color: var(--accent); }
.identity-actions { display: flex; align-items: center; gap: .75rem; font-family: var(--mono); font-size: .68rem; }

/* structure --------------------------------------------------------------- */
main { display: block; }
.section { padding: clamp(2.25rem, 5vw, 4rem) var(--gutter); border-bottom: 1px solid var(--rule); }
.section-number, .eyebrow { margin: 0 0 1.25rem; color: var(--accent); font: 700 .68rem/1.4 var(--mono); letter-spacing: .12em; text-transform: uppercase; }
h1 { margin: 0; font: 620 clamp(2.1rem, 5vw, 4.2rem)/1.02 var(--sans); letter-spacing: -.05em; text-wrap: balance; }
h2 { margin: 0 0 1rem; font: 500 clamp(1.4rem, 2.6vw, 2.2rem)/1.1 var(--sans); letter-spacing: -.03em; }
h3 { margin: 0 0 .5rem; font-size: 1.05rem; letter-spacing: -.01em; }
p { color: var(--muted); }
.lede { max-width: 44rem; font-size: clamp(1rem, 1.3vw, 1.18rem); }
.grid { display: grid; gap: 1px; background: var(--rule); border: 1px solid var(--rule); }
.grid-3 { grid-template-columns: repeat(3, 1fr); }
.grid-2 { grid-template-columns: repeat(2, 1fr); }
.card { background: var(--paper); padding: clamp(1.25rem, 2.4vw, 2rem); }
.card > span.label { color: var(--accent); font: .66rem/1 var(--mono); letter-spacing: .08em; text-transform: uppercase; }
.stack { display: flex; flex-direction: column; gap: 1rem; }
.row { display: flex; flex-wrap: wrap; gap: .85rem; align-items: center; }

/* controls ---------------------------------------------------------------- */
.button, button.button {
  display: inline-flex; align-items: center; gap: .6rem; padding: .85rem 1.1rem; border: 1px solid var(--ink);
  background: var(--ink); color: var(--paper); text-decoration: none; cursor: pointer;
  font: 700 .72rem/1 var(--mono); letter-spacing: .05em; text-transform: uppercase;
}
.button:hover, .button:focus-visible { background: var(--accent); border-color: var(--accent); color: var(--paper); }
.button.secondary { background: transparent; color: var(--ink); border-color: var(--rule); }
.button.secondary:hover, .button.secondary:focus-visible { border-color: var(--accent); color: var(--accent); background: transparent; }
.button.danger { background: transparent; color: var(--danger); border-color: var(--danger); }
.text-link { color: var(--muted); font: .74rem/1 var(--mono); text-underline-offset: .3rem; }
.entry-points { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 1px; background: var(--rule); border: 1px solid var(--rule); }
.entry-point { background: var(--paper); padding: clamp(1.5rem, 3vw, 2.5rem); display: flex; flex-direction: column; gap: .75rem; }

/* forms ------------------------------------------------------------------- */
form { margin: 0; }
.field { display: flex; flex-direction: column; gap: .4rem; margin-bottom: 1.1rem; }
.field label { font: .68rem/1.4 var(--mono); text-transform: uppercase; letter-spacing: .08em; color: var(--muted); }
.field input, .field select, .field textarea {
  padding: .8rem .9rem; background: var(--raised); color: var(--ink);
  border: 1px solid var(--rule); font: inherit; font-size: .95rem; width: 100%;
}
.field textarea { min-height: 9rem; font-family: var(--mono); font-size: .82rem; }
.field .hint { color: var(--muted); font-size: .78rem; }
.field .error { color: var(--danger); font-size: .8rem; }
fieldset { border: 1px solid var(--rule); padding: 1.1rem; margin: 0 0 1.1rem; }
legend { padding: 0 .4rem; font: .66rem/1 var(--mono); letter-spacing: .08em; text-transform: uppercase; color: var(--accent); }

/* tables ------------------------------------------------------------------ */
.table-wrap { overflow-x: auto; border: 1px solid var(--rule); }
table { border-collapse: collapse; width: 100%; min-width: 34rem; }
th, td { text-align: left; padding: .8rem 1rem; border-bottom: 1px solid var(--rule); font-size: .88rem; vertical-align: top; }
th { font: .66rem/1 var(--mono); text-transform: uppercase; letter-spacing: .08em; color: var(--muted); }
tbody tr:last-child td { border-bottom: 0; }
td.numeric, th.numeric { text-align: right; font-family: var(--mono); }

/* status ------------------------------------------------------------------ */
.badge { display: inline-flex; align-items: center; gap: .4rem; padding: .25rem .55rem; border: 1px solid var(--rule); font: .64rem/1.4 var(--mono); text-transform: uppercase; letter-spacing: .06em; }
.badge.ok { color: var(--accent); border-color: var(--accent); }
.badge.bad { color: var(--danger); border-color: var(--danger); }
.badge.warn { color: var(--warn); border-color: var(--warn); }
.alert { border: 1px solid var(--rule); border-left-width: 3px; padding: 1rem 1.1rem; margin-bottom: 1.25rem; }
.alert.error { border-left-color: var(--danger); }
.alert.ok { border-left-color: var(--accent); }
.alert h3 { margin-bottom: .35rem; }
.alert p { margin: 0; }
.empty { padding: 2.5rem 1rem; text-align: center; color: var(--muted); border: 1px dashed var(--rule); }

/* wizard ------------------------------------------------------------------ */
.steps { display: flex; flex-wrap: wrap; gap: 1px; background: var(--rule); border: 1px solid var(--rule); margin-bottom: 1.75rem; list-style: none; padding: 0; }
.steps li { flex: 1 1 8rem; background: var(--paper); padding: .8rem 1rem; font: .66rem/1.4 var(--mono); text-transform: uppercase; letter-spacing: .07em; color: var(--muted); }
.steps li .index { color: var(--rule); display: block; }
.steps li[data-state="current"] { color: var(--ink); }
.steps li[data-state="current"] .index, .steps li[data-state="done"] .index { color: var(--accent); }
.steps li[data-state="done"] { color: var(--accent); }

/* log stream -------------------------------------------------------------- */
.logs { background: var(--raised); border: 1px solid var(--rule); max-height: 26rem; overflow: auto; padding: .9rem 1rem; font: .78rem/1.6 var(--mono); white-space: pre-wrap; word-break: break-word; }
.logs .line { display: block; }
.logs .line .ts { color: var(--muted); }
.logs .line.stderr { color: var(--danger); }
.live-dot { width: .45rem; height: .45rem; border-radius: 50%; background: var(--accent); display: inline-block; }

/* chat -------------------------------------------------------------------- */
.chat { position: fixed; right: 1rem; bottom: 1rem; z-index: 35; width: min(22rem, calc(100vw - 2rem)); border: 1px solid var(--rule); background: var(--paper); }
.chat > summary { list-style: none; cursor: pointer; padding: .8rem 1rem; border-bottom: 1px solid transparent; font: 700 .68rem/1 var(--mono); text-transform: uppercase; letter-spacing: .08em; display: flex; justify-content: space-between; gap: .75rem; }
.chat[open] > summary { border-bottom-color: var(--rule); }
.chat > summary::-webkit-details-marker { display: none; }
.chat-body { padding: 1rem; }
.chat-log { max-height: 14rem; overflow: auto; margin-bottom: .75rem; font-size: .85rem; }

/* footer ------------------------------------------------------------------ */
footer.site { display: grid; grid-template-columns: 1fr auto 1fr; gap: 1.5rem; padding: 1.25rem var(--gutter) 5rem; color: var(--muted); font: .64rem/1.4 var(--mono); text-transform: uppercase; letter-spacing: .08em; }
footer.site span:last-child { text-align: right; }

/* compact (m.indiebuild.dev) ---------------------------------------------- */
body.compact { padding-bottom: 4.5rem; --gutter: 5vw; }
body.compact .primary-nav { display: none; }
body.compact .grid-3, body.compact .grid-2, body.compact .entry-points { grid-template-columns: 1fr; }
body.compact h1 { font-size: clamp(1.9rem, 8vw, 2.6rem); }
body.compact .card { padding: 1.1rem var(--gutter); }
body.compact .section { padding: 1.5rem var(--gutter); }
body.compact footer.site { grid-template-columns: 1fr; padding-bottom: 1.25rem; }
body.compact footer.site span:last-child { text-align: left; }
body.compact .chat { right: .5rem; bottom: 4.25rem; }
.bottom-nav { position: fixed; left: 0; right: 0; bottom: 0; z-index: 30; display: none; background: var(--paper); border-top: 1px solid var(--rule); }
body.compact .bottom-nav { display: grid; grid-auto-flow: column; grid-auto-columns: 1fr; }
.bottom-nav a { display: flex; flex-direction: column; align-items: center; gap: .2rem; padding: .7rem .35rem calc(.7rem + env(safe-area-inset-bottom)); text-decoration: none; color: var(--muted); font: .6rem/1.2 var(--mono); text-transform: uppercase; letter-spacing: .06em; min-height: 3rem; }
.bottom-nav a[aria-current="page"] { color: var(--accent); }
.bottom-nav a .glyph { font-size: 1rem; line-height: 1; }

/* responsive -------------------------------------------------------------- */
@media (max-width: 900px) {
  .grid-3 { grid-template-columns: 1fr; }
  .entry-points { grid-template-columns: 1fr; }
}
@media (max-width: 680px) {
  .primary-nav { gap: .75rem; font-size: .62rem; }
  .grid-2 { grid-template-columns: 1fr; }
  footer.site { grid-template-columns: 1fr; }
  footer.site span:last-child { text-align: left; }
}
@media (prefers-reduced-motion: reduce) {
  * { transition: none !important; animation: none !important; }
  html { scroll-behavior: auto; }
}
/* htmx: only ever a progress cue, never a layout dependency. */
.htmx-request .live-dot { opacity: .35; }
.htmx-indicator { display: none; }
.htmx-request .htmx-indicator { display: inline; }
"#;

#[cfg(test)]
mod tests {
    use super::CSS;

    #[test]
    fn the_stylesheet_cannot_break_out_of_its_style_element() {
        assert!(
            !CSS.contains("</style"),
            "a literal </style would terminate the element early"
        );
        assert!(!CSS.contains("<script"));
    }

    #[test]
    fn both_themes_and_the_compact_layout_are_defined() {
        assert!(CSS.contains("prefers-color-scheme: light"));
        assert!(CSS.contains("body.compact"));
        assert!(CSS.contains(".bottom-nav"));
        assert!(CSS.contains("prefers-reduced-motion"));
    }

    #[test]
    fn the_marketing_palette_is_carried_over() {
        for token in ["#0e1317", "#e8eef2", "#93a0aa", "#67c2a3", "#29343c"] {
            assert!(CSS.contains(token), "missing marketing palette token {token}");
        }
    }
}
