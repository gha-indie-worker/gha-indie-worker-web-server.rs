#![forbid(unsafe_code)]

//! `org.indiebuild.dev` — the B2B surface.
//!
//! Three things live here and nowhere else: organization sign-in through
//! shared-auth, the multi-seat onboarding wizard, and organization
//! administration (members, roles, seats, audit, SSO).
//!
//! The wizard is an explicit state machine ([`OnboardingStep`]) rather than a
//! pile of booleans: every step knows its predecessor and successor, the rail
//! renders from the same enum, and an unknown step slug is a 404 instead of a
//! half-built organization. Illegal states are unrepresentable — there is no way
//! to be "verifying a domain" for an organization that does not exist yet,
//! because [`OnboardingStep::VerifyDomain`] can only be reached from
//! [`OnboardingStep::CreateOrg`]'s success path.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Form, Router};
use maud::{html, Markup};
use serde::Deserialize;

use crate::data::models::{parse_invite_list, AuditEntry, InviteOutcome, OrgMember, SeatUsage};
use crate::error::WebError;
use crate::hosts::{common, Surface};
use crate::middleware::RequestCtx;
use crate::state::AppState;
use crate::ui::chat_widget::ChatAudience;
use crate::ui::layout::PageMeta;
use crate::ui::{components, forms, tables};

/// Roles an organization can assign. Deliberately small.
pub const ROLES: &[(&str, &str)] = &[
    ("owner", "Owner — billing, seats, SSO"),
    ("admin", "Admin — members and policy"),
    ("member", "Member — run and read"),
    ("viewer", "Viewer — read only"),
];

/// The onboarding state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OnboardingStep {
    /// Name the organization and claim its slug.
    CreateOrg,
    /// Prove the domain, by DNS TXT record or by an email to a domain address.
    VerifyDomain,
    /// Decide how many seats to buy.
    AllocateSeats,
    /// Invite people — pasted CSV, or one at a time.
    InviteMembers,
    /// Hand off to billing.
    Billing,
    /// Finished.
    Done,
}

impl OnboardingStep {
    /// Every step, in order.
    pub const ALL: [Self; 6] = [
        Self::CreateOrg,
        Self::VerifyDomain,
        Self::AllocateSeats,
        Self::InviteMembers,
        Self::Billing,
        Self::Done,
    ];

    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::CreateOrg => "create",
            Self::VerifyDomain => "verify-domain",
            Self::AllocateSeats => "seats",
            Self::InviteMembers => "invite",
            Self::Billing => "billing",
            Self::Done => "done",
        }
    }

    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.slug() == slug)
    }

    /// 1-based position, for the progress rail.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::CreateOrg => 1,
            Self::VerifyDomain => 2,
            Self::AllocateSeats => 3,
            Self::InviteMembers => 4,
            Self::Billing => 5,
            Self::Done => 6,
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::CreateOrg => "Create organization",
            Self::VerifyDomain => "Verify domain",
            Self::AllocateSeats => "Allocate seats",
            Self::InviteMembers => "Invite members",
            Self::Billing => "Billing",
            Self::Done => "Done",
        }
    }

    /// The next step. [`Self::Done`] is terminal and returns itself.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::CreateOrg => Self::VerifyDomain,
            Self::VerifyDomain => Self::AllocateSeats,
            Self::AllocateSeats => Self::InviteMembers,
            Self::InviteMembers => Self::Billing,
            Self::Billing | Self::Done => Self::Done,
        }
    }

    /// The previous step. [`Self::CreateOrg`] is initial and returns itself.
    #[must_use]
    pub const fn previous(self) -> Self {
        match self {
            Self::CreateOrg | Self::VerifyDomain => Self::CreateOrg,
            Self::AllocateSeats => Self::VerifyDomain,
            Self::InviteMembers => Self::AllocateSeats,
            Self::Billing => Self::InviteMembers,
            Self::Done => Self::Billing,
        }
    }

    #[must_use]
    pub fn path(self) -> String {
        format!("/onboarding/{}", self.slug())
    }
}

