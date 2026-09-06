#![forbid(unsafe_code)]

//! Stateless HTTP to the api-server (`GHA_INDIE_WORKER_API_BASE`).
//!
//! Every **write** goes through here, and the actor's bearer is forwarded
//! verbatim so authorization is decided exactly once — in the api-server, which
//! owns the domain rules. This server never mints a privileged token of its own
//! and never has a service credential that could bypass a user's permissions.
//!
//! Failures are collapsed into [`WebError`] deliberately: an upstream status
//! line or body must never reach a page.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::WebError;

use super::models::{
    AuditEntry, InviteOutcome, OrgMember, OrgSummary, PlanSummary, RunDetail, RunSummary, SeatUsage, TokenSummary,
    WorkerSummary,
};

/// Upper bound on a response body this server will parse.
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct ApiClient {
    base: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApiClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    #[must_use]
    pub fn new(base: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            http,
        }
    }

    /// A client that will never be called; used by unit tests.
    #[must_use]
    pub fn for_tests() -> Self {
        Self::new("http://127.0.0.1:0", default_http_client())
    }

    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, bearer: Option<&str>) -> Result<T, WebError> {
        let mut request = self.http.get(self.url(path));
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|_| WebError::Unavailable)?;
        Self::decode(response).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        bearer: Option<&str>,
        body: &B,
    ) -> Result<T, WebError> {
        let mut request = self.http.post(self.url(path)).json(body);
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|_| WebError::Unavailable)?;
        Self::decode(response).await
    }

    async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, WebError> {
        let status = response.status();
        if !status.is_success() {
            return Err(Self::map_status(status.as_u16()));
        }
        let text = response.text().await.map_err(|_| WebError::Unavailable)?;
        if text.len() > MAX_RESPONSE_BYTES {
            return Err(WebError::Unavailable);
        }
        serde_json::from_str(&text).map_err(|_| WebError::Unavailable)
    }

    /// Upstream status → the error a visitor is allowed to see.
    #[must_use]
    pub const fn map_status(status: u16) -> WebError {
        match status {
            401 => WebError::Unauthenticated,
            403 => WebError::Forbidden,
            404 => WebError::NotFound,
            429 => WebError::RateLimited,
            400 | 409 | 422 => WebError::BadRequest("The api-server rejected that request."),
            _ => WebError::Unavailable,
        }
    }

    // ---- reads -------------------------------------------------------------

    pub async fn runs(&self, bearer: Option<&str>, limit: u64) -> Result<Vec<RunSummary>, WebError> {
        self.get(&format!("/v1/runs?limit={limit}"), bearer).await
    }

    pub async fn run(&self, bearer: Option<&str>, run_id: &str) -> Result<RunDetail, WebError> {
        self.get(&format!("/v1/runs/{}", encode_segment(run_id)), bearer).await
    }

    pub async fn run_log(
        &self,
        bearer: Option<&str>,
        run_id: &str,
        since: Option<u64>,
    ) -> Result<Vec<String>, WebError> {
        let path = match since {
            Some(cursor) => format!("/v1/runs/{}/log?since={cursor}", encode_segment(run_id)),
            None => format!("/v1/runs/{}/log", encode_segment(run_id)),
        };
        self.get(&path, bearer).await
    }

    pub async fn workers(&self, bearer: Option<&str>) -> Result<Vec<WorkerSummary>, WebError> {
        self.get("/v1/workers", bearer).await
    }

    pub async fn plans(&self, bearer: Option<&str>) -> Result<Vec<PlanSummary>, WebError> {
        self.get("/v1/plans", bearer).await
    }

    pub async fn members(&self, bearer: Option<&str>, org_id: &str) -> Result<Vec<OrgMember>, WebError> {
        self.get(&format!("/v1/orgs/{}/members", encode_segment(org_id)), bearer)
            .await
    }

    pub async fn seats(&self, bearer: Option<&str>, org_id: &str) -> Result<SeatUsage, WebError> {
        self.get(&format!("/v1/orgs/{}/seats", encode_segment(org_id)), bearer)
            .await
    }

    pub async fn audit(&self, bearer: Option<&str>, org_id: &str, limit: u64) -> Result<Vec<AuditEntry>, WebError> {
        self.get(
            &format!("/v1/orgs/{}/audit?limit={limit}", encode_segment(org_id)),
            bearer,
        )
        .await
    }

    pub async fn tokens(&self, bearer: Option<&str>) -> Result<Vec<TokenSummary>, WebError> {
        self.get("/v1/me/tokens", bearer).await
    }

    // ---- writes ------------------------------------------------------------

    pub async fn rerun(&self, bearer: Option<&str>, run_id: &str) -> Result<RunSummary, WebError> {
        self.post(
            &format!("/v1/runs/{}/rerun", encode_segment(run_id)),
            bearer,
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn cancel_run(&self, bearer: Option<&str>, run_id: &str) -> Result<RunSummary, WebError> {
        self.post(
            &format!("/v1/runs/{}/cancel", encode_segment(run_id)),
            bearer,
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn create_org(&self, bearer: Option<&str>, name: &str, slug: &str) -> Result<OrgSummary, WebError> {
        self.post("/v1/orgs", bearer, &serde_json::json!({ "name": name, "slug": slug }))
            .await
    }

    pub async fn start_domain_verification(
        &self,
        bearer: Option<&str>,
        org_id: &str,
        domain: &str,
        method: &str,
    ) -> Result<serde_json::Value, WebError> {
        self.post(
            &format!("/v1/orgs/{}/domain", encode_segment(org_id)),
            bearer,
            &serde_json::json!({ "domain": domain, "method": method }),
        )
        .await
    }

    pub async fn allocate_seats(&self, bearer: Option<&str>, org_id: &str, seats: u32) -> Result<SeatUsage, WebError> {
        self.post(
            &format!("/v1/orgs/{}/seats", encode_segment(org_id)),
            bearer,
            &serde_json::json!({ "allocated": seats }),
        )
        .await
    }

    pub async fn invite_members(
        &self,
        bearer: Option<&str>,
        org_id: &str,
        invites: &[(String, String)],
    ) -> Result<Vec<InviteOutcome>, WebError> {
        let payload: Vec<serde_json::Value> = invites
            .iter()
            .map(|(email, role)| serde_json::json!({ "email": email, "role": role }))
            .collect();
        self.post(
            &format!("/v1/orgs/{}/invites", encode_segment(org_id)),
            bearer,
            &serde_json::json!({ "invites": payload }),
        )
        .await
    }

    pub async fn set_member_role(
        &self,
        bearer: Option<&str>,
        org_id: &str,
        member_id: &str,
        role: &str,
    ) -> Result<OrgMember, WebError> {
        self.post(
            &format!(
                "/v1/orgs/{}/members/{}/role",
                encode_segment(org_id),
                encode_segment(member_id)
            ),
            bearer,
            &serde_json::json!({ "role": role }),
        )
        .await
    }

    pub async fn create_token(
        &self,
        bearer: Option<&str>,
        name: &str,
        scopes: &[String],
    ) -> Result<serde_json::Value, WebError> {
        self.post(
            "/v1/me/tokens",
            bearer,
            &serde_json::json!({ "name": name, "scopes": scopes }),
        )
        .await
    }

    pub async fn revoke_token(&self, bearer: Option<&str>, token_id: &str) -> Result<serde_json::Value, WebError> {
        self.post(
            &format!("/v1/me/tokens/{}/revoke", encode_segment(token_id)),
            bearer,
            &serde_json::json!({}),
        )
        .await
    }

    // ---- chat --------------------------------------------------------------

    /// Opens an ores-chat session. `surface` is `visitor` or `customer`.
    pub async fn open_chat_session(
        &self,
        bearer: Option<&str>,
        surface: &str,
        page: &str,
    ) -> Result<serde_json::Value, WebError> {
        self.post(
            "/v1/chat/sessions",
            bearer,
            &serde_json::json!({ "surface": surface, "page": page }),
        )
        .await
    }

    pub async fn send_chat_message(
        &self,
        bearer: Option<&str>,
        session_id: &str,
        message: &str,
    ) -> Result<serde_json::Value, WebError> {
        self.post(
            &format!("/v1/chat/sessions/{}/messages", encode_segment(session_id)),
            bearer,
            &serde_json::json!({ "message": message }),
        )
        .await
    }

    pub async fn chat_messages(&self, bearer: Option<&str>, session_id: &str) -> Result<serde_json::Value, WebError> {
        self.get(
            &format!("/v1/chat/sessions/{}/messages", encode_segment(session_id)),
            bearer,
        )
        .await
    }
}

/// The shared outbound client: bounded, redirect-free, no cookie jar.
#[must_use]
pub fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("gha-indie-worker-web-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Percent-encodes one path segment. Identifiers come from the URL, so a `..`
/// or a `/` must never be able to reshape the upstream path.
#[must_use]
pub fn encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_url_never_keeps_a_trailing_slash() {
        let client = ApiClient::new("https://api.indiebuild.dev/", default_http_client());
        assert_eq!(client.base(), "https://api.indiebuild.dev");
        assert_eq!(client.url("/v1/runs"), "https://api.indiebuild.dev/v1/runs");
    }

    #[test]
    fn path_segments_cannot_escape_their_position() {
        assert_eq!(encode_segment("../../admin"), "..%2F..%2Fadmin");
        assert_eq!(encode_segment("run_1"), "run_1");
        assert_eq!(encode_segment("a b"), "a%20b");
        assert_eq!(encode_segment("a?b=c#d"), "a%3Fb%3Dc%23d");
    }

    #[test]
    fn upstream_statuses_map_to_safe_errors() {
        assert!(matches!(ApiClient::map_status(401), WebError::Unauthenticated));
        assert!(matches!(ApiClient::map_status(403), WebError::Forbidden));
        assert!(matches!(ApiClient::map_status(404), WebError::NotFound));
        assert!(matches!(ApiClient::map_status(429), WebError::RateLimited));
        assert!(matches!(ApiClient::map_status(422), WebError::BadRequest(_)));
        assert!(matches!(ApiClient::map_status(500), WebError::Unavailable));
        assert!(matches!(ApiClient::map_status(502), WebError::Unavailable));
    }

    #[test]
    fn the_debug_view_shows_only_the_base() {
        let text = format!(
            "{:?}",
            ApiClient::new("https://api.indiebuild.dev", default_http_client())
        );
        assert!(text.contains("base: \"https://api.indiebuild.dev\""));
        assert!(
            !text.contains("http:"),
            "no reqwest internals should be printed: {text}"
        );
        assert!(text.contains(".."), "the debug view must stay non-exhaustive");
    }
}
