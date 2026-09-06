#![forbid(unsafe_code)]
//! `org.` — the B2B surface: seats, invitations, members, domains, SSO.
//!
//! Every mutation on this surface goes through `lib_core::runtime::tenancy` and every refusal it
//! can return is rendered **next to the control that caused it**, with a message written for that
//! case and a way out. The mapping is `crate::present::explain`, it is total, and
//! `crate::bridge::refusal` is a `match` on the real enum — so a new `OnboardingError` variant
//! stops this crate compiling until somebody writes its sentence.
//!
//! The seat ledger is the spine of the page. It is recomputed from members and open invitations on
//! every render, never stored, and it is shown *before* the invite form rather than after the
//! refusal, so "we are out of seats" is something you see rather than something you discover.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use gha_indie_worker_lib_core::runtime::tenancy::{Invitation, OnboardingError, Role, Seats};
use maud::{html, Markup};

use crate::csrf;
use crate::error::WebError;
use crate::persistence::{DomainClaim, Member, Organization};
use crate::present::{explain, Field, Page};
use crate::state::{now_seconds, AppState, Ctx};

use super::forms::{csrf_field, error_block, wants_fragment, Feedback};
use super::layout::surface_url;

/// Render an `org.` page.
#[must_use]
pub fn render(state: &AppState, ctx: &Ctx, page: &Page) -> Markup {
    let now = now_seconds();
    let organization = state.store.organization();
    match page {
        Page::OrgSignIn => sign_in(state, ctx),
        Page::OrgCreate => create_form(state, ctx),
        Page::OrgMembers => members_page(ctx, &organization),
        Page::OrgInvitations => invitations_page(ctx, &organization, now),
        Page::OrgDomains => domains_page(ctx, &organization),
        Page::OrgSso => sso_page(&organization),
        Page::OrgSettings => settings_page(ctx, &organization, now),
        _ => super::not_found(ctx),
    }
}

fn actor_role(ctx: &Ctx) -> Role {
    ctx.session
        .as_ref()
        .map_or(Role::Viewer, |session| session.role())
}

// ---------------------------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------------------------

fn sign_in(state: &AppState, ctx: &Ctx) -> Markup {
    let personal = surface_url(
        &state.config.apex,
        state.config.cookies_are_secure(),
        "user",
        "/",
    );
    html! {
        div class="wrap-narrow stack" {
            header class="stack-tight" {
                p class="eyebrow" { "For your team" }
                h1 { "Sign in to your organization" }
                p class="lede" { "We will email you a link. It works once and expires in fifteen minutes." }
            }
            div class="card" {
                form method="post" action="/auth/magic-link"
                    hx-post="/auth/magic-link" hx-target="#invite-email-error"
                    hx-swap="outerHTML" class="stack" {
                    (csrf_field(&ctx.csrf_token))
                    input type="hidden" name="intent" value="organization";
                    div class="field" {
                        label for="invite-email" { "Work email address" }
                        input id="invite-email" type="email" name="email" autocomplete="email"
                            inputmode="email" required aria-describedby="invite-email-error";
                        (Feedback::Quiet.slot(Field::Email))
                    }
                    div class="row" {
                        button type="submit" class="button button-primary button-large" { "Email me a link" }
                        span class="busy faint" aria-live="polite" { "Sending…" }
                    }
                }
            }
            div class="row" {
                a class="button button-quiet" href="/new" { "Create an organization" }
                a class="button button-quiet" href=(personal) { "I am signing up for myself" }
            }
        }
    }
}