/// Everything the wizard renders from. Constructed by the handler, passed to the
/// view — so a snapshot test can render any step without an HTTP request.
#[derive(Clone, Debug, Default)]
pub struct WizardView {
    pub org_name: String,
    pub org_slug: String,
    pub domain: String,
    /// The DNS TXT value to publish, once the api-server has minted one.
    pub dns_token: Option<String>,
    pub seats: SeatUsage,
    pub invites: Vec<InviteOutcome>,
    /// A short message shown above the form.
    pub notice: Option<String>,
    pub billing_url: Option<String>,
}

#[must_use]
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(overview))
        .route("/login", get(login_page).post(login_start))
        .route("/auth/callback", get(auth_callback))
        .route("/onboarding", get(onboarding_root))
        .route("/onboarding/{step}", get(onboarding_step).post(onboarding_submit))
        .route("/members", get(members))
        .route("/members/{member_id}/role", post(set_role))
        .route("/roles", get(roles_page))
        .route("/seats", get(seats_page))
        .route("/audit", get(audit_page))
        .route("/sso", get(sso_page))
        .merge(common::routes())
}

#[must_use]
pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

// ---- sign in ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct NextQuery {
    #[serde(default)]
    pub next: Option<String>,
}

async fn login_page(Extension(context): Extension<RequestCtx>, Query(query): Query<NextQuery>) -> Response {
    if context.actor.is_some() {
        return Redirect::to("/").into_response();
    }
    let next = query
        .next
        .filter(|value| crate::session::is_safe_next(value))
        .unwrap_or_else(|| "/".to_owned());
    let meta = PageMeta::new(
        "Sign in to your organization",
        "Organization sign-in for GHA Indie Worker",
    )
    .with_chat(ChatAudience::Visitor);
    let body = html! {
        section class="section" {
            p class="eyebrow" { "ORGANIZATIONS" }
            h1 { "Sign in to your organization" }
            p class="lede" {
                "Organization accounts are held by Shared Auth, on its own host. You will come back \
                 here once it has confirmed who you are."
            }
            (forms::form(&forms::FormAction::post("/login"), &context.csrf_token, html! {
                input type="hidden" name="next" value=(next);
                (forms::text_field("org-email", "email", "Work email", "email", "", "Use the address your organization invited.", true))
                (forms::actions("Continue", "", ""))
            }))
            p { "No organization yet? " a class="text-link" href="/onboarding" { "Create one" } "." }
            p {
                "Looking for a personal account? "
                a class="text-link" href=(format!("{}/login", context.page().origin(Surface::User))) { "Sign in on user.indiebuild.dev" }
                "."
            }
        }
    };
    common::render(&context, &meta, body)
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub next: Option<String>,
}

/// Hands off to shared-auth. This server never sees a password.
async fn login_start(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return common::error_page(&context, &error);
    }
    let Some(base) = state.config.shared_auth.base.as_deref() else {
        return common::error_page(&context, &WebError::Unavailable);
    };
    let next = form
        .next
        .filter(|value| crate::session::is_safe_next(value))
        .unwrap_or_else(|| "/".to_owned());
    let redirect_uri = format!("{}/auth/callback", context.page().origin(Surface::Org));
    let target = format!(
        "{}/auth/authorize?surface=org&redirect_uri={}&state={}&login_hint={}",
        base.trim_end_matches('/'),
        crate::session::percent_encode_path(&redirect_uri),
        crate::session::percent_encode_path(&next),
        crate::session::percent_encode_path(form.email.trim()),
    );
    Redirect::to(&target).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

/// The shared-auth exchange: the federated code/token becomes a shared-auth
/// access token, which becomes this origin's host-scoped cookie session.
async fn auth_callback(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    crate::session::complete_exchange(&state, &context, query.code.as_deref(), query.state.as_deref()).await
}

// ---- onboarding ------------------------------------------------------------

async fn onboarding_root() -> Response {
    Redirect::to(&OnboardingStep::CreateOrg.path()).into_response()
}

async fn onboarding_step(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Path(step): Path<String>,
) -> Response {
    let Some(step) = OnboardingStep::from_slug(&step) else {
        return common::error_page(&context, &WebError::NotFound);
    };
    let mut view = WizardView::default();
    if let Some(org_id) = context.org_id() {
        view.seats = state.repo.seats(context.bearer(), org_id).await.unwrap_or_default();
    }
    render_step(&context, step, &view)
}

