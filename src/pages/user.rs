#![forbid(unsafe_code)]
//! `user.` — the B2C surface: one person, signing up for themselves.
//!
//! Sign-in is a **magic link**: an address, an email, a single-use token that expires in fifteen
//! minutes. There is no password field anywhere in this product, so there is no password to
//! phish, reset, reuse or store.
//!
//! The other thing this surface owes the visitor is the **hand-off**. Somebody who arrived here,
//! typed their work address, and then realised they are actually setting this up for their
//! company should not have to delete an account and start again. `/for-your-team` is a visible,
//! permanent link — in the sign-in form, in the settings page, and in the footer — that carries
//! the address they already typed across to `org.`, so the second attempt starts where the first
//! one stopped.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Response;
use gha_indie_worker_lib_core::runtime::tenancy::{self, AccountKind, OnboardingError, Role};
use maud::{html, Markup};

use crate::auth::{self, Intent, Session, SESSION_TTL_SECONDS};
use crate::csrf::{self, CookieLife};
use crate::error::WebError;
use crate::present::{explain, Field, Page};
use crate::state::{now_seconds, AppState, Ctx};

use super::forms::{csrf_field, error_block, wants_fragment, Feedback};
use super::layout::surface_url;

/// Render a `user.` page.
#[must_use]
pub fn render(state: &AppState, ctx: &Ctx, page: &Page, query: &[(String, String)]) -> Markup {
    let prefill = csrf::form_value(query, "email").unwrap_or_default();
    match page {
        Page::UserSignIn => sign_in(state, ctx, false, prefill),
        Page::UserSignUp => sign_in(state, ctx, true, prefill),
        Page::UserCheckEmail => check_email(state, ctx, prefill),
        Page::UserCallback => signing_in(),
        Page::UserSettings => settings(state, ctx),
        Page::UserSwitchToOrg => switch_to_org(state, ctx, prefill),
        _ => super::not_found(ctx),
    }
}

fn sign_in(state: &AppState, ctx: &Ctx, new_account: bool, prefill: &str) -> Markup {
    let intent = Intent::Individual;
    let team = surface_url(
        &state.config.apex,
        state.config.cookies_are_secure(),
        "user",
        "/for-your-team",
    );
    html! {
        div class="wrap-narrow stack" {
            header class="stack-tight" {
                p class="eyebrow" { "For yourself" }
                h1 { @if new_account { "Create your personal account" } @else { "Sign in" } }
                p class="lede" {
                    "We will email you a link. It works once and expires in fifteen minutes."
                }
            }

            div class="card" {
                // The error slot is itself the swap target, replaced `outerHTML`. That is what
                // keeps a swapped fragment from adding a second element with the same `id`, which
                // would leave `aria-describedby` pointing at whichever one the browser found first.
                form method="post" action="/auth/magic-link"
                    hx-post="/auth/magic-link" hx-target="#invite-email-error"
                    hx-swap="outerHTML" class="stack" {
                    (csrf_field(&ctx.csrf_token))
                    input type="hidden" name="intent" value=(intent.as_str());
                    div class="field" {
                        label for="invite-email" { "Email address" }
                        input id="invite-email" type="email" name="email" value=(prefill)
                            autocomplete="email" inputmode="email" required
                            aria-describedby="invite-email-error email-hint";
                        span class="hint" id="email-hint" { "Work or personal — either is fine." }
                        (Feedback::Quiet.slot(Field::Email))
                    }
                    div class="row" {
                        button type="submit" class="button button-primary button-large" {
                            @if new_account { "Create my account" } @else { "Email me a link" }
                        }
                        span class="busy faint" aria-live="polite" { "Sending…" }
                    }
                }
            }

            // The hand-off. Deliberately not a footnote: it is a card of its own, with the same
            // visual weight as the form above it.
            aside class="panel stack-tight" aria-labelledby="handoff-heading" {
                h2 id="handoff-heading" { "Actually, this is for my company" }
                p class="muted" {
                    "Organizations get seats, invitations, roles and domain verification. Setting \
                     one up now is easier than moving a personal workspace into one later."
                }
                p { a class="button" href=(team) { "Set this up for my company instead" } }
            }
        }
    }
}

fn check_email(state: &AppState, ctx: &Ctx, email: &str) -> Markup {
    html! {
        div class="wrap-narrow stack" {
            h1 { "Check your email" }
            p class="lede" {
                @if email.is_empty() {
                    "If that address has an account, a sign-in link is on its way."
                } @else {
                    "If " (email) " has an account, a sign-in link is on its way."
                }
            }
            p class="muted" { "The link works once and expires in fifteen minutes." }
            (development_notice(state, ctx))
        }
    }
}

fn signing_in() -> Markup {
    html! {
        div class="wrap-narrow stack" {
            h1 { "Signing you in" }
            p class="lede" { "One moment." }
            noscript { p { "If nothing happens, go back and request a new link." } }
        }
    }
}