fn create_form(state: &AppState, ctx: &Ctx) -> Markup {
    let personal = surface_url(
        &state.config.apex,
        state.config.cookies_are_secure(),
        "user",
        "/",
    );
    html! {
        div class="wrap-narrow stack" {
            header class="stack-tight" {
                p class="eyebrow" { "For your team" }
                h1 { "Create an organization" }
                p class="lede" {
                    "You will be its first owner. An organization always keeps at least one, so \
                     this account cannot be removed until somebody else is one too."
                }
            }
            div class="card" {
                form method="post" action="/org/new" hx-post="/org/new"
                    hx-target="#invite-email-error" hx-swap="outerHTML" class="stack" {
                    (csrf_field(&ctx.csrf_token))
                    div class="field" {
                        label for="org-name" { "Organization name" }
                        input id="org-name" type="text" name="name" required maxlength="80"
                            autocomplete="organization" aria-describedby="org-name-hint";
                        span class="hint" id="org-name-hint" { "What your colleagues will recognise. You can change it later." }
                    }
                    div class="field" {
                        label for="invite-email" { "Your work email address" }
                        input id="invite-email" type="email" name="email" required
                            autocomplete="email" inputmode="email"
                            aria-describedby="invite-email-error";
                        (Feedback::Quiet.slot(Field::Email))
                    }
                    div class="row" {
                        button type="submit" class="button button-primary button-large" { "Create the organization" }
                        span class="busy faint" aria-live="polite" { "Working…" }
                    }
                }
            }
            p class="faint" {
                "Only ever going to be you? "
                a href=(personal) { "A personal account is simpler" }
                "."
            }
        }
    }
}

fn seat_ledger(seats: Seats, feedback: Feedback<'_>) -> Markup {
    let used = seats.occupied.saturating_add(seats.reserved);
    html! {
        section class="panel stack-tight" id="seat-ledger" aria-labelledby="seat-heading" {
            div class="row-between" {
                h2 id="seat-heading" { "Seats" }
                p class="mono" {
                    (used) " of " (seats.purchased) " in use"
                }
            }
            p class="muted" {
                (seats.occupied) " held by members, " (seats.reserved)
                " reserved by invitations that have not been accepted yet."
                @if seats.available() == 0 {
                    " There are no seats left."
                } @else {
                    " " (seats.available()) " available."
                }
            }
            (feedback.slot(Field::Seats))
        }
    }
}

fn invitations_page(ctx: &Ctx, organization: &Organization, now: i64) -> Markup {
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Seats and invitations" }
                p class="lede" {
                    "An invitation holds a seat from the moment it is sent until it is accepted or \
                     revoked, so an organization cannot promise more seats than it bought."
                }
            }
            (invitations_panel(ctx, organization, now, Feedback::Quiet))
        }
    }
}