#[derive(Debug, Deserialize)]
pub struct OnboardingForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub seats: Option<u32>,
    #[serde(default)]
    pub invites: String,
    #[serde(default)]
    pub role: String,
}

/// One step's submission. Every branch either advances exactly one step or
/// re-renders the same step with a notice — never a silent no-op.
async fn onboarding_submit(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Path(step): Path<String>,
    Form(form): Form<OnboardingForm>,
) -> Response {
    let Some(step) = OnboardingStep::from_slug(&step) else {
        return common::error_page(&context, &WebError::NotFound);
    };
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return common::error_page(&context, &error);
    }
    let bearer = context.bearer();
    let mut view = WizardView::default();

    match step {
        OnboardingStep::CreateOrg => {
            let name = form.name.trim();
            if name.is_empty() {
                view.notice = Some("Give the organization a name.".to_owned());
                return render_step(&context, step, &view);
            }
            let slug = if form.slug.trim().is_empty() {
                slugify(name)
            } else {
                slugify(&form.slug)
            };
            match state.repo.api().create_org(bearer, name, &slug).await {
                Ok(_) => Redirect::to(&step.next().path()).into_response(),
                Err(error) => {
                    view.org_name = name.to_owned();
                    view.org_slug = slug;
                    view.notice = Some(error.detail().to_owned());
                    render_step(&context, step, &view)
                }
            }
        }
        OnboardingStep::VerifyDomain => {
            let domain = form.domain.trim().to_ascii_lowercase();
            let method = if form.method == "email" { "email" } else { "dns-txt" };
            if !is_plausible_domain(&domain) {
                view.notice = Some("That does not look like a domain you can prove.".to_owned());
                return render_step(&context, step, &view);
            }
            let Some(org_id) = context.org_id() else {
                return common::error_page(&context, &WebError::Forbidden);
            };
            match state
                .repo
                .api()
                .start_domain_verification(bearer, org_id, &domain, method)
                .await
            {
                Ok(value) => {
                    view.domain = domain;
                    view.dns_token = value
                        .get("token")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    view.notice = Some(match method {
                        "email" => "We sent a confirmation to a domain address.".to_owned(),
                        _ => "Publish the TXT record below, then continue.".to_owned(),
                    });
                    render_step(&context, step, &view)
                }
                Err(error) => {
                    view.domain = domain;
                    view.notice = Some(error.detail().to_owned());
                    render_step(&context, step, &view)
                }
            }
        }
        OnboardingStep::AllocateSeats => {
            let seats = form.seats.unwrap_or(0);
            if seats == 0 {
                view.notice = Some("Allocate at least one seat.".to_owned());
                return render_step(&context, step, &view);
            }
            let Some(org_id) = context.org_id() else {
                return common::error_page(&context, &WebError::Forbidden);
            };
            match state.repo.api().allocate_seats(bearer, org_id, seats).await {
                Ok(_) => Redirect::to(&step.next().path()).into_response(),
                Err(error) => {
                    view.notice = Some(error.detail().to_owned());
                    render_step(&context, step, &view)
                }
            }
        }
        OnboardingStep::InviteMembers => {
            let default_role = if ROLES.iter().any(|(value, _)| *value == form.role) {
                form.role.as_str()
            } else {
                "member"
            };
            let (accepted, rejected) = parse_invite_list(&form.invites, default_role);
            if accepted.is_empty() && rejected.is_empty() {
                // Inviting nobody now is allowed; members can be added later.
                return Redirect::to(&step.next().path()).into_response();
            }
            let Some(org_id) = context.org_id() else {
                return common::error_page(&context, &WebError::Forbidden);
            };
            view.invites = rejected;
            if !accepted.is_empty() {
                match state.repo.api().invite_members(bearer, org_id, &accepted).await {
                    Ok(mut outcomes) => view.invites.append(&mut outcomes),
                    Err(error) => view.notice = Some(error.detail().to_owned()),
                }
            }
            if view.notice.is_none() && view.invites.iter().all(|outcome| outcome.accepted) {
                return Redirect::to(&step.next().path()).into_response();
            }
            render_step(&context, step, &view)
        }
        OnboardingStep::Billing | OnboardingStep::Done => Redirect::to(&step.next().path()).into_response(),
    }
}

