#![forbid(unsafe_code)]

pub fn markup() -> String {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Sign in · IndieBuild</title>
  <style>
    :root { color-scheme: dark; font-family: ui-sans-serif, system-ui, sans-serif; }
    body { margin: 0; background: #0a0d12; color: #edf2f7; min-height: 100vh; display: grid; place-items: center; }
    main { width: min(440px, calc(100vw - 40px)); border: 1px solid #273244; border-radius: 14px; padding: 28px; background: #111720; box-shadow: 0 20px 70px #0008; }
    .brand { font-weight: 800; letter-spacing: .04em; }
    .eyebrow { color: #7bdcb5; font-size: 12px; font-weight: 800; letter-spacing: .12em; text-transform: uppercase; margin-top: 28px; }
    h1 { margin: 8px 0 10px; font-size: 30px; }
    p { color: #aeb9c8; line-height: 1.55; }
    .button { display: block; text-align: center; margin-top: 22px; padding: 12px 16px; border-radius: 9px; text-decoration: none; color: #07110d; background: #7bdcb5; font-weight: 800; }
    .note { font-size: 13px; border-top: 1px solid #273244; margin-top: 24px; padding-top: 18px; }
    a.secondary { color: #aeb9c8; }
  </style>
</head>
<body>
<main>
  <div class="brand">INDIEBUILD</div>
  <div class="eyebrow">Customer control plane</div>
  <h1>Sign in</h1>
  <p>Authenticate with the IndieBuild product identity service to view your CI/CD pipelines, deployments, application health, infrastructure state, runners, errors, and audit history.</p>
  <a class="button" href="/auth/start">Continue with IndieBuild</a>
  <p class="note"><strong>Fail-closed preview:</strong> the shared-auth exchange is not enabled on this branch yet. <code>/app/**</code> accepts only an already-verified bearer identity and otherwise redirects here. No development identity or browser database credential is used.</p>
  <a class="secondary" href="/">Back to indiebuild.dev</a>
</main>
</body>
</html>"#.into()
}

#[cfg(test)]
mod tests {
    use super::markup;

    #[test]
    fn login_explains_fail_closed_identity_boundary() {
        let html = markup();
        assert!(html.contains("Continue with IndieBuild"));
        assert!(html.contains("Fail-closed preview"));
        assert!(html.contains("/auth/start"));
        assert!(!html.contains("service_role"));
    }
}
