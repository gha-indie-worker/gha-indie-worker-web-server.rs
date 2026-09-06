#![forbid(unsafe_code)]
//! Sessions, tokens, and the seam to `shared-auth`.
//!
//! Two things happen here and they are deliberately kept apart.
//!
//! **Verifying a credential** is `lib_core::runtime::authz`'s job. Everything that decides whether
//! a token is acceptable — issuer, audience, realm, expiry, assurance, scopes — is a pure function
//! over an [`Introspection`], exhaustively tested in lib-core. This file supplies the
//! [`Introspector`] that produces one, and nothing else: there is no second copy of the rules
//! here that could disagree with them.
//!
//! **Holding a browser session** is this file's job. A session is a random opaque identifier in a
//! `HttpOnly` cookie plus a server-side record; the cookie is not a token and carries no claims,
//! so it cannot be replayed anywhere but here.
//!
//! The store is in memory. That is a real limitation and it is stated in the PR: two instances do
//! not share sessions, and a deploy signs everyone out. The alternative — a signed cookie
//! containing claims — trades that inconvenience for a credential we cannot revoke, and revocation
//! is worth more than a redeploy.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use gha_indie_worker_lib_core::runtime::authz::{
    self, Audience, Introspection, Requirements, VerifiedActor, VerifyError,
};
use gha_indie_worker_lib_core::runtime::tenancy::{AccountKind, Role};

use crate::config::WebConfig;

/// How long a browser session lives without being renewed.
pub const SESSION_TTL_SECONDS: i64 = 12 * 60 * 60;
/// How long a magic link is good for. Short on purpose: it arrives by email, which is a channel we
/// do not control once it has left.
pub const MAGIC_LINK_TTL_SECONDS: i64 = 15 * 60;

/// A signed-in browser session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub id: String,
    /// The subject from the identity provider — stable, opaque, never an email.
    pub subject: String,
    pub email: String,
    pub display_name: String,
    pub account: AccountKind,
    /// The organization this session is acting in, for a B2B account.
    pub organization: Option<String>,
    pub role: Role,
    /// The CSRF token bound to this session. See [`crate::csrf`].
    pub csrf_token: String,
    pub expires_at: i64,
}

impl Session {
    #[must_use]
    pub const fn is_organization(&self) -> bool {
        matches!(self.account, AccountKind::Organization)
    }

    /// What the account can do inside its organization. An individual account is the owner of its
    /// own personal workspace, which is why the role is not `Option`.
    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }
}

/// Server-side session records. Cheap to clone; the lock is only ever held for a map operation.
#[derive(Clone, Debug, Default)]
pub struct SessionStore {
    inner: Arc<RwLock<HashMap<String, Session>>>,
}

impl SessionStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look a session up and drop it if it has expired. Expiry is checked on read rather than by a
    /// sweeper, so a clock that jumps forward cannot leave a session alive.
    #[must_use]
    pub fn lookup(&self, id: &str, now: i64) -> Option<Session> {
        let found = {
            let guard = self.inner.read().ok()?;
            guard.get(id).cloned()
        };
        match found {
            Some(session) if session.expires_at > now => Some(session),
            Some(_) => {
                self.remove(id);
                None
            }
            None => None,
        }
    }

    pub fn insert(&self, session: Session) {
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(session.id.clone(), session);
        }
    }

    pub fn remove(&self, id: &str) {
        if let Ok(mut guard) = self.inner.write() {
            guard.remove(id);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().map(|guard| guard.len()).unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Which door a magic link was requested at. The link lands on the surface it was asked for, so a
/// person who started at `org.` is never dropped into the individual settings page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Intent {
    /// `user.` — an individual signing in or signing up for themselves.
    Individual,
    /// `org.` — someone signing in to an organization.
    Organization,
    /// `org./new` — someone creating an organization.
    CreateOrganization,
}

impl Intent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Intent::Individual => "individual",
            Intent::Organization => "organization",
            Intent::CreateOrganization => "create-organization",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "individual" => Intent::Individual,
            "organization" => Intent::Organization,
            "create-organization" => Intent::CreateOrganization,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn account_kind(self) -> AccountKind {
        match self {
            Intent::Individual => AccountKind::Individual,
            Intent::Organization | Intent::CreateOrganization => AccountKind::Organization,
        }
    }
}

/// An outstanding magic link.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MagicLink {
    pub token: String,
    pub email: String,
    pub intent: Intent,
    pub expires_at: i64,
}

/// Outstanding magic links, single use.
#[derive(Clone, Debug, Default)]
pub struct MagicLinkStore {
    inner: Arc<RwLock<HashMap<String, MagicLink>>>,
}

