#![forbid(unsafe_code)]
//! Shared application state, and the per-request context every page handler starts from.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;
use gha_indie_worker_lib_core::runtime::surface;

use crate::auth::{MagicLinkStore, Session, SessionStore};
use crate::bridge;
use crate::config::WebConfig;
use crate::csrf;
use crate::persistence::Store;
use crate::present::Face;

/// Seconds since the epoch. Passed into every pure rule rather than read inside one, so expiry is
/// testable.
#[must_use]
pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Everything a handler is given. Cheap to clone: one `Arc` for the config, and two handles.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<WebConfig>,
    pub sessions: SessionStore,
    /// Outstanding magic links. Separate from sessions because a link is not a credential yet.
    pub magic: MagicLinkStore,
    pub store: Store,
    /// When this process started, for `/healthz`.
    pub started_at: i64,
}

impl AppState {
    #[must_use]
    pub fn new(config: WebConfig) -> Self {
        Self {
            config: Arc::new(config),
            sessions: SessionStore::new(),
            magic: MagicLinkStore::new(),
            store: Store::with_fixtures(),
            started_at: now_seconds(),
        }
    }

    /// The `Host` header, lowercased, with the port left on: it is part of the origin we compare
    /// against.
    #[must_use]
    pub fn host_of(&self, headers: &HeaderMap) -> String {
        headers
            .get(axum::http::header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    }

    /// Resolve the request into everything a page needs: which surface it arrived on, who is
    /// signed in, and the CSRF token its forms must carry.
    ///
    /// Returns `None` when the `Host` is not one of ours or names a surface this binary refuses —
    /// the caller answers 404, exactly as `surface::dispose` decided.
    #[must_use]
    pub fn context(&self, headers: &HeaderMap) -> Option<Ctx> {
        let host = self.host_of(headers);
        let face = bridge::face(surface::resolve(&host, &self.config.apex)?)?;
        let cookie_header = headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok());
        let session = csrf::cookie_from(cookie_header, csrf::SESSION_COOKIE)
            .and_then(|id| self.sessions.lookup(id, now_seconds()));
        let cookie_token = csrf::cookie_from(cookie_header, csrf::COOKIE_NAME);

        // For a signed-in request the server's own record is authoritative; otherwise the cookie
        // is, and if there is not a usable one yet we mint it now and set it on the way out.
        let (token, refresh_cookie) = match session.as_ref() {
            Some(session) => {
                let matches = cookie_token
                    .is_some_and(|value| csrf::constant_time_eq(value, &session.csrf_token));
                (session.csrf_token.clone(), !matches)
            }
            None => match cookie_token.filter(|value| csrf::is_well_formed(value)) {
                Some(existing) => (existing.to_owned(), false),
                None => (crate::auth::random_token(), true),
            },
        };

        Some(Ctx {
            face,
            host,
            session,
            csrf_token: token,
            refresh_csrf_cookie: refresh_cookie,
            origin: headers
                .get(axum::http::header::ORIGIN)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        })
    }
}

/// One request's view of the world.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub face: Face,
    pub host: String,
    pub session: Option<Session>,
    /// The token every form on this page carries, and the value the CSRF cookie must hold.
    pub csrf_token: String,
    /// True when the response must (re)issue the CSRF cookie.
    pub refresh_csrf_cookie: bool,
    pub origin: Option<String>,
}

impl Ctx {
    #[must_use]
    pub const fn authenticated(&self) -> bool {
        self.session.is_some()
    }