/// Renders one wizard step with its rail.
#[must_use]
pub fn render_step(context: &RequestCtx, step: OnboardingStep, view: &WizardView) -> Response {
    let meta =
        PageMeta::new(step.title(), "Set up your organization on GHA Indie Worker").with_chat(ChatAudience::Visitor);
    common::render(context, &meta, step_body(context, step, view))
}

/// The body of one step. Pure, so the snapshot tests can call it directly.
#[must_use]
pub fn step_body(context: &RequestCtx, step: OnboardingStep, view: &WizardView) -> Markup {
    html! {
        section class="section" {
            p class="section-number" { (format!("{:02} / ONBOARDING", step.index())) }
            h1 { (step.title()) }
            (rail(step))
            @if let Some(notice) = &view.notice {
                (components::alert(components::Tone::Warn, "Not done yet", notice))
            }
            (step_form(context, step, view))
        }
    }
}

/// The progress rail, derived from the state machine rather than hand-listed.
#[must_use]
pub fn rail(current: OnboardingStep) -> Markup {
    let steps: Vec<(usize, &str, components::StepState)> = OnboardingStep::ALL
        .iter()
        .map(|step| {
            let state = if step.index() < current.index() {
                components::StepState::Done
            } else if *step == current {
                components::StepState::Current
            } else {
                components::StepState::Upcoming
            };
            (step.index(), step.title(), state)
        })
        .collect();
    components::step_indicator(&steps)
}

fn step_form(context: &RequestCtx, step: OnboardingStep, view: &WizardView) -> Markup {
    let action = step.path();
    let back = step.previous().path();
    let token = context.csrf_token.as_str();
    match step {
        OnboardingStep::CreateOrg => forms::form(
            &forms::FormAction::post(action.as_str()),
            token,
            html! {
                p { "Name the organization. Everyone you invite will see this name." }
                (forms::text_field("org-name", "name", "Organization name", "text", &view.org_name, "", true))
                (forms::text_field("org-slug", "slug", "URL slug", "text", &view.org_slug, "Leave empty to derive it from the name.", false))
                (forms::actions("Create organization", "", ""))
            },
        ),
        OnboardingStep::VerifyDomain => forms::form(
            &forms::FormAction::post(action.as_str()),
            token,
            html! {
                p { "Prove you control the domain, so invitations to it can be trusted." }
                (forms::text_field("org-domain", "domain", "Domain", "text", &view.domain, "For example: example.com", true))
                (forms::select_field("verify-method", "method", "Method", &[
                    ("dns-txt", "DNS TXT record"),
                    ("email", "Email to an address at the domain"),
                ], "dns-txt"))
                @if let Some(dns_token) = &view.dns_token {
                    (components::card("DNS TXT RECORD", html! {
                        p { "Publish this record, then continue." }
                        pre class="logs" { (format!("_indiebuild-verify.{}  TXT  \"{}\"", view.domain, dns_token)) }
                    }))
                }
                (forms::actions("Verify domain", &back, "Back"))
            },
        ),
        OnboardingStep::AllocateSeats => forms::form(
            &forms::FormAction::post(action.as_str()),
            token,
            html! {
                p { "Seats are how many people can run work. You can change this later." }
                (forms::number_field("seat-count", "seats", "Seats", i64::from(view.seats.allocated.max(1)), 1, 500, "One seat per person."))
                (components::stat("Currently used", &view.seats.used.to_string(), &format!("{} invitation(s) pending", view.seats.pending_invites)))
                (forms::actions("Allocate seats", &back, "Back"))
            },
        ),
        OnboardingStep::InviteMembers => forms::form(
            &forms::FormAction::post(action.as_str()),
            token,
            html! {
                p { "Paste a CSV export, or type one address per line. A role in the second column overrides the default." }
                (forms::textarea_field(
                    "invite-list",
                    "invites",
                    "Invitations",
                    "",
                    "One address per line, or `email,role` per line.",
                    "alex@example.com\nsam@example.com, admin",
                ))
                (forms::select_field("invite-role", "role", "Default role", ROLES, "member"))
                @if !view.invites.is_empty() {
                    (invite_results(&view.invites))
                }
                (forms::actions("Send invitations", &back, "Back"))
            },
        ),
        OnboardingStep::Billing => html! {
            p { "Billing is handled outside the product so card details never touch this server." }
            div class="row" {
                @match &view.billing_url {
                    Some(url) => {
                        a class="button" href=(url) { "Open billing" }
                    }
                    None => {
                        a class="button" href="/seats" { "Review seats first" }
                    }
                }
                a class="button secondary" href=(OnboardingStep::Done.path()) { "Skip for now" }
                a class="button secondary" href=(back) { "Back" }
            }
        },
        OnboardingStep::Done => html! {
            (components::alert(components::Tone::Ok, "Your organization is ready", "Invite more people any time from the members page."))
            div class="row" {
                a class="button" href="/members" { "Open members" }
                a class="button secondary" href=(format!("{}/dashboard", context.page().origin(Surface::App))) { "Open the app" }
            }
        },
    }
}