impl MagicLinkStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn issue(&self, email: &str, intent: Intent, now: i64) -> MagicLink {
        let link = MagicLink {
            token: random_token(),
            email: email.to_ascii_lowercase(),
            intent,
            expires_at: now + MAGIC_LINK_TTL_SECONDS,
        };
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(link.token.clone(), link.clone());
        }
        link
    }

    /// Redeem a link. It is removed whether or not it had expired: a token that has been presented
    /// once is spent, and leaving an expired one in the map only gives it a second chance.
    #[must_use]
    pub fn consume(&self, token: &str, now: i64) -> Option<MagicLink> {
        let taken = self.inner.write().ok()?.remove(token)?;
        (taken.expires_at > now).then_some(taken)
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------------------------

/// 32 bytes of operating-system randomness as 64 lowercase hex characters.
///
/// Used for session identifiers, CSRF tokens and magic links alike. `rand::random` is seeded from
/// the OS and is the crate's cryptographically secure default; the value is never derived from a
/// counter, a timestamp or a user identifier, all of which have been session-prediction bugs.
#[must_use]
pub fn random_token() -> String {
    let bytes: [u8; 32] = rand::random();
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// A fresh CSP nonce: 16 bytes as 22 base64url characters, matching
/// [`crate::policy::nonce_is_well_formed`].
#[must_use]
pub fn random_nonce() -> String {
    let bytes: [u8; 16] = rand::random();
    crate::wire::base64url(&bytes)
}

// ---------------------------------------------------------------------------------------------
// The shared-auth seam
// ---------------------------------------------------------------------------------------------

/// Produces an RFC 7662 introspection answer for a bearer token.
///
/// The only implementation in this crate is [`NoIntrospector`]. The real one is an HTTP client
/// against `SHARED_AUTH_BASE_URL` using `SHARED_AUTH_INTROSPECTION_CREDENTIAL`, and it lives
/// behind this trait so that adding it cannot change the rules in
/// `lib_core::runtime::authz::project`, which is where a mistake would actually matter.
pub trait Introspector: Send + Sync + std::fmt::Debug {
    /// # Errors
    /// [`VerifyError::Inactive`] when the token cannot be introspected at all.
    fn introspect(&self, token: &str) -> Result<Introspection, VerifyError>;
}

/// Refuses every token. Active whenever shared-auth is not configured, which is every environment
/// except the deployed ones — and it fails closed rather than trusting anything.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoIntrospector;

impl Introspector for NoIntrospector {
    fn introspect(&self, _token: &str) -> Result<Introspection, VerifyError> {
        Err(VerifyError::Inactive)
    }
}

/// Verify an `Authorization` header against the rules in lib-core.
///
/// Every failure returns the same [`VerifyError`] shape and every caller renders the same 401:
/// saying which check failed is an oracle for finding a token that passes more of them.
///
/// # Errors
/// Any [`VerifyError`].
pub fn verify_bearer(
    header: Option<&str>,
    introspector: &dyn Introspector,
    config: &WebConfig,
    now: i64,
) -> Result<VerifiedActor, VerifyError> {
    let token = authz::bearer(header)?;
    let introspection = introspector.introspect(token)?;
    let requirements = Requirements::new(&config.shared_auth_issuer, Audience::Web);
    authz::project(&introspection, &requirements, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, expires_at: i64) -> Session {
        Session {
            id: id.to_owned(),
            subject: "sub_01HZ".into(),
            email: "alex@acme.test".into(),
            display_name: "Alex".into(),
            account: AccountKind::Organization,
            organization: Some("acme".into()),
            role: Role::Owner,
            csrf_token: random_token(),
            expires_at,
        }
    }

    #[test]
    fn tokens_are_long_random_and_well_formed_for_the_csrf_check() {
        let first = random_token();
        let second = random_token();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        assert!(crate::csrf::is_well_formed(&first));
        assert!(first
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    }

    #[test]
    fn nonces_are_shaped_the_way_the_content_security_policy_requires() {
        for _ in 0..16 {
            let nonce = random_nonce();
            assert!(crate::policy::nonce_is_well_formed(&nonce), "nonce {nonce}");
        }
    }

    #[test]
    fn an_expired_session_is_forgotten_on_the_first_read() {
        let store = SessionStore::new();
        store.insert(session("live", 100));
        store.insert(session("dead", 100));
        assert_eq!(store.len(), 2);
        assert!(store.lookup("live", 99).is_some());
        assert!(store.lookup("dead", 101).is_none());
        assert_eq!(
            store.len(),
            1,
            "the expired record was dropped, not just hidden"
        );
        assert!(store.lookup("never-existed", 0).is_none());
    }

    #[test]
    fn a_magic_link_works_once_and_only_before_it_expires() {
        let store = MagicLinkStore::new();
        let link = store.issue("Alex@Acme.Test", Intent::Organization, 1_000);
        assert_eq!(
            link.email, "alex@acme.test",
            "the address is normalised when it is issued"
        );
        assert_eq!(link.expires_at, 1_000 + MAGIC_LINK_TTL_SECONDS);
        assert_eq!(store.consume(&link.token, 1_100).as_ref(), Some(&link));
        assert_eq!(
            store.consume(&link.token, 1_100),
            None,
            "a link is single use"
        );

        let stale = store.issue("alex@acme.test", Intent::Individual, 1_000);
        assert_eq!(
            store.consume(&stale.token, 1_000 + MAGIC_LINK_TTL_SECONDS + 1),
            None
        );
        assert_eq!(
            store.consume(&stale.token, 1_000),
            None,
            "an expired link is spent too"
        );
    }

    #[test]
    fn intents_round_trip_and_choose_the_account_kind() {
        for intent in [
            Intent::Individual,
            Intent::Organization,
            Intent::CreateOrganization,
        ] {
            assert_eq!(Intent::parse(intent.as_str()), Some(intent));
        }
        assert_eq!(Intent::parse("admin"), None);
        assert_eq!(Intent::Individual.account_kind(), AccountKind::Individual);
        assert_eq!(
            Intent::CreateOrganization.account_kind(),
            AccountKind::Organization
        );
    }

    #[test]
    fn without_shared_auth_every_bearer_token_is_refused() {
        let config = WebConfig::from_env();
        assert!(verify_bearer(Some("Bearer abc"), &NoIntrospector, &config, 0).is_err());
        assert_eq!(
            verify_bearer(None, &NoIntrospector, &config, 0).unwrap_err(),
            VerifyError::MissingBearer
        );
        assert_eq!(
            verify_bearer(Some("Basic abc"), &NoIntrospector, &config, 0).unwrap_err(),
            VerifyError::MissingBearer
        );
    }
}
