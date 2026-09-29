#![forbid(unsafe_code)]

const NAV: &[(&str, &str)] = &[
    ("overview", "Overview"),
    ("applications", "Applications"),
    ("pipelines", "Pipelines"),
    ("deployments", "Deployments"),
    ("environments", "Environments"),
    ("infrastructure", "Infrastructure"),
    ("runners", "Runners"),
    ("errors", "Errors"),
    ("artifacts", "Artifacts & caches"),
    ("ecosystem", "Ecosystem"),
    ("audit", "Audit"),
    ("settings", "Settings"),
];

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn markup(active: &str) -> String {
    let active = if NAV.iter().any(|(slug, _)| *slug == active) {
        active
    } else {
        "overview"
    };
    let title = NAV
        .iter()
        .find_map(|(slug, title)| (*slug == active).then_some(*title))
        .unwrap_or("Overview");
    let nav = NAV
        .iter()
        .map(|(slug, label)| {
            let current = if *slug == active {
                " aria-current=\"page\""
            } else {
                ""
            };
            format!(
                "<a class=\"nav-item{}\" href=\"/app/{}\"{}>{}</a>",
                if *slug == active { " active" } else { "" },
                slug,
                current,
                escape_text(label)
            )
        })
        .collect::<String>();

    let main = match active {
        "overview" => overview(),
        "applications" => applications(),
        "pipelines" => pipelines(),
        "deployments" => deployments(),
        "environments" => environments(),
        "infrastructure" => infrastructure(),
        "runners" => runners(),
        "errors" => errors(),
        "artifacts" => artifacts(),
        "ecosystem" => ecosystem(),
        "audit" => audit(),
        "settings" => settings(),
        _ => overview(),
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>{title} · IndieBuild</title>
  <style>
    :root {{ color-scheme: dark; font-family: ui-sans-serif, system-ui, sans-serif; background:#090d12; color:#edf2f7; }}
    * {{ box-sizing:border-box; }} body {{ margin:0; }}
    .shell {{ min-height:100vh; display:grid; grid-template-columns:230px 1fr; }}
    aside {{ border-right:1px solid #253042; background:#0d131b; padding:20px 14px; position:sticky; top:0; height:100vh; }}
    .brand {{ padding:4px 10px 20px; font-weight:900; letter-spacing:.06em; }}
    .brand small {{ display:block; color:#7bdcb5; font-size:10px; letter-spacing:.14em; margin-top:3px; }}
    nav {{ display:grid; gap:3px; }} .nav-item {{ color:#aab7c7; padding:9px 10px; border-radius:7px; text-decoration:none; font-size:14px; }}
    .nav-item:hover,.nav-item.active {{ color:#fff; background:#172130; }} .nav-item.active {{ box-shadow:inset 3px 0 #7bdcb5; }}
    main {{ padding:24px 30px 60px; max-width:1500px; width:100%; }}
    .top {{ display:flex; justify-content:space-between; align-items:center; gap:20px; margin-bottom:24px; }}
    h1 {{ margin:0; font-size:25px; }} h2 {{ font-size:16px; margin:0 0 14px; }}
    .meta {{ color:#8492a6; font-size:13px; }} .pill {{ display:inline-flex; align-items:center; gap:6px; border:1px solid #304057; border-radius:999px; padding:5px 9px; color:#b8c4d4; font-size:12px; }}
    .dot {{ width:7px; height:7px; border-radius:50%; background:#7bdcb5; }}
    .grid {{ display:grid; grid-template-columns:repeat(4,minmax(150px,1fr)); gap:12px; margin-bottom:18px; }}
    .card,.panel {{ border:1px solid #253042; background:#101720; border-radius:10px; }} .card {{ padding:15px; }} .panel {{ padding:16px; margin-bottom:14px; overflow:auto; }}
    .kpi {{ font-size:26px; font-weight:800; margin-top:6px; }} .danger {{ color:#ff8c8c; }} .warn {{ color:#f0c674; }} .good {{ color:#7bdcb5; }}
    table {{ width:100%; border-collapse:collapse; min-width:720px; }} th,td {{ padding:10px 9px; border-bottom:1px solid #202b3b; text-align:left; font-size:13px; }} th {{ color:#7f8da1; font-size:11px; text-transform:uppercase; letter-spacing:.06em; }}
    code {{ color:#bfd4ff; }} .status {{ font-size:11px; border:1px solid #34445c; border-radius:999px; padding:3px 7px; white-space:nowrap; }}
    .tree {{ font-family:ui-monospace,SFMono-Regular,monospace; line-height:1.85; color:#b9c8da; }} .tree strong {{ color:#fff; }}
    .dag {{ display:flex; align-items:center; gap:9px; flex-wrap:wrap; }} .node {{ border:1px solid #34445c; border-radius:8px; padding:9px 12px; background:#131d29; }} .arrow {{ color:#5e7088; }}
    .notice {{ border-left:3px solid #7bdcb5; padding:10px 12px; background:#111c21; color:#b8c4d4; margin-bottom:14px; }}
    @media(max-width:900px) {{ .shell {{ grid-template-columns:1fr; }} aside {{ position:static;height:auto;border-right:0;border-bottom:1px solid #253042; }} nav {{ grid-template-columns:repeat(3,1fr); }} main {{ padding:18px; }} .grid {{ grid-template-columns:repeat(2,1fr); }} }}
  </style>
</head>
<body>
<div class="shell">
  <aside><div class="brand">INDIEBUILD<small>CI / CD CONTROL PLANE</small></div><nav>{nav}</nav></aside>
  <main>
    <div class="top"><div><h1>{title}</h1><div class="meta">Organization-scoped operational view</div></div><span class="pill"><span class="dot"></span> authenticated</span></div>
    {main}
  </main>
</div>
</body>
</html>"#,
        title = escape_text(title),
        nav = nav,
        main = main
    )
}

fn overview() -> &'static str {
    r#"<div class="notice">Canonical execution/deployment/infra state comes from Neon/PostgreSQL. Personal view state belongs in Supabase. Error groups come from <code>ORESoftware/ores-err-trace</code>.</div>
<div class="grid">
 <div class="card"><div class="meta">Applications healthy</div><div class="kpi good">—</div></div>
 <div class="card"><div class="meta">Runs active</div><div class="kpi">—</div></div>
 <div class="card"><div class="meta">Deployments waiting</div><div class="kpi warn">—</div></div>
 <div class="card"><div class="meta">Unresolved errors</div><div class="kpi danger">—</div></div>
</div>
<div class="panel"><h2>Recent activity</h2><table><thead><tr><th>Type</th><th>Repository / app</th><th>State</th><th>Revision</th><th>Age</th></tr></thead><tbody><tr><td colspan="5" class="meta">Connect the read-only projection API to populate this view.</td></tr></tbody></table></div>"#
}

fn applications() -> &'static str {
    r#"<div class="panel"><h2>Desired / observed resource tree</h2><div class="tree"><strong>application</strong> checkout-api <span class="status">health —</span> <span class="status">sync —</span><br>├─ deployment / revision<br>│  ├─ service / endpoint<br>│  └─ workload / instances<br>└─ database / cache / external dependencies</div></div><div class="panel"><h2>Resource details</h2><div class="meta">Summary · Desired/Observed · Diff · Events · Errors · External links</div></div>"#
}

fn pipelines() -> &'static str {
    r#"<div class="panel"><h2>Run attempt DAG</h2><div class="dag"><span class="node">checkout</span><span class="arrow">→</span><span class="node">build</span><span class="arrow">→</span><span class="node">test</span><span class="arrow">→</span><span class="node">package</span><span class="arrow">→</span><span class="node">deploy</span></div></div><div class="panel"><h2>Selected job / steps / live log</h2><table><thead><tr><th>#</th><th>Step</th><th>State</th><th>Duration</th><th>Error groups</th></tr></thead><tbody><tr><td colspan="5" class="meta">Run → attempt → job → step data will come from the canonical control-plane schema.</td></tr></tbody></table></div>"#
}

fn deployments() -> &'static str {
    r#"<div class="panel"><h2>Deployment history</h2><table><thead><tr><th>Application</th><th>Environment</th><th>Revision</th><th>State</th><th>Approval</th><th>Result</th></tr></thead><tbody><tr><td colspan="6" class="meta">created → waiting → approved → deploying → verifying → completed</td></tr></tbody></table></div>"#
}

fn environments() -> &'static str {
    r#"<div class="panel"><h2>Environment protection</h2><table><thead><tr><th>Name</th><th>Kind</th><th>Branch policy</th><th>Required approvals</th><th>Current revision</th></tr></thead><tbody><tr><td colspan="5" class="meta">Development, preview, staging, production, DR and custom targets are modeled explicitly.</td></tr></tbody></table></div>"#
}

fn infrastructure() -> &'static str {
    r#"<div class="panel"><h2>Stacks and drift</h2><table><thead><tr><th>Stack</th><th>Provider</th><th>Engine</th><th>Environment</th><th>Drift</th><th>Last plan</th></tr></thead><tbody><tr><td colspan="6" class="meta">AWS · GCP · Azure · Cloudflare · Kubernetes · Vercel · Neon · Supabase · Upstash · provider-native resources</td></tr></tbody></table></div>"#
}

fn runners() -> &'static str {
    r#"<div class="panel"><h2>Worker pools</h2><table><thead><tr><th>Worker</th><th>Pool</th><th>Platform</th><th>State</th><th>Leases</th><th>Disk / load</th></tr></thead><tbody><tr><td colspan="6" class="meta">Backed by existing workers + worker_heartbeats canonical tables.</td></tr></tbody></table></div>"#
}

fn errors() -> &'static str {
    r#"<div class="notice">No duplicate IndieBuild error table: this page is a projection of <code>public.error_tracking</code> from <code>ORESoftware/ores-err-trace</code>, correlated through provider refs and <code>giw.error_trace_links</code>.</div><div class="panel"><h2>Error groups</h2><table><thead><tr><th>Fingerprint</th><th>Service</th><th>Priority</th><th>Occurrences</th><th>Last seen</th><th>Linked run / deployment</th></tr></thead><tbody><tr><td colspan="6" class="meta">Unresolved priority + recurrence is the default sort.</td></tr></tbody></table></div>"#
}

fn artifacts() -> &'static str {
    r#"<div class="panel"><h2>Artifacts and caches</h2><div class="meta">Immutable artifact identity, provenance/SBOM, retention, cache hit/miss/eviction and provider references.</div></div>"#
}

fn ecosystem() -> &'static str {
    r#"<div class="panel"><h2>ORES adoption matrix</h2><table><thead><tr><th>Repository</th><th>api-docs</th><th>ores-otel</th><th>ores-err-trace</th><th>middleware</th><th>TJSV/contracts</th></tr></thead><tbody><tr><td colspan="6" class="meta">Statuses: current · outdated · missing · misconfigured · blocked · unknown. Discovery covers ORESoftware/ores-* plus ores-* GitHub organizations and critical non-prefix tooling.</td></tr></tbody></table></div>"#
}

fn audit() -> &'static str {
    r#"<div class="panel"><h2>Audit trail</h2><table><thead><tr><th>Time</th><th>Actor</th><th>Action</th><th>Resource</th><th>Request</th></tr></thead><tbody><tr><td colspan="5" class="meta">Append-only tenant audit projection.</td></tr></tbody></table></div>"#
}

fn settings() -> &'static str {
    r#"<div class="panel"><h2>Organization settings</h2><div class="meta">Repositories · provider connections · auth · policies · notifications · API tokens · billing/limits. Provider credentials remain server-side secret references.</div></div>"#
}

#[cfg(test)]
mod tests {
    use super::markup;

    #[test]
    fn shell_exposes_operational_navigation_and_error_owner() {
        let pipelines = markup("pipelines");
        for label in [
            "Applications",
            "Pipelines",
            "Deployments",
            "Infrastructure",
            "Runners",
            "Errors",
            "Ecosystem",
            "Audit",
        ] {
            assert!(pipelines.contains(label));
        }
        assert!(pipelines.contains("aria-current=\"page\""));
        assert!(!pipelines.contains("React"));

        let errors = markup("errors");
        assert!(errors.contains("ORESoftware/ores-err-trace"));
        assert!(errors.contains("public.error_tracking"));
    }

    #[test]
    fn unknown_section_falls_back_to_overview() {
        let html = markup("not-a-section");
        assert!(html.contains("<title>Overview · IndieBuild</title>"));
    }
}