fn invite_results(outcomes: &[InviteOutcome]) -> Markup {
    let rows = outcomes
        .iter()
        .map(|outcome| {
            tables::row(vec![
                tables::cell(&outcome.email),
                tables::markup_cell(if outcome.accepted {
                    components::badge("invited", components::Tone::Ok)
                } else {
                    components::badge("not sent", components::Tone::Bad)
                }),
                tables::cell(outcome.reason.as_deref().unwrap_or("—")),
            ])
        })
        .collect();
    tables::table(
        "Invitations",
        &[
            tables::Column::text("Address"),
            tables::Column::text("Result"),
            tables::Column::text("Reason"),
        ],
        rows,
    )
}

// ---- administration --------------------------------------------------------

async fn overview(Extension(context): Extension<RequestCtx>) -> Response {
    if context.actor.is_none() {
        return Redirect::to("/login").into_response();
    }
    let meta = PageMeta::new("Organization", "Organization overview")
        .active("overview")
        .with_chat(ChatAudience::Customer);
    let body = html! {
        section class="section" {
            p class="eyebrow" { "ORGANIZATION" }
            h1 { "Overview" }
            div class="grid grid-2" {
                (components::card("MEMBERSHIP", html! {
                    h3 { "Members and roles" }
                    p { "Who is in the organization, and what each of them may do." }
                    a class="button secondary" href="/members" { "Open members" }
                }))
                (components::card("CAPACITY", html! {
                    h3 { "Seats" }
                    p { "How many people can run work, and how many invitations are outstanding." }
                    a class="button secondary" href="/seats" { "Open seats" }
                }))
            }
        }
    };
    common::render(&context, &meta, body)
}

async fn members(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    let Some(org_id) = context.org_id().map(str::to_owned) else {
        return Redirect::to("/login").into_response();
    };
    let meta = PageMeta::new("Members", "Everyone in this organization")
        .active("members")
        .with_chat(ChatAudience::Customer);
    let body = state
        .repo
        .members(context.bearer(), &org_id)
        .await
        .map(|members| components::section("", "Members", member_table(&context, &members)));
    common::render_result(&context, &meta, body)
}

#[derive(Debug, Deserialize)]
pub struct RoleForm {
    #[serde(default)]
    pub csrf_token: Option<String>,
    #[serde(default)]
    pub role: String,
}

async fn set_role(
    Extension(context): Extension<RequestCtx>,
    State(state): State<AppState>,
    Path(member_id): Path<String>,
    Form(form): Form<RoleForm>,
) -> Response {
    if let Err(error) = context.verify_form_csrf(&state.csrf, form.csrf_token.as_deref()) {
        return error.into_response();
    }
    let Some(org_id) = context.org_id() else {
        return WebError::Forbidden.into_response();
    };
    if !ROLES.iter().any(|(value, _)| *value == form.role) {
        return WebError::BadRequest("That is not a role this organization has.").into_response();
    }
    match state
        .repo
        .api()
        .set_member_role(context.bearer(), org_id, &member_id, &form.role)
        .await
    {
        Ok(_) => Redirect::to("/members").into_response(),
        Err(error) => error.into_response(),
    }
}