fn settings(state: &AppState, ctx: &Ctx) -> Markup {
    let Some(session) = &ctx.session else {
        return super::not_found(ctx);
    };
    let team = surface_url(
        &state.config.apex,
        state.config.cookies_are_secure(),
        "org",
        "/new",
    );
    let app = surface_url(
        &state.config.apex,
        state.config.cookies_are_secure(),
        "app",
        "/",
    );
    html! {
        div class="wrap-narrow stack" {
            header class="stack-tight" {
                h1 { "Personal settings" }
                p class="lede" { "Signed in as " span class="mono" { (session.email) } "." }
            }

            section class="card stack" {
                h2 { "Your account" }
                dl class="stack-tight" {
                    div class="row-between" { dt class="muted" { "Email" } dd class="mono" { (session.email) } }
                    div class="row-between" { dt class="muted" { "Account type" } dd {
                        @if session.is_organization() { "Organization member" } @else { "Personal workspace" }
                    } }
                    div class="row-between" { dt class="muted" { "Subject" } dd class="mono faint" { (session.subject) } }
                }
                p { a class="button" href=(app) { "Go to your builds" } }
            }

            section class="card stack" {
                h2 { "Set this up for your company" }
                p class="muted" {
                    "Your personal workspace stays yours. Creating an organization gives you seats \
                     to invite colleagues into, roles, and a domain you can verify — and you can \
                     hold both with this same address."
                }
                p { a class="button button-primary" href=(team) { "Create an organization" } }
            }
        }
    }
}

fn switch_to_org(state: &AppState, ctx: &Ctx, prefill: &str) -> Markup {
    let carry = if prefill.is_empty() {
        "/new".to_owned()
    } else {
        format!("/new?email={}", encode_query_component(prefill))
    };
    let create = surface_url(
        &state.config.apex,
        state.config.cookies_are_secure(),
        "org",
        &carry,
    );
    let back = surface_url(
        &state.config.apex,
        state.config.cookies_are_secure(),
        "user",
        "/",
    );
    html! {
        div class="wrap-narrow stack" {
            header class="stack-tight" {
                p class="eyebrow" { "For your team" }
                h1 { "Setting this up for a company" }
            }
            p class="lede" {
                "Nothing is lost by switching now — you have not created anything yet. An \
                 organization is the right shape when more than one person needs to see the builds."
            }
            div class="card stack" {
                h2 { "What changes" }
                ul class="stack-tight muted" {
                    li { "You buy seats, and each person who can see builds holds one." }
                    li { "You invite people by email, or verify a domain and let colleagues join." }
                    li { "Roles decide who can trigger runs, manage runners, and manage seats." }
                    li { "You become the owner, and an organization always keeps at least one." }
                }
                p class="row" {
                    a class="button button-primary button-large" href=(create) { "Create an organization" }
                    a class="button button-quiet" href=(back) { "No, keep it personal" }
                }
            }
            (development_notice(state, ctx))
        }
    }
}

/// Said out loud on the page, not only in a log: when shared-auth is not configured the sign-in
/// flow is an in-process stub, and nobody should mistake this deployment for a real one.
fn development_notice(state: &AppState, _ctx: &Ctx) -> Markup {
    html! {
        @if !state.config.shared_auth_configured() {
            p class="notice notice-warn" role="status" {
                "Development mode: shared-auth is not configured, so sign-in links are printed by \
                 this server instead of emailed."
            }
        }
    }
}

/// Percent-encode a value for a query string. Only the characters that would change the meaning of
/// the URL are escaped; the rest are left readable.
fn encode_query_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

/// `POST /auth/magic-link`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn request_magic_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let email = csrf::form_value(&form, "email").unwrap_or_default();
    let intent = csrf::form_value(&form, "intent")
        .and_then(Intent::parse)
        .unwrap_or(Intent::Individual);

    match tenancy::parse_email(email) {
        Err(error) => {
            let rendered = error_block(&explain(&crate::bridge::refusal(&error)));
            if wants_fragment(&headers) {
                Ok(super::fragment(
                    &state,
                    &ctx,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    rendered,
                ))
            } else {
                let page = Page::UserSignIn;
                let nonce = auth::random_nonce();
                let body = html! {
                    div class="wrap-narrow stack" {
                        h1 { "Sign in" }
                        (rendered)
                        p { a class="button" href="/" { "Try again" } }
                    }
                };
                Ok(super::respond(
                    &state,
                    &ctx,
                    &page,
                    &nonce,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    body,
                ))
            }
        }
        Ok((normalized, _)) => {
            let link = state.magic.issue(&normalized, intent, now_seconds());
            // In a configured deployment this is where shared-auth sends the mail. Here the link
            // is rendered, and the page says why.
            let development = !state.config.shared_auth_configured();
            let href = format!("/auth/callback?token={}", link.token);
            let rendered = html! {
                (Feedback::Done(Field::Email, "Check your email for a sign-in link.").slot(Field::Email))
                @if development {
                    p class="notice notice-warn" {
                        "Development mode — no mail was sent. "
                        a href=(href) { "Use the link" }
                    }
                }
            };
            if wants_fragment(&headers) {
                Ok(super::fragment(&state, &ctx, StatusCode::OK, rendered))
            } else {
                let target = format!("/check-email?email={}", encode_query_component(&normalized));
                Ok(super::redirect(&state, &ctx, &target))
            }
        }
    }
}