    /// The token recorded server-side for this session, if any. `None` before sign-in, which is
    /// what makes the pre-session forms fall back to plain double-submit.
    #[must_use]
    pub fn bound_token(&self) -> Option<&str> {
        self.session
            .as_ref()
            .map(|session| session.csrf_token.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;

    fn headers(host: &str, cookie: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(header::HOST, host.parse().unwrap());
        if let Some(cookie) = cookie {
            map.insert(header::COOKIE, cookie.parse().unwrap());
        }
        map
    }

    fn state() -> AppState {
        let mut config = WebConfig::from_env();
        config.apex = "indiebuild.dev".to_owned();
        config.public_url = "https://app.indiebuild.dev".to_owned();
        AppState::new(config)
    }

    #[test]
    fn a_host_that_is_not_ours_has_no_context_at_all() {
        let state = state();
        for host in [
            "app.indiebuild.dev.evil.test",
            "admin.indiebuild.dev",
            "admin-api.indiebuild.dev",
            "api.indiebuild.dev",
            "auth.indiebuild.dev",
            "",
        ] {
            assert!(state.context(&headers(host, None)).is_none(), "host {host}");
        }
    }

    #[test]
    fn each_product_host_resolves_to_its_face() {
        let state = state();
        for (host, face) in [
            ("indiebuild.dev", Face::Marketing),
            ("www.indiebuild.dev", Face::Marketing),
            ("user.indiebuild.dev", Face::User),
            ("org.indiebuild.dev", Face::Org),
            ("app.indiebuild.dev", Face::App),
            ("m.indiebuild.dev", Face::Mobile),
        ] {
            let ctx = state.context(&headers(host, None)).expect("a product host");
            assert_eq!(ctx.face, face, "host {host}");
            assert!(!ctx.authenticated());
        }
    }

    #[test]
    fn an_anonymous_visitor_is_given_a_fresh_csrf_token_to_set() {
        let state = state();
        let ctx = state
            .context(&headers("user.indiebuild.dev", None))
            .unwrap();
        assert!(
            ctx.refresh_csrf_cookie,
            "there was no cookie, so one must be issued"
        );
        assert!(csrf::is_well_formed(&ctx.csrf_token));
        assert_eq!(ctx.bound_token(), None);

        // A cookie that is already well formed is kept, so a reload does not rotate the token
        // out from under a form the visitor is halfway through filling in.
        let existing = crate::auth::random_token();
        let ctx = state
            .context(&headers(
                "user.indiebuild.dev",
                Some(&format!("giw_csrf={existing}")),
            ))
            .unwrap();
        assert_eq!(ctx.csrf_token, existing);
        assert!(!ctx.refresh_csrf_cookie);
    }

    #[test]
    fn a_signed_in_request_uses_the_token_the_server_recorded() {
        let state = state();
        let session = crate::auth::Session {
            id: crate::auth::random_token(),
            subject: "sub".into(),
            email: "alex@acme.test".into(),
            display_name: "Alex".into(),
            account: gha_indie_worker_lib_core::runtime::tenancy::AccountKind::Organization,
            organization: Some("acme".into()),
            role: gha_indie_worker_lib_core::runtime::tenancy::Role::Owner,
            csrf_token: crate::auth::random_token(),
            expires_at: now_seconds() + 3_600,
        };
        let cookie = format!(
            "giw_session={}; giw_csrf={}",
            session.id, session.csrf_token
        );
        let bound = session.csrf_token.clone();
        state.sessions.insert(session);

        let ctx = state
            .context(&headers("app.indiebuild.dev", Some(&cookie)))
            .unwrap();
        assert!(ctx.authenticated());
        assert_eq!(ctx.csrf_token, bound);
        assert_eq!(ctx.bound_token(), Some(bound.as_str()));
        assert!(
            !ctx.refresh_csrf_cookie,
            "the cookie already matched the record"
        );
    }

    #[test]
    fn a_csrf_cookie_that_disagrees_with_the_session_is_replaced_not_trusted() {
        let state = state();
        let session = crate::auth::Session {
            id: crate::auth::random_token(),
            subject: "sub".into(),
            email: "alex@acme.test".into(),
            display_name: "Alex".into(),
            account: gha_indie_worker_lib_core::runtime::tenancy::AccountKind::Individual,
            organization: None,
            role: gha_indie_worker_lib_core::runtime::tenancy::Role::Owner,
            csrf_token: crate::auth::random_token(),
            expires_at: now_seconds() + 3_600,
        };
        let planted = crate::auth::random_token();
        let cookie = format!("giw_session={}; giw_csrf={planted}", session.id);
        let bound = session.csrf_token.clone();
        state.sessions.insert(session);

        let ctx = state
            .context(&headers("app.indiebuild.dev", Some(&cookie)))
            .unwrap();
        assert_eq!(
            ctx.csrf_token, bound,
            "the server record wins over the cookie"
        );
        assert!(
            ctx.refresh_csrf_cookie,
            "the planted cookie must be overwritten"
        );
    }
}