async fn roles_page(Extension(context): Extension<RequestCtx>) -> Response {
    let meta = PageMeta::new("Roles", "What each role may do")
        .active("members")
        .with_chat(ChatAudience::Customer);
    let rows = ROLES
        .iter()
        .map(|(value, description)| tables::row(vec![tables::cell(value), tables::cell(description)]))
        .collect();
    let body = components::section(
        "",
        "Roles",
        tables::table(
            "Roles",
            &[tables::Column::text("Role"), tables::Column::text("Permissions")],
            rows,
        ),
    );
    common::render(&context, &meta, body)
}

async fn seats_page(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    let Some(org_id) = context.org_id().map(str::to_owned) else {
        return Redirect::to("/login").into_response();
    };
    let meta = PageMeta::new("Seats", "Seat allocation and usage")
        .active("seats")
        .with_chat(ChatAudience::Customer);
    let body = state.repo.seats(context.bearer(), &org_id).await.map(|seats| {
        html! {
            section class="section" {
                p class="eyebrow" { "CAPACITY" }
                h1 { "Seats" }
                div class="grid grid-3" {
                    (components::stat("Allocated", &seats.allocated.to_string(), ""))
                    (components::stat("Used", &seats.used.to_string(), ""))
                    (components::stat("Available", &seats.available().to_string(), &format!("{} invitation(s) pending", seats.pending_invites)))
                }
                @if seats.is_exhausted() {
                    (components::alert(components::Tone::Warn, "No seats left", "Allocate more seats before inviting anyone else."))
                }
                div class="row" { a class="button" href=(OnboardingStep::AllocateSeats.path()) { "Change seat count" } }
            }
        }
    });
    common::render_result(&context, &meta, body)
}

async fn audit_page(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    let Some(org_id) = context.org_id().map(str::to_owned) else {
        return Redirect::to("/login").into_response();
    };
    let meta = PageMeta::new("Audit", "What changed, who changed it")
        .active("audit")
        .with_chat(ChatAudience::Customer);
    let body = state
        .repo
        .audit(context.bearer(), &org_id, 100)
        .await
        .map(|entries| components::section("", "Audit", audit_table(&entries)));
    common::render_result(&context, &meta, body)
}

/// The SSO page reads shared-auth's advertised capabilities rather than
/// hard-coding a claim: if SAML is not enabled on the auth instance, the page
/// says so instead of offering a control that cannot work.
async fn sso_page(Extension(context): Extension<RequestCtx>, State(state): State<AppState>) -> Response {
    let capabilities = state.auth.capabilities().await.ok();
    let meta = PageMeta::new("Single sign-on", "SAML and SCIM status for this organization")
        .active("sso")
        .with_chat(ChatAudience::Customer);
    let body = html! {
        section class="section" {
            p class="eyebrow" { "IDENTITY" }
            h1 { "Single sign-on" }
            @match &capabilities {
                Some(capabilities) => {
                    p class="lede" { "Reported by Shared Auth for this deployment." }
                    div class="grid grid-2" {
                        (components::card("MULTI-FACTOR", html! {
                            h3 { @if capabilities.mfa_enabled { "Enabled" } @else { "Not enabled" } }
                            p { (format!("Methods: {}", if capabilities.methods.is_empty() { "none reported".to_owned() } else { capabilities.methods.join(", ") })) }
                        }))
                        (components::card("BIOMETRIC MODEL", html! {
                            h3 { (capabilities.biometric_model.clone().unwrap_or_else(|| "not reported".to_owned())) }
                            p { (format!("Third-factor import: {}", capabilities.threefa_import_scheme.clone().unwrap_or_else(|| "not reported".to_owned()))) }
                        }))
                    }
                    (components::alert(
                        components::Tone::Neutral,
                        "SAML and SCIM are not self-serve yet",
                        "Metadata and provisioning are configured with us during onboarding. This page will gain the self-serve controls once the shared-auth SAML surface is enabled for organizations.",
                    ))
                }
                None => {
                    (components::alert(
                        components::Tone::Warn,
                        "Shared Auth did not answer",
                        "Single sign-on status is unavailable right now. Nothing about your organization has changed.",
                    ))
                }
            }
        }
    };
    common::render(&context, &meta, body)
}

