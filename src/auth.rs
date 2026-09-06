#![forbid(unsafe_code)]

//! Dual authentication for the web surfaces.
//!
//! Two credentials are accepted, and they map onto the same [`Actor`]:
//!
//! 1. the **cookie session** minted by the shared-auth code exchange
//!    ([`crate::session`]) — how a browser is signed in;
//! 2. an **`Authorization: Bearer …`** on API-ish htmx calls, which is either a
//!    shared-auth token (introspected against `SHARED_AUTH_BASE`) or a
//!    Supabase/Neon session JWT verified locally against cached JWKS.
//!
//! shared-auth federates Supabase Auth and Neon Auth, so both shapes are
//! legitimate for a product surface; they are distinguished only by
//! [`ActorSource`] so downstream code can log which path was taken.
//!
//! **Admin instances are never accepted here.** This process holds the product
//! audience and product introspection secret only. A token carrying an admin
//! role, or issued for an admin audience, is rejected outright — the admin
//! console lives behind Cloudflare Access on its own host and its own server.

pub mod jwks;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ores_middleware::{AuthDecision, AuthVerifier, IntegrationError, RequestMetadata};
use shared_auth_client::{Introspection, SharedAuthClient};
use thiserror::Error;

use crate::config::WebConfig;
use crate::hosts::{HostPolicy, Surface};
use crate::session::{Actor, ActorSource, SessionCodec};

/// Roles that only ever exist on the admin instance. Seeing one on this server
/// means a token crossed a boundary it must not cross.
pub const ADMIN_ROLES: &[&str] = &[
    "admin",
    "superuser",
    "platform_admin",
    "internal_support",
    "owner_internal",
    "fleet_operator",
];

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("no credential was presented")]
    Missing,
    #[error("the credential is not valid")]
    Invalid,
    #[error("an admin-instance credential was presented to a product surface")]
    AdminCredentialRejected,
    #[error("the identity provider could not be reached")]
    ProviderUnavailable,
    #[error("shared-auth is not configured on this deployment")]
    NotConfigured,
}

impl From<AuthError> for crate::WebError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Missing | AuthError::Invalid => Self::Unauthenticated,
            AuthError::AdminCredentialRejected => Self::Forbidden,
            AuthError::ProviderUnavailable | AuthError::NotConfigured => Self::Unavailable,
        }
    }
}

/// Everything needed to turn a credential into an [`Actor`].
pub struct AuthContext {
    shared: Option<SharedAuthClient>,
    audience: Option<String>,
    jwks: jwks::JwksCache,
    supabase_jwks_url: Option<String>,
    neon_jwks_url: Option<String>,
    accepted_issuers: Vec<String>,
    accepted_audiences: Vec<String>,
}

impl std::fmt::Debug for AuthContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthContext")
            .field("shared_auth_configured", &self.shared.is_some())
            .field("audience", &self.audience)
            .field("accepted_issuers", &self.accepted_issuers)
            .finish_non_exhaustive()
    }
}

impl AuthContext {
    /// Builds the context from configuration. Construction performs no network
    /// I/O, so a bad `SHARED_AUTH_BASE` surfaces on the first request rather
    /// than as a boot panic.
    #[must_use]
    pub fn from_config(config: &WebConfig, http: reqwest::Client) -> Self {
        let shared = config.shared_auth.base.as_ref().and_then(|base| {
            let client = SharedAuthClient::try_new(base.clone()).ok()?;
            Some(match config.shared_auth.introspect_secret.as_ref() {
                Some(secret) => client.with_service_credential(secret.clone()),
                None => client,
            })
        });
        Self {
            shared,
            audience: config.shared_auth.audience.clone(),
            jwks: jwks::JwksCache::new(http, config.jwt.jwks_ttl_seconds),
            supabase_jwks_url: config.jwt.supabase_jwks_url.clone(),
            neon_jwks_url: config.jwt.neon_auth_jwks_url.clone(),
            accepted_issuers: config.jwt.accepted_issuers.clone(),
            accepted_audiences: config.jwt.accepted_audiences.clone(),
        }
    }

    #[must_use]
    pub fn shared_auth(&self) -> Option<&SharedAuthClient> {
        self.shared.as_ref()
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.shared.is_some()
    }

