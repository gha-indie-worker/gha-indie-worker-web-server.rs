# Pilot: the shared WASM loading layer

This repository is the Rust half of the pilot for
[OWLS](https://github.com/ores-wasm-loaders) — the fleet's shared loading layer for marketing
sites and the WASM applications behind them. `gha-indie-worker-flutter` is the Flutter half,
and the browser matrix belongs in
[`gha-indie-worker-test`](https://github.com/gha-indie-worker-test) so this org's Actions
minutes stay free.

## What has landed here

One CI job. Every build derives a release manifest from the files the build actually emitted
and validates it against the public contract. That is all: no page wiring, no preparation for
visitors, no change to server behavior.

It is deliberately the first thing to land. A loader's integrity guarantees rest entirely on
the manifest being true about its build — sizes and digests included — so a manifest that is
generated and checked in CI is the precondition for everything after it.

## What stage 1 needs, and what has to be decided first

See the [pilot plan](https://github.com/ores-wasm-loaders/owls-docs/blob/main/docs/pilot-plan.md),
which states entry, exit and stop conditions per stage.

1. Serve releases from an immutable, content-addressed path with the right headers:
   `application/wasm` (streaming instantiation needs it, and the failure is silent), immutable
   caching on release assets, and a manifest that is *not* immutable, because it is the pointer
   that moves.
2. On the marketing surface, emit declarative hints — the zero-JavaScript tier already open in
   [`gha-indie-worker.github.io#3`](https://github.com/gha-indie-worker/gha-indie-worker.github.io/pull/3) —
   and prepare **one** destination on intent. Nothing on page load.
3. On the application surface, activate through the matching adapter. The page must work with
   preparation disabled entirely; that is an acceptance test, not an aspiration.

Then measure, and only then claim: bytes and requests after the click, cold versus prepared;
time to *useful interaction*, not "the loader resolved"; and the bytes spent preparing for
visitors who never clicked. A pilot that finds no improvement and says so is a successful pilot.

## What the loader will never do on a marketing page

Run application code. Preparation is fetch-only and capability-scoped: no script insertion, no
dynamic import of an entrypoint, no auth, no writes, no subscriptions. For Flutter
specifically, the generated bootstrap is never executed to "warm the loader" — running it
starts the application.