/// The seat ledger, the invite form and the pending list, as one swap target.
///
/// They are one fragment rather than three because they are one fact: sending an invitation
/// changes the ledger, revoking one changes it back, and a page where the list has updated and the
/// count has not is a page that has just told somebody a number that is wrong. Swapping the whole
/// panel also means the fragment can never introduce an `id` the surrounding page already has.
#[must_use]
pub fn invitations_panel(
    ctx: &Ctx,
    organization: &Organization,
    now: i64,
    feedback: Feedback<'_>,
) -> Markup {
    let seats = organization.seats(now);
    let actor = actor_role(ctx);
    let may_invite = actor.at_least(Role::Admin);
    let pending = organization.pending(now);
    html! {
        div class="stack" id="invitation-panel" {
            (seat_ledger(seats, feedback))

            @if may_invite {
                section class="card stack" aria-labelledby="invite-heading" {
                    h2 id="invite-heading" { "Invite someone" }
                    form method="post" action="/org/invitations" hx-post="/org/invitations"
                        hx-target="#invitation-panel" hx-swap="outerHTML" class="stack" {
                        (csrf_field(&ctx.csrf_token))
                        div class="row" {
                            div class="field grow" {
                                label for="invite-email" { "Email address" }
                                input id="invite-email" type="email" name="email" required
                                    inputmode="email" autocomplete="off"
                                    aria-describedby="invite-email-error";
                            }
                            div class="field" {
                                label for="invite-role" { "Role" }
                                select id="invite-role" name="role" aria-describedby="invite-role-error" {
                                    @for role in [Role::Viewer, Role::Member, Role::Admin, Role::Owner] {
                                        @if role <= actor {
                                            option value=(role.as_str())
                                                selected[role == Role::Viewer] { (role_label(role)) }
                                        }
                                    }
                                }
                            }
                        }
                        (feedback.slot(Field::Email))
                        (feedback.slot(Field::Role))
                        div class="row" {
                            button type="submit" class="button button-primary" { "Send invitation" }
                            span class="busy faint" aria-live="polite" { "Sending…" }
                        }
                    }
                }
            } @else {
                p class="notice" { "Only an admin or an owner can invite people." }
            }

            section class="card stack" aria-labelledby="pending-heading" {
                div class="row-between" {
                    h2 id="pending-heading" { "Pending invitations" }
                    p class="faint" { (pending.len()) " open" }
                }
                (feedback.slot(Field::Invitation))
                @if pending.is_empty() {
                    p class="muted" { "Nobody is waiting on an invitation." }
                } @else {
                    div class="table-scroll" {
                        table id="invitation-list" {
                            caption class="visually-hidden" { "Open invitations" }
                            thead { tr {
                                th scope="col" { "Email" }
                                th scope="col" { "Role" }
                                th scope="col" { "Expires" }
                                th scope="col" { span class="visually-hidden" { "Actions" } }
                            } }
                            tbody {
                                @for invitation in &pending {
                                    tr {
                                        td class="mono" { (invitation.email) }
                                        td { (role_label(invitation.role)) }
                                        td class="faint" { (relative_days(invitation.expires_at - now)) }
                                        td {
                                            @if actor.at_least(Role::Admin) {
                                                div class="row" {
                                                    (invitation_action(ctx, invitation, "/org/invitations/resend", "Resend", "button-quiet"))
                                                    (invitation_action(ctx, invitation, "/org/invitations/revoke", "Revoke", "button-danger"))
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn invitation_action(
    ctx: &Ctx,
    invitation: &Invitation,
    action: &str,
    label: &str,
    class: &str,
) -> Markup {
    let button_class = format!("button {class}");
    html! {
        form method="post" action=(action) hx-post=(action)
            hx-target="#invitation-panel" hx-swap="outerHTML" {
            (csrf_field(&ctx.csrf_token))
            input type="hidden" name="email" value=(invitation.email);
            button type="submit" class=(button_class) {
                (label)
                span class="visually-hidden" { " the invitation for " (invitation.email) }
            }
        }
    }
}

fn members_page(ctx: &Ctx, organization: &Organization) -> Markup {
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Members" }
                p class="lede" {
                    "Roles are ordered: an owner can do everything an admin can, and so on down. \
                     Nobody can grant a role above their own."
                }
            }
            (member_panel(ctx, organization, Feedback::Quiet))
        }
    }
}

/// The member table, and the htmx swap target for a role change or a removal.
#[must_use]
pub fn member_panel(ctx: &Ctx, organization: &Organization, feedback: Feedback<'_>) -> Markup {
    let actor = actor_role(ctx);
    let owners = organization.owner_count();
    html! {
        section class="card stack" id="member-panel" aria-labelledby="member-heading" {
            div class="row-between" {
                h2 id="member-heading" { "People" }
                p class="faint" { (organization.members.len()) " members, " (owners) " owner" @if owners != 1 { "s" } }
            }
            (feedback.slot(Field::Member))
            (feedback.slot(Field::Role))
            div class="table-scroll" {
                table id="member-list" {
                    caption class="visually-hidden" { "Organization members and their roles" }
                    thead { tr {
                        th scope="col" { "Person" }
                        th scope="col" { "Role" }
                        th scope="col" { span class="visually-hidden" { "Actions" } }
                    } }
                    tbody {
                        @for member in &organization.members {
                            (member_row(ctx, member, actor))
                        }
                    }
                }
            }
        }
    }
}

fn member_row(ctx: &Ctx, member: &Member, actor: Role) -> Markup {
    let may_edit = actor.at_least(Role::Admin) && member.role <= actor;
    let role_select_id = format!("role-{}", member.subject);
    html! {
        tr {
            td {
                div class="stack-tight" {
                    strong { (member.name) }
                    span class="faint mono" { (member.email) }
                }
            }
            td {
                @if may_edit {
                    form method="post" action="/org/members/role" hx-post="/org/members/role"
                        hx-target="#member-panel" hx-swap="outerHTML" class="row" {
                        (csrf_field(&ctx.csrf_token))
                        input type="hidden" name="subject" value=(member.subject);
                        label class="visually-hidden" for=(role_select_id) {
                            "Role for " (member.name)
                        }
                        select id=(role_select_id) name="role" {
                            @for role in [Role::Viewer, Role::Member, Role::Admin, Role::Owner] {
                                @if role <= actor {
                                    option value=(role.as_str()) selected[role == member.role] {
                                        (role_label(role))
                                    }
                                }
                            }
                        }
                        button type="submit" class="button button-quiet" { "Change" }
                    }
                } @else {
                    span { (role_label(member.role)) }
                }
            }
            td {
                @if may_edit {
                    form method="post" action="/org/members/remove" hx-post="/org/members/remove"
                        hx-target="#member-panel" hx-swap="outerHTML" {
                        (csrf_field(&ctx.csrf_token))
                        input type="hidden" name="subject" value=(member.subject);
                        button type="submit" class="button button-danger" {
                            "Remove"
                            span class="visually-hidden" { " " (member.name) }
                        }
                    }
                }
            }
        }
    }
}

fn domains_page(ctx: &Ctx, organization: &Organization) -> Markup {
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Domains" }
                p class="lede" {
                    "Verifying a domain proves you control it. Until then a claim grants nothing at \
                     all — it is a row in a table, not a permission."
                }
            }
            (domain_panel(ctx, organization, Feedback::Quiet))
        }
    }
}

/// The claim form, the claimed-domain table and the join-policy switch, as one swap target — for
/// the same reason the invitations panel is one: claiming, verifying and switching all change what
/// the others display.
#[must_use]
pub fn domain_panel(ctx: &Ctx, organization: &Organization, feedback: Feedback<'_>) -> Markup {
    let may_admin = actor_role(ctx).at_least(Role::Admin);
    html! {
        div class="stack" id="domain-panel" {
            @if may_admin {
                section class="card stack" aria-labelledby="claim-heading" {
                    h2 id="claim-heading" { "Claim a domain" }
                    form method="post" action="/org/domains" hx-post="/org/domains"
                        hx-target="#domain-panel" hx-swap="outerHTML" class="stack" {
                        (csrf_field(&ctx.csrf_token))
                        div class="field" {
                            label for="domain-name" { "Domain" }
                            input id="domain-name" type="text" name="domain" required
                                placeholder="acme.test" autocomplete="off" spellcheck="false"
                                aria-describedby="domain-name-error";
                            (feedback.slot(Field::Domain))
                        }
                        button type="submit" class="button button-primary" { "Claim it" }
                    }
                }
            }

            section class="card stack" aria-labelledby="domain-heading" {
                h2 id="domain-heading" { "Claimed domains" }
                @if organization.domains.is_empty() {
                    p class="muted" { "No domains claimed yet." }
                } @else {
                    div class="table-scroll" {
                        table {
                            caption class="visually-hidden" { "Claimed domains and their verification state" }
                            thead { tr {
                                th scope="col" { "Domain" }
                                th scope="col" { "State" }
                                th scope="col" { "DNS record" }
                                th scope="col" { span class="visually-hidden" { "Actions" } }
                            } }
                            tbody {
                                @for claim in &organization.domains {
                                    (domain_row(ctx, claim, may_admin))
                                }
                            }
                        }
                    }
                }

                @if may_admin {
                    form method="post" action="/org/domains/join" hx-post="/org/domains/join"
                        hx-target="#domain-panel" hx-swap="outerHTML" class="stack-tight" {
                        (csrf_field(&ctx.csrf_token))
                        input type="hidden" name="enabled"
                            value=(if organization.policy.domain_join_enabled { "false" } else { "true" });
                        button type="submit" class="button" {
                            @if organization.policy.domain_join_enabled {
                                "Stop letting people join automatically"
                            } @else {
                                "Let anyone at a verified domain join"
                            }
                        }
                        p class="faint" {
                            "Currently "
                            @if organization.policy.domain_join_enabled { "on" } @else { "off" }
                            ". Convenient, and exactly how a former employee keeps access, which is \
                             why it is off by default."
                        }
                    }
                }
            }
        }
    }
}

fn domain_row(ctx: &Ctx, claim: &DomainClaim, may_admin: bool) -> Markup {
    html! {
        tr {
            td class="mono" { (claim.domain) }
            td {
                @if claim.verified {
                    span class="badge badge-ok" {
                        span class="glyph" aria-hidden="true" { "✓" }
                        "Verified"
                    }
                } @else {
                    span class="badge badge-queued" {
                        span class="glyph" aria-hidden="true" { "◷" }
                        "Awaiting DNS"
                    }
                }
            }
            td class="mono faint" {
                @if claim.verified {
                    "—"
                } @else {
                    (claim.txt_name) " TXT " (claim.txt_value)
                }
            }
            td {
                @if may_admin && !claim.verified {
                    form method="post" action="/org/domains/verify" hx-post="/org/domains/verify"
                        hx-target="#domain-panel" hx-swap="outerHTML" {
                        (csrf_field(&ctx.csrf_token))
                        input type="hidden" name="domain" value=(claim.domain);
                        button type="submit" class="button button-quiet" {
                            "Check now"
                            span class="visually-hidden" { " for " (claim.domain) }
                        }
                    }
                }
            }
        }
    }
}

fn sso_page(organization: &Organization) -> Markup {
    html! {
        div class="wrap-narrow stack" {
            header class="stack-tight" {
                h1 { "Single sign-on" }
                p class="lede" { "Not in this release." }
            }
            div class="card stack" {
                p {
                    "SSO is deliberately not half-built here. An identity integration that is \
                     ninety percent finished is worse than one that does not exist, because people \
                     configure it and believe it."
                }
                p class="muted" {
                    "Until it ships, "
                    a href="/domains" { "domain verification" }
                    " plus invitations covers the case most teams actually need: only people at \
                     your domain, and only people you invited."
                }
                p {
                    span class="badge badge-muted" {
                        span class="glyph" aria-hidden="true" { "⊘" }
                        @if organization.sso_configured { "Configured" } @else { "Not configured" }
                    }
                }
            }
        }
    }
}

fn settings_page(ctx: &Ctx, organization: &Organization, now: i64) -> Markup {
    html! {
        div class="stack" {
            header class="stack-tight" {
                h1 { "Organization settings" }
                p class="lede" { (organization.name) }
            }
            (seat_ledger(organization.seats(now), Feedback::Quiet))
            div class="card stack" {
                h2 { "Details" }
                dl class="stack-tight" {
                    div class="row-between" { dt class="muted" { "Name" } dd { (organization.name) } }
                    div class="row-between" { dt class="muted" { "Slug" } dd class="mono" { (organization.slug) } }
                    div class="row-between" { dt class="muted" { "Your role" } dd { (role_label(actor_role(ctx))) } }
                    div class="row-between" { dt class="muted" { "Verified domains" } dd class="mono" {
                        @if organization.policy.verified_domains.is_empty() {
                            "none"
                        } @else {
                            (organization.policy.verified_domains.join(", "))
                        }
                    } }
                }
            }
        }
    }
}

/// A role, spelled for a person rather than for a database.
#[must_use]
pub const fn role_label(role: Role) -> &'static str {
    match role {
        Role::Viewer => "Viewer — can read runs and logs",
        Role::Member => "Member — can trigger runs and manage runners",
        Role::Admin => "Admin — can manage seats and invitations",
        Role::Owner => "Owner — everything, including billing",
    }
}

/// "in 5 days" / "today" / "expired" — a duration a person can act on rather than a timestamp.
#[must_use]
pub fn relative_days(seconds: i64) -> String {
    if seconds <= 0 {
        return "expired".to_owned();
    }
    let days = seconds / 86_400;
    match days {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        other => format!("in {other} days"),
    }
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

/// Answer a mutation.
///
/// htmx gets the re-rendered panel — with the message already in the right slot — and a browser
/// without it gets a redirect back to the page, which re-renders the same panel from the same
/// data. There is one code path producing the markup either way.
fn answer(
    state: &AppState,
    ctx: &Ctx,
    headers: &HeaderMap,
    refused: bool,
    panel: Markup,
    fallback: &str,
) -> Response {
    if wants_fragment(headers) {
        let status = if refused {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::OK
        };
        super::fragment(state, ctx, status, panel)
    } else {
        super::redirect(state, ctx, fallback)
    }
}

/// `POST /org/invitations`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let now = now_seconds();
    let actor = actor_role(&ctx);
    let email = csrf::form_value(&form, "email").unwrap_or_default();
    let granted = csrf::form_value(&form, "role")
        .and_then(Role::parse)
        .unwrap_or(Role::Viewer);
    let outcome = state.store.invite(actor, email, granted, now);
    Ok(invitations_answer(
        &state,
        &ctx,
        &headers,
        now,
        outcome.map(|_| "Invitation sent. The seat is held until it is accepted or revoked."),
    ))
}

/// `POST /org/invitations/revoke`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let now = now_seconds();
    let email = csrf::form_value(&form, "email").unwrap_or_default();
    let outcome = state.store.revoke_invitation(actor_role(&ctx), email, now);
    Ok(invitations_answer(
        &state,
        &ctx,
        &headers,
        now,
        outcome.map(|()| "Invitation revoked. Its seat is free again."),
    ))
}

/// `POST /org/invitations/resend`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn resend(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let now = now_seconds();
    let email = csrf::form_value(&form, "email").unwrap_or_default();
    let outcome = state.store.resend_invitation(actor_role(&ctx), email, now);
    Ok(invitations_answer(
        &state,
        &ctx,
        &headers,
        now,
        outcome.map(|_| "Invitation sent again; it is good for another week."),
    ))
}

fn invitations_answer(
    state: &AppState,
    ctx: &Ctx,
    headers: &HeaderMap,
    now: i64,
    outcome: Result<&'static str, OnboardingError>,
) -> Response {
    let organization = state.store.organization();
    match outcome {
        Ok(message) => {
            let panel = invitations_panel(
                ctx,
                &organization,
                now,
                Feedback::Done(Field::Invitation, message),
            );
            answer(state, ctx, headers, false, panel, "/invitations")
        }
        Err(error) => {
            let explained = explain(&crate::bridge::refusal(&error));
            let panel = invitations_panel(ctx, &organization, now, Feedback::Refused(&explained));
            answer(state, ctx, headers, true, panel, "/invitations")
        }
    }
}

/// `POST /org/members/role`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn change_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let subject = csrf::form_value(&form, "subject").unwrap_or_default();
    let next = csrf::form_value(&form, "role")
        .and_then(Role::parse)
        .unwrap_or(Role::Viewer);
    let outcome = state.store.change_role(actor_role(&ctx), subject, next);
    Ok(members_answer(
        &state,
        &ctx,
        &headers,
        outcome.map(|_| "Role changed."),
    ))
}

/// `POST /org/members/remove`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn remove_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let subject = csrf::form_value(&form, "subject").unwrap_or_default();
    let outcome = state.store.remove_member(actor_role(&ctx), subject);
    Ok(members_answer(
        &state,
        &ctx,
        &headers,
        outcome.map(|()| "Removed. Their seat is free again."),
    ))
}

fn members_answer(
    state: &AppState,
    ctx: &Ctx,
    headers: &HeaderMap,
    outcome: Result<&'static str, OnboardingError>,
) -> Response {
    let organization = state.store.organization();
    match outcome {
        Ok(message) => {
            let panel = member_panel(ctx, &organization, Feedback::Done(Field::Member, message));
            answer(state, ctx, headers, false, panel, "/members")
        }
        Err(error) => {
            let explained = explain(&crate::bridge::refusal(&error));
            let panel = member_panel(ctx, &organization, Feedback::Refused(&explained));
            answer(state, ctx, headers, true, panel, "/members")
        }
    }
}

/// `POST /org/domains`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn claim_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let domain = csrf::form_value(&form, "domain").unwrap_or_default();
    let outcome = state.store.claim_domain(actor_role(&ctx), domain);
    Ok(domains_answer(
        &state,
        &ctx,
        &headers,
        outcome.map(|_| "Claimed. Publish the TXT record below, then check it."),
    ))
}

/// `POST /org/domains/verify`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn verify_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let domain = csrf::form_value(&form, "domain").unwrap_or_default();
    let outcome = state.store.verify_domain(actor_role(&ctx), domain);
    Ok(domains_answer(
        &state,
        &ctx,
        &headers,
        outcome.map(|()| "Verified."),
    ))
}