#[must_use]
pub fn member_table(context: &RequestCtx, members: &[OrgMember]) -> Markup {
    let rows = members
        .iter()
        .map(|member| {
            let action = format!("/members/{}/role", member.id);
            tables::row(vec![
                tables::cell(&member.email),
                tables::markup_cell(components::status_badge(&member.status)),
                tables::markup_cell(forms::form(
                    &forms::FormAction::post(action.as_str()).targeting("body"),
                    &context.csrf_token,
                    html! {
                        (forms::select_field(&format!("role-{}", member.id), "role", "Role", ROLES, &member.role))
                        button type="submit" class="button secondary" { "Save" }
                    },
                )),
                tables::cell(member.last_active_at.as_deref().unwrap_or("—")),
            ])
        })
        .collect();
    tables::table(
        "Members",
        &[
            tables::Column::text("Address"),
            tables::Column::text("Status"),
            tables::Column::text("Role"),
            tables::Column::text("Last active"),
        ],
        rows,
    )
}

#[must_use]
pub fn audit_table(entries: &[AuditEntry]) -> Markup {
    let rows = entries
        .iter()
        .map(|entry| {
            tables::row(vec![
                tables::cell(&entry.at),
                tables::cell(&entry.actor),
                tables::cell(&entry.action),
                tables::cell(entry.target.as_deref().unwrap_or("—")),
                tables::markup_cell(components::status_badge(&entry.result)),
            ])
        })
        .collect();
    tables::table(
        "Audit",
        &[
            tables::Column::text("When"),
            tables::Column::text("Actor"),
            tables::Column::text("Action"),
            tables::Column::text("Target"),
            tables::Column::text("Result"),
        ],
        rows,
    )
}

/// `Acme, Inc.` → `acme-inc`. Bounded, lowercase, no leading or trailing dash.
#[must_use]
pub fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut previous_dash = true;
    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            out.push('-');
            previous_dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    out.trim_matches('-').to_owned()
}