    /// Reads shared-auth's advertised capabilities (MFA methods, SAML, SCIM).
    /// Used by the org SSO page so the placeholder reflects what is actually on.
    pub async fn capabilities(&self) -> Result<shared_auth_client::Capabilities, AuthError> {
        let client = self.shared.as_ref().ok_or(AuthError::NotConfigured)?;
        client.capabilities().await.map_err(|_| AuthError::ProviderUnavailable)
    }

    /// Verifies a bearer credential and maps it onto an [`Actor`].
    ///
    /// shared-auth introspection is tried first when configured; a token it does
    /// not recognise is then verified as a federated Supabase/Neon JWT.
    pub async fn verify_bearer(&self, token: &str) -> Result<Actor, AuthError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(AuthError::Missing);
        }
        if let Some(client) = &self.shared {
            match self.introspect(client, token).await {
                Ok(Some(actor)) => return Ok(actor),
                Ok(None) => {}
                Err(error @ AuthError::AdminCredentialRejected) => return Err(error),
                Err(_) => {}
            }
        }
        self.verify_federated_jwt(token).await
    }

    async fn introspect(&self, client: &SharedAuthClient, token: &str) -> Result<Option<Actor>, AuthError> {
        let introspection = match self.audience.as_deref() {
            Some(audience) => client.introspect_for_audience(token, audience).await,
            None => client.introspect(token).await,
        };
        let introspection = match introspection {
            Ok(value) => value,
            Err(shared_auth_client::ClientError::Unauthorized) => return Ok(None),
            Err(_) => return Err(AuthError::ProviderUnavailable),
        };
        if !introspection.active {
            return Ok(None);
        }
        if roles_contain_admin(&introspection.roles) {
            return Err(AuthError::AdminCredentialRejected);
        }
        Ok(Some(actor_from_introspection(&introspection, token)))
    }

    async fn verify_federated_jwt(&self, token: &str) -> Result<Actor, AuthError> {
        let mut urls: Vec<&str> = Vec::new();
        if let Some(url) = &self.supabase_jwks_url {
            urls.push(url);
        }
        if let Some(url) = &self.neon_jwks_url {
            urls.push(url);
        }
        if urls.is_empty() {
            return Err(AuthError::Invalid);
        }
        for url in urls {
            match self
                .jwks
                .verify(url, token, &self.accepted_issuers, &self.accepted_audiences)
                .await
            {
                Ok(claims) => {
                    if roles_contain_admin(&claims.roles()) {
                        return Err(AuthError::AdminCredentialRejected);
                    }
                    return Ok(claims.into_actor(token));
                }
                Err(jwks::JwksError::Unavailable) => return Err(AuthError::ProviderUnavailable),
                Err(_) => {}
            }
        }
        Err(AuthError::Invalid)
    }
}

