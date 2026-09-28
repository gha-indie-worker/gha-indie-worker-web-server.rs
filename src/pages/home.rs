#![forbid(unsafe_code)]

pub fn markup(api_http_base: Option<&str>) -> String {
    let api = api_http_base.unwrap_or("http://127.0.0.1:18090");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>IndieBuild</title></head><body><main><h1>GHA Indie Worker</h1><p>Local standalone web surface is running.</p><p>API: <code>{api}</code></p><p>Authentication is provided by Shared Auth through the Cloudflare edge; the SDK test applications exercise signup and login without React or a webview.</p></main></body></html>"
    )
}
