# `/assets/loader`

`ores-web-loader.js` is served from here, same-origin, so the
Content-Security-Policy stays `script-src 'self' 'nonce-…'` with no CDN.

The file itself is produced by the
[`ORESoftware/ores-web-loader`](https://github.com/ORESoftware/ores-web-loader)
pipeline and copied in by the deploy job — it is not vendored from a public
package and is not committed here.

Pages that carry a Leptos/Dioxus island emit:

```html
<link rel="modulepreload" href="/assets/loader/ores-web-loader.js">
<script type="module" src="/assets/loader/ores-web-loader.js" nonce="…"></script>
<div data-ores-app="gha-indie-worker-web"
     data-ores-island="…"
     data-release="…"
     data-assets="/assets/releases/<release>/">…server-rendered fallback…</div>
```

`src/ui/loader.rs` renders those tags. Islands are behind the `islands` cargo
feature (default OFF), so with no loader present the mount keeps its
server-rendered fallback and nothing breaks.