/// `GET /auth/callback?token=…` — redeem a magic link.
///
/// # Errors
/// [`WebError::NotFound`] for a host that is not ours; a spent, unknown or expired token renders
/// the sign-in page with a message rather than an error status, because that is what it is.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, WebError> {
    let ctx = state.context(&headers).ok_or(WebError::NotFound)?;
    let query = csrf::parse_form(uri.query().unwrap_or_default());
    let token = csrf::form_value(&query, "token").unwrap_or_default();
    let now = now_seconds();

    let Some(link) = state.magic.consume(token, now) else {
        let nonce = auth::random_nonce();
        let body = html! {
            div class="wrap-narrow stack" {
                h1 { "That link no longer works" }
                p class="lede" {
                    "Sign-in links work once and expire after fifteen minutes. Ask for a new one."
                }
                p { a class="button button-primary" href="/" { "Get a new link" } }
            }
        };
        return Ok(super::respond(
            &state,
            &ctx,
            &Page::UserSignIn,
            &nonce,
            StatusCode::OK,
            body,
        ));
    };

    let session = Session {
        id: auth::random_token(),
        // Until shared-auth is wired in, the subject is derived from the address rather than
        // issued by an identity provider. It is stable, opaque to the page, and replaced by the
        // real `sub` claim the moment `Introspector` has a real implementation.
        subject: format!("dev:{}", link.email),
        display_name: link.email.split('@').next().unwrap_or("there").to_owned(),
        email: link.email.clone(),
        account: link.intent.account_kind(),
        organization: match link.intent.account_kind() {
            AccountKind::Organization => Some(state.store.organization().slug),
            AccountKind::Individual => None,
        },
        role: Role::Owner,
        csrf_token: auth::random_token(),
        expires_at: now + SESSION_TTL_SECONDS,
    };
    let secure = state.config.cookies_are_secure();
    let session_cookie = csrf::set_cookie(
        csrf::SESSION_COOKIE,
        &session.id,
        state.config.cookie_domain(),
        CookieLife::Seconds(SESSION_TTL_SECONDS),
        secure,
        true,
    );
    let csrf_cookie = csrf::set_cookie(
        csrf::COOKIE_NAME,
        &session.csrf_token,
        state.config.cookie_domain(),
        CookieLife::Session,
        secure,
        true,
    );
    state.sessions.insert(session);

    let target = match link.intent {
        Intent::Individual => surface_url(&state.config.apex, secure, "app", "/"),
        Intent::Organization => surface_url(&state.config.apex, secure, "org", "/members"),
        Intent::CreateOrganization => surface_url(&state.config.apex, secure, "org", "/new"),
    };
    let mut response = super::redirect(&state, &ctx, &target);
    super::attach_cookie(&mut response, &session_cookie);
    super::attach_cookie(&mut response, &csrf_cookie);
    Ok(response)
}

/// `POST /auth/signout`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn sign_out(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, _form) = super::accept_post(&state, &headers, &body)?;
    if let Some(session) = &ctx.session {
        state.sessions.remove(&session.id);
    }
    let secure = state.config.cookies_are_secure();
    let target = surface_url(&state.config.apex, secure, "user", "/");
    let mut response = super::redirect_fragment(&state, &ctx, &target);
    for name in [csrf::SESSION_COOKIE, csrf::COOKIE_NAME] {
        let cleared = csrf::set_cookie(
            name,
            "",
            state.config.cookie_domain(),
            CookieLife::Expire,
            secure,
            true,
        );
        super::attach_cookie(&mut response, &cleared);
    }
    Ok(response)
}

/// A tenancy refusal, as it should be shown on this surface. Kept next to the handler that needs
/// it so the mapping from `OnboardingError` to markup is never more than one hop.
#[must_use]
pub fn refusal_markup(error: &OnboardingError) -> Markup {
    error_block(&explain(&crate::bridge::refusal(error)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_component_cannot_escape_the_url_it_is_put_in() {
        assert_eq!(encode_query_component("alex@acme.test"), "alex%40acme.test");
        assert_eq!(encode_query_component("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(encode_query_component("../../etc"), "..%2F..%2Fetc");
        assert_eq!(
            encode_query_component("plain-name_1.0~x"),
            "plain-name_1.0~x"
        );
        // And it round-trips through the parser the callback uses.
        let encoded = encode_query_component("alex+ci@acme.test");
        let parsed = csrf::parse_form(&format!("email={encoded}"));
        assert_eq!(
            csrf::form_value(&parsed, "email"),
            Some("alex+ci@acme.test")
        );
    }

    #[test]
    fn every_onboarding_refusal_renders_something_specific_on_this_surface() {
        let markup = refusal_markup(&OnboardingError::InvalidEmail).into_string();
        assert!(markup.contains("email address"), "{markup}");
        assert!(markup.contains("invite-email-error"), "{markup}");
    }
}
