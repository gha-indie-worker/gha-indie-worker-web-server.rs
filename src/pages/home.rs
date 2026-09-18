#![forbid(unsafe_code)]

pub fn markup() -> String {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>IndieBuild · CI/CD control plane</title>
  <style>
    :root { color-scheme: dark; font-family: ui-sans-serif, system-ui, sans-serif; }
    body { margin:0; min-height:100vh; background:#090d12; color:#edf2f7; display:grid; place-items:center; }
    main { width:min(880px,calc(100vw - 40px)); }
    .eyebrow { color:#7bdcb5; text-transform:uppercase; font-size:12px; font-weight:900; letter-spacing:.14em; }
    h1 { font-size:clamp(38px,7vw,76px); line-height:1; margin:12px 0 18px; }
    p { color:#aeb9c8; font-size:18px; line-height:1.55; max-width:720px; }
    .actions { display:flex; gap:12px; margin-top:28px; flex-wrap:wrap; }
    a { padding:11px 15px; border-radius:8px; text-decoration:none; font-weight:800; }
    .primary { background:#7bdcb5; color:#07110d; } .secondary { border:1px solid #324057; color:#d6deea; }
    .features { margin-top:48px; display:grid; grid-template-columns:repeat(3,1fr); gap:12px; }
    .feature { border:1px solid #253042; border-radius:10px; padding:16px; background:#101720; color:#aeb9c8; }
    .feature strong { display:block; color:#fff; margin-bottom:6px; }
    @media(max-width:720px){ .features{grid-template-columns:1fr;} }
  </style>
</head>
<body><main>
  <div class="eyebrow">GHA Indie Worker</div>
  <h1>Build, deploy, and understand your infrastructure.</h1>
  <p>IndieBuild is the Rust-first CI/CD control plane for pipelines, deployments, application health, infrastructure drift, runners, and error correlation.</p>
  <div class="actions"><a class="primary" href="/login">Sign in</a><a class="secondary" href="/app/overview">Open app</a></div>
  <div class="features">
    <div class="feature"><strong>Pipelines</strong>Run attempts, job DAGs, steps, logs, artifacts and deployment links.</div>
    <div class="feature"><strong>Applications & infra</strong>Desired/observed resource trees, environments, plans and drift.</div>
    <div class="feature"><strong>Errors & ecosystem</strong>ores-err-trace correlation plus measurable ORES component adoption.</div>
  </div>
</main></body></html>"#.into()
}