/// `POST /org/domains/join`.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn set_domain_join(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let enabled = csrf::form_value(&form, "enabled") == Some("true");
    let outcome = state.store.set_domain_join(actor_role(&ctx), enabled);
    Ok(domains_answer(
        &state,
        &ctx,
        &headers,
        outcome.map(|()| "Joining policy changed."),
    ))
}

fn domains_answer(
    state: &AppState,
    ctx: &Ctx,
    headers: &HeaderMap,
    outcome: Result<&'static str, OnboardingError>,
) -> Response {
    let organization = state.store.organization();
    match outcome {
        Ok(message) => {
            let panel = domain_panel(ctx, &organization, Feedback::Done(Field::Domain, message));
            answer(state, ctx, headers, false, panel, "/domains")
        }
        Err(error) => {
            let explained = explain(&crate::bridge::refusal(&error));
            let panel = domain_panel(ctx, &organization, Feedback::Refused(&explained));
            answer(state, ctx, headers, true, panel, "/domains")
        }
    }
}

/// `POST /org/new` — create an organization.
///
/// # Errors
/// [`WebError`] from [`super::accept_post`].
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, WebError> {
    let (ctx, form) = super::accept_post(&state, &headers, &body)?;
    let email = csrf::form_value(&form, "email").unwrap_or_default();
    // Creating the organization is the same magic-link flow with a different intent: nothing is
    // written until the link is redeemed, so a stranger cannot create an organization by posting
    // a form — only somebody who can read that mailbox can.
    match gha_indie_worker_lib_core::runtime::tenancy::parse_email(email) {
        Err(error) => {
            let explained = explain(&crate::bridge::refusal(&error));
            Ok(super::fragment(
                &state,
                &ctx,
                StatusCode::UNPROCESSABLE_ENTITY,
                error_block(&explained),
            ))
        }
        Ok((normalized, _)) => {
            let link = state.magic.issue(
                &normalized,
                crate::auth::Intent::CreateOrganization,
                now_seconds(),
            );
            let development = !state.config.shared_auth_configured();
            let href = format!("/auth/callback?token={}", link.token);
            let markup = html! {
                (Feedback::Done(
                    Field::Email,
                    "Check your email — the link finishes creating your organization.",
                )
                .slot(Field::Email))
                @if development {
                    p class="notice notice-warn" {
                        "Development mode — no mail was sent. "
                        a href=(href) { "Use the link" }
                    }
                }
            };
            Ok(super::fragment(&state, &ctx, StatusCode::OK, markup))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_label_says_what_the_role_can_do_not_just_its_name() {
        for role in [Role::Viewer, Role::Member, Role::Admin, Role::Owner] {
            let label = role_label(role);
            assert!(label.contains(" — "), "{role} has no explanation: {label}");
            assert!(label.len() > 12, "{role} has a bare label");
        }
    }

    #[test]
    fn expiry_is_shown_as_something_a_person_can_act_on() {
        assert_eq!(relative_days(-1), "expired");
        assert_eq!(relative_days(0), "expired");
        assert_eq!(relative_days(3_600), "today");
        assert_eq!(relative_days(86_400 + 10), "tomorrow");
        assert_eq!(relative_days(5 * 86_400), "in 5 days");
    }

    #[test]
    fn the_seat_ledger_names_every_number_it_uses() {
        let markup = seat_ledger(
            Seats {
                purchased: 5,
                occupied: 3,
                reserved: 1,
            },
            Feedback::Quiet,
        )
        .into_string();
        assert!(markup.contains("4 of 5 in use"), "{markup}");
        assert!(markup.contains("3 held by members"), "{markup}");
        assert!(markup.contains("1 reserved by invitations"), "{markup}");
        assert!(markup.contains("1 available"), "{markup}");

        let full = seat_ledger(
            Seats {
                purchased: 2,
                occupied: 2,
                reserved: 0,
            },
            Feedback::Quiet,
        )
        .into_string();
        assert!(full.contains("There are no seats left"), "{full}");
    }
}