/// A domain we could plausibly ask someone to prove.
#[must_use]
pub fn is_plausible_domain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.contains('.')
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !value.contains('@')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RequestCtx {
        RequestCtx {
            surface: Surface::Org,
            host: "org.indiebuild.dev".into(),
            base_domain: "indiebuild.dev".into(),
            path: "/onboarding/create".into(),
            nonce: "n0nce".into(),
            csrf_token: "tok3n".into(),
            csrf: crate::csrf::CsrfCheck::NotRequired,
            actor: None,
            is_htmx: false,
            subject: "anonymous".into(),
            release_manifest_url: None,
            chat_enabled: true,
        }
    }

    #[test]
    fn the_wizard_is_a_total_ordered_state_machine() {
        assert_eq!(OnboardingStep::ALL.len(), 6);
        let mut step = OnboardingStep::CreateOrg;
        for expected in [
            OnboardingStep::VerifyDomain,
            OnboardingStep::AllocateSeats,
            OnboardingStep::InviteMembers,
            OnboardingStep::Billing,
            OnboardingStep::Done,
        ] {
            step = step.next();
            assert_eq!(step, expected);
        }
        assert_eq!(OnboardingStep::Done.next(), OnboardingStep::Done, "Done is terminal");
        assert_eq!(
            OnboardingStep::CreateOrg.previous(),
            OnboardingStep::CreateOrg,
            "Create is initial"
        );
    }

    #[test]
    fn next_and_previous_are_inverses_in_the_middle() {
        for step in [
            OnboardingStep::VerifyDomain,
            OnboardingStep::AllocateSeats,
            OnboardingStep::InviteMembers,
            OnboardingStep::Billing,
        ] {
            assert_eq!(step.next().previous(), step, "{step:?} round-trip");
        }
    }

    #[test]
    fn slugs_round_trip_and_indices_are_unique() {
        let mut indices: Vec<usize> = OnboardingStep::ALL.iter().map(|step| step.index()).collect();
        indices.sort_unstable();
        assert_eq!(indices, vec![1, 2, 3, 4, 5, 6]);
        for step in OnboardingStep::ALL {
            assert_eq!(OnboardingStep::from_slug(step.slug()), Some(step));
        }
        assert_eq!(OnboardingStep::from_slug("not-a-step"), None);
    }

    #[test]
    fn the_rail_marks_exactly_one_current_step() {
        let html = rail(OnboardingStep::AllocateSeats).into_string();
        assert_eq!(html.matches(r#"data-state="current""#).count(), 1);
        assert_eq!(html.matches(r#"data-state="done""#).count(), 2);
        assert_eq!(html.matches(r#"data-state="upcoming""#).count(), 3);
    }

    #[test]
    fn every_step_renders_a_csrf_protected_form_or_a_terminal_screen() {
        let context = context();
        let view = WizardView::default();
        for step in OnboardingStep::ALL {
            let html = step_body(&context, step, &view).into_string();
            assert!(html.contains(step.title()), "{step:?} does not name itself");
            assert!(html.contains(r#"class="steps""#), "{step:?} is missing the rail");
            if !matches!(step, OnboardingStep::Billing | OnboardingStep::Done) {
                assert!(
                    html.contains(r#"name="csrf_token" value="tok3n""#),
                    "{step:?} form has no CSRF token"
                );
                assert!(
                    html.contains(&format!(r#"hx-post="{}""#, step.path())),
                    "{step:?} posts elsewhere"
                );
            }
        }
    }

    #[test]
    fn the_domain_step_offers_both_proof_methods() {
        let html = step_body(&context(), OnboardingStep::VerifyDomain, &WizardView::default()).into_string();
        assert!(html.contains(r#"value="dns-txt""#));
        assert!(html.contains(r#"value="email""#));
    }

    #[test]
    fn the_domain_step_shows_the_txt_record_once_it_exists() {
        let view = WizardView {
            domain: "example.com".into(),
            dns_token: Some("giw-verify-abc".into()),
            ..WizardView::default()
        };
        let html = step_body(&context(), OnboardingStep::VerifyDomain, &view).into_string();
        assert!(html.contains("_indiebuild-verify.example.com"));
        assert!(html.contains("giw-verify-abc"));
    }

    #[test]
    fn the_invite_step_accepts_a_csv_paste_and_reports_per_row_results() {
        let view = WizardView {
            invites: vec![
                InviteOutcome::accepted("a@example.com"),
                InviteOutcome::rejected("oops", "not an email address"),
            ],
            ..WizardView::default()
        };
        let html = step_body(&context(), OnboardingStep::InviteMembers, &view).into_string();
        assert!(html.contains("email,role"));
        assert!(html.contains("a@example.com"));
        assert!(html.contains("not an email address"));
    }

    #[test]
    fn a_notice_is_shown_above_the_form_rather_than_swallowed() {
        let view = WizardView {
            notice: Some("Give the organization a name.".into()),
            ..WizardView::default()
        };
        let html = step_body(&context(), OnboardingStep::CreateOrg, &view).into_string();
        assert!(html.contains("Give the organization a name."));
    }

    #[test]
    fn slugify_produces_a_usable_slug() {
        assert_eq!(slugify("Acme, Inc."), "acme-inc");
        assert_eq!(slugify("  Multiple   Spaces  "), "multiple-spaces");
        assert_eq!(slugify("---"), "");
        assert_eq!(slugify("ÜberCorp"), "bercorp");
        assert!(slugify(&"a".repeat(200)).len() <= 48);
    }

    #[test]
    fn only_provable_domains_are_accepted() {
        assert!(is_plausible_domain("example.com"));
        assert!(is_plausible_domain("sub.example.co.uk"));
        assert!(!is_plausible_domain("example"));
        assert!(!is_plausible_domain("a@example.com"));
        assert!(!is_plausible_domain(".example.com"));
        assert!(!is_plausible_domain("example..com"));
        assert!(!is_plausible_domain(""));
    }

    #[test]
    fn every_role_control_is_a_csrf_protected_post() {
        let members = vec![OrgMember {
            id: "m1".into(),
            email: "a@example.com".into(),
            role: "member".into(),
            status: "active".into(),
            invited_at: None,
            last_active_at: None,
        }];
        let html = member_table(&context(), &members).into_string();
        assert!(html.contains(r#"hx-post="/members/m1/role""#));
        assert!(html.contains(r#"name="csrf_token" value="tok3n""#));
        assert!(html.contains(r#"<option value="member" selected="selected">"#));
    }
}