/// `Authorization: Bearer <token>` → `<token>`. Case-insensitive scheme.
#[must_use]
pub fn bearer_token(value: &str) -> Option<&str> {
    let value = value.trim();
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

#[must_use]
pub fn roles_contain_admin(roles: &[String]) -> bool {
    roles
        .iter()
        .any(|role| ADMIN_ROLES.iter().any(|admin| role.eq_ignore_ascii_case(admin)))
}

/// shared-auth carries the organization either as an explicit `org_id` claim or
/// as the federated `provider_tenant`. Both are accepted, `org_id` wins.
#[must_use]
pub fn actor_from_introspection(introspection: &Introspection, token: &str) -> Actor {
    let org_id = introspection
        .rest
        .get("org_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| introspection.provider_tenant.clone());
    Actor {
        sub: introspection.sub.clone().unwrap_or_default(),
        email: introspection.email.clone(),
        org_id,
        roles: introspection.roles.clone(),
        scopes: introspection
            .scope
            .as_deref()
            .map(|scope| scope.split_ascii_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        acr: introspection.acr.clone(),
        source: ActorSource::SharedAuthBearer,
        access_token: token.to_owned(),
    }
}

/// ores-middleware's auth stage, backed by the same two credentials.
///
/// It is deliberately **permissive about absence**: the marketing home and the
/// login pages are anonymous, so no credential is not an error. A credential
/// that is present and bad, or that carries an admin role, fails the request
/// before it ever reaches a handler.
pub struct WebAuthVerifier {
    auth: Arc<AuthContext>,
    sessions: Arc<SessionCodec>,
    hosts: Arc<HostPolicy>,
}

impl WebAuthVerifier {
    #[must_use]
    pub const fn new(auth: Arc<AuthContext>, sessions: Arc<SessionCodec>, hosts: Arc<HostPolicy>) -> Self {
        Self { auth, sessions, hosts }
    }

    fn surface_of(&self, headers: &BTreeMap<String, String>) -> Surface {
        headers
            .get("host")
            .map_or(Surface::Unknown, |host| self.hosts.surface_for_host(host))
    }
}

impl AuthVerifier for WebAuthVerifier {
    fn verify<'a>(
        &'a self,
        request: &'a RequestMetadata,
    ) -> Pin<Box<dyn Future<Output = Result<AuthDecision, IntegrationError>> + Send + 'a>> {
        Box::pin(async move {
            // 1. Bearer, if the caller sent one.
            if let Some(actor) = request
                .headers
                .get("authorization")
                .and_then(|value| bearer_token(value))
            {
                return match self.auth.verify_bearer(actor).await {
                    Ok(actor) => Ok(decision_for(&actor)),
                    Err(AuthError::AdminCredentialRejected) => Err(IntegrationError {
                        code: "admin_credential_rejected",
                        message: "admin-instance credentials are not accepted by product surfaces".into(),
                    }),
                    Err(AuthError::ProviderUnavailable) => Err(IntegrationError {
                        code: "auth_provider_unavailable",
                        message: "the identity provider did not answer".into(),
                    }),
                    Err(_) => Err(IntegrationError {
                        code: "invalid_credential",
                        message: "the presented credential is not valid".into(),
                    }),
                };
            }

            // 2. Cookie session, scoped to the surface the request arrived on.
            let surface = self.surface_of(&request.headers);
            if let Some(raw) = request
                .headers
                .get("cookie")
                .and_then(|cookies| cookie_from_header(cookies, self.sessions.cookie_name()))
            {
                if let Some(session) = self.sessions.decode(&raw, surface, crate::session::now_unix()) {
                    if roles_contain_admin(&session.roles) {
                        return Err(IntegrationError {
                            code: "admin_credential_rejected",
                            message: "admin-instance credentials are not accepted by product surfaces".into(),
                        });
                    }
                    return Ok(decision_for(&Actor::from_session(&session)));
                }
            }

            // 3. Anonymous. Marketing, login and signup pages live here.
            Ok(AuthDecision::default())
        })
    }
}

fn decision_for(actor: &Actor) -> AuthDecision {
    let mut claims = BTreeMap::new();
    if let Some(acr) = &actor.acr {
        claims.insert("acr".to_owned(), acr.clone());
    }
    claims.insert(
        "auth.source".to_owned(),
        match actor.source {
            ActorSource::Session => "session",
            ActorSource::SharedAuthBearer => "shared-auth",
            ActorSource::FederatedJwt => "federated-jwt",
        }
        .to_owned(),
    );
    AuthDecision {
        user_id: Some(actor.sub.clone()),
        tenant_id: actor.org_id.clone(),
        claims,
    }
}

fn cookie_from_header(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.trim().split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_tokens_are_parsed_case_insensitively() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer  abc "), Some("abc"));
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("Bearer "), None);
        assert_eq!(bearer_token("abc"), None);
    }

    #[test]
    fn admin_roles_are_detected_regardless_of_case() {
        assert!(roles_contain_admin(&["Admin".into()]));
        assert!(roles_contain_admin(&["member".into(), "platform_admin".into()]));
        assert!(!roles_contain_admin(&["member".into(), "org_owner".into()]));
        assert!(!roles_contain_admin(&[]));
    }

    #[test]
    fn cookie_header_parsing_finds_the_named_cookie() {
        assert_eq!(
            cookie_from_header("a=1; giw_session=xyz; b=2", "giw_session").as_deref(),
            Some("xyz")
        );
        assert_eq!(cookie_from_header("a=1", "giw_session"), None);
        assert_eq!(cookie_from_header("", "giw_session"), None);
    }

    #[test]
    fn an_auth_error_maps_to_a_safe_web_error() {
        assert!(matches!(
            crate::WebError::from(AuthError::Invalid),
            crate::WebError::Unauthenticated
        ));
        assert!(matches!(
            crate::WebError::from(AuthError::AdminCredentialRejected),
            crate::WebError::Forbidden
        ));
        assert!(matches!(
            crate::WebError::from(AuthError::NotConfigured),
            crate::WebError::Unavailable
        ));
    }
}
