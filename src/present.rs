//! Presentation rules — the pure half.
//!
//! Three things live here, and all three are decisions that must be right before any HTML exists:
//!
//! * **Which page a (surface, path, signed-in?) triple means.** Host routing is
//!   `lib_core::runtime::surface`'s job; *what to render once the host is known* is this file's.
//! * **What a refused onboarding step says to a human, and next to which field.** Every
//!   `OnboardingError` the domain can produce has exactly one message here, and the mapping is
//!   total.
//! * **How a run status is signalled** — always a word and a shape, never a colour alone.
//!
//! Nothing in this file imports anything. The domain types it mirrors ([`Refusal`], [`Face`]) are
//! converted in [`crate::bridge`], where the compiler checks the match is exhaustive against the
//! real enums. That is the whole reason for the mirror: this file compiles and tests standalone,
//!
//! ```text
//! rustc --edition 2021 --test src/present.rs -o /tmp/present && /tmp/present
//! ```
//!
//! and the bridge cannot silently forget a variant, because a new variant in lib-core fails the
//! bridge's `match` at compile time.

// ---------------------------------------------------------------------------------------------
// Surfaces → pages
// ---------------------------------------------------------------------------------------------

/// The product surfaces this binary renders. Mirror of the five `Surface` values that
/// `surface::Surface::is_product_web` admits; the conversion is in [`crate::bridge::face`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Face {
    /// `www.` and the apex.
    Marketing,
    /// `user.` — B2C.
    User,
    /// `org.` — B2B.
    Org,
    /// `app.` — the signed-in shell.
    App,
    /// `m.` — the mobile shell.
    Mobile,
}

impl Face {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Face::Marketing => "www",
            Face::User => "user",
            Face::Org => "org",
            Face::App => "app",
            Face::Mobile => "m",
        }
    }

    /// The mobile shell renders the same pages as `app.` with different chrome: bottom navigation,
    /// larger targets, and no live-log island.
    #[must_use]
    pub const fn is_small_screen(self) -> bool {
        matches!(self, Face::Mobile)
    }
}

/// A page this server can render. One value per distinct thing a person can be looking at, so the
/// router never decides anything by re-reading the path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Page {
    /// The marketing landing: two equal doors, "for your team" and "for yourself".
    MarketingHome,
    MarketingPricing,

    /// B2C sign-in / sign-up (one form, two framings).
    UserSignIn,
    UserSignUp,
    /// "We sent you a link" — the magic-link interstitial.
    UserCheckEmail,
    /// The magic-link landing.
    UserCallback,
    UserSettings,
    /// The hand-off from an individual account to `org.` — the "actually, this is for my company"
    /// door.
    UserSwitchToOrg,

    OrgSignIn,
    OrgCreate,
    OrgMembers,
    OrgInvitations,
    OrgDomains,
    OrgSso,
    OrgSettings,

    AppDashboard,
    AppRuns,
    /// `/runs/{id}` — the live log tail lives here.
    AppRunDetail(String),
    AppRunners,
    AppCaches,
    AppSettings,

    /// Nothing here for this surface. Rendered as a 404 that says which surface it was.
    NotFound,
}

impl Page {
    /// Whether an unauthenticated visitor may be shown this page at all. The router already
    /// applied `surface::dispose`, so this is the second, page-level gate.
    #[must_use]
    pub const fn is_public(&self) -> bool {
        matches!(
            self,
            Page::MarketingHome
                | Page::MarketingPricing
                | Page::UserSignIn
                | Page::UserSignUp
                | Page::UserCheckEmail
                | Page::UserCallback
                | Page::UserSwitchToOrg
                | Page::OrgSignIn
                | Page::OrgCreate
                | Page::NotFound
        )
    }

    /// The `<title>` for this page, without the product suffix.
    #[must_use]
    pub fn title(&self) -> &'static str {
        match self {
            Page::MarketingHome => "Self-hosted GitHub Actions runners, without the babysitting",
            Page::MarketingPricing => "Pricing",
            Page::UserSignIn => "Sign in",
            Page::UserSignUp => "Create your personal account",
            Page::UserCheckEmail => "Check your email",
            Page::UserCallback => "Signing you in",
            Page::UserSettings => "Personal settings",
            Page::UserSwitchToOrg => "Set this up for your company",
            Page::OrgSignIn => "Organization sign-in",
            Page::OrgCreate => "Create an organization",
            Page::OrgMembers => "Members",
            Page::OrgInvitations => "Seats and invitations",
            Page::OrgDomains => "Domains",
            Page::OrgSso => "Single sign-on",
            Page::OrgSettings => "Organization settings",
            Page::AppDashboard => "Overview",
            Page::AppRuns => "Runs",
            Page::AppRunDetail(_) => "Run",
            Page::AppRunners => "Runners",
            Page::AppCaches => "Caches",
            Page::AppSettings => "Settings",
            Page::NotFound => "Not found",
        }
    }
}

/// Which page a request is asking for.
///
/// `path` is the request path with its query already removed. `authenticated` decides only the
/// landing pages: it never turns a private page into a public one.
#[must_use]
pub fn page_for(face: Face, path: &str, authenticated: bool) -> Page {
    let path = normalize_path(path);
    match face {
        Face::Marketing => match path {
            "/" => Page::MarketingHome,
            "/pricing" => Page::MarketingPricing,
            _ => Page::NotFound,
        },
        Face::User => match path {
            "/" => {
                if authenticated {
                    Page::UserSettings
                } else {
                    Page::UserSignIn
                }
            }
            "/signin" => Page::UserSignIn,
            "/signup" => Page::UserSignUp,
            "/check-email" => Page::UserCheckEmail,
            "/auth/callback" => Page::UserCallback,
            "/settings" => {
                if authenticated {
                    Page::UserSettings
                } else {
                    Page::UserSignIn
                }
            }
            "/for-your-team" => Page::UserSwitchToOrg,
            _ => Page::NotFound,
        },
        Face::Org => match path {
            "/" => {
                if authenticated {
                    Page::OrgMembers
                } else {
                    Page::OrgSignIn
                }
            }
            "/signin" => Page::OrgSignIn,
            "/new" => Page::OrgCreate,
            "/auth/callback" => Page::UserCallback,
            "/members" => gated(authenticated, Page::OrgMembers, Page::OrgSignIn),
            "/invitations" => gated(authenticated, Page::OrgInvitations, Page::OrgSignIn),
            "/domains" => gated(authenticated, Page::OrgDomains, Page::OrgSignIn),
            "/sso" => gated(authenticated, Page::OrgSso, Page::OrgSignIn),
            "/settings" => gated(authenticated, Page::OrgSettings, Page::OrgSignIn),
            _ => Page::NotFound,
        },
        // `app.` is the signed-in shell; `surface::dispose` has already redirected anonymous
        // visitors, so an unauthenticated request that reaches here is a bug and gets nothing.
        Face::App => {
            if authenticated {
                app_page(path)
            } else {
                Page::NotFound
            }
        }
        // `m.` is both: a login surface when anonymous, the app shell when not.
        Face::Mobile => {
            if authenticated {
                app_page(path)
            } else {
                match path {
                    "/" | "/signin" => Page::UserSignIn,
                    "/signup" => Page::UserSignUp,
                    "/check-email" => Page::UserCheckEmail,
                    "/auth/callback" => Page::UserCallback,
                    _ => Page::NotFound,
                }
            }
        }
    }
}

fn gated(authenticated: bool, yes: Page, no: Page) -> Page {
    if authenticated {
        yes
    } else {
        no
    }
}

fn app_page(path: &str) -> Page {
    match path {
        "/" => Page::AppDashboard,
        "/runs" => Page::AppRuns,
        "/runners" => Page::AppRunners,
        "/caches" => Page::AppCaches,
        "/settings" => Page::AppSettings,
        _ => match path.strip_prefix("/runs/") {
            Some(id) if is_run_id(id) => Page::AppRunDetail(id.to_owned()),
            _ => Page::NotFound,
        },
    }
}

/// Run identifiers are ULID-ish: bare `[A-Za-z0-9_-]`, bounded. Anything else is not one, and in
/// particular must never reach a template with a `/` or a `%` still in it.
#[must_use]
pub fn is_run_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Strip a trailing slash (but never the root's), and a query or fragment if one survived.
fn normalize_path(path: &str) -> &str {
    let path = path.split(['?', '#']).next().unwrap_or("/");
    if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    }
}

// ---------------------------------------------------------------------------------------------
// Refusals → messages
// ---------------------------------------------------------------------------------------------

/// The form control an error belongs beside. Also the anchor an error fragment is swapped into,
/// so a message never lands in a generic banner at the top of the page where it is easy to miss.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Field {
    Email,
    Role,
    Seats,
    Domain,
    Invitation,
    Member,
}

impl Field {
    /// The `id` of the input this error belongs to; the label's `for` points at the same id.
    #[must_use]
    pub const fn input_id(self) -> &'static str {
        match self {
            Field::Email => "invite-email",
            Field::Role => "invite-role",
            Field::Seats => "seat-ledger",
            Field::Domain => "domain-name",
            Field::Invitation => "invitation-list",
            Field::Member => "member-list",
        }
    }

    /// The `id` of the live region the error fragment is swapped into.
    #[must_use]
    pub const fn error_id(self) -> &'static str {
        match self {
            Field::Email => "invite-email-error",
            Field::Role => "invite-role-error",
            Field::Seats => "seat-ledger-error",
            Field::Domain => "domain-name-error",
            Field::Invitation => "invitation-list-error",
            Field::Member => "member-list-error",
        }
    }
}

/// A refusal from the tenancy rules, mirrored so this module stays dependency-free.
///
/// One variant per `lib_core::runtime::tenancy::OnboardingError` variant. Adding a variant there
/// breaks [`crate::bridge::refusal`] at compile time, which is the point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    NoSeatsAvailable {
        purchased: u32,
        occupied: u32,
        reserved: u32,
    },
    DomainNotAllowed {
        domain: String,
    },
    InvitationNotOpen,
    InvitationAddressMismatch,
    LastOwner,
    RoleEscalation {
        actor: &'static str,
        granted: &'static str,
    },
    InvalidEmail,
}

/// A message, the field it belongs to, and what to do about it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldError {
    pub field: Field,
    /// The sentence shown next to the control. Written to be actionable and never to blame.
    pub message: String,
    /// The secondary line: the way out. `None` when the message already contains it.
    pub remedy: Option<String>,
    /// Where the "way out" goes, when it is somewhere in this app.
    pub remedy_href: Option<&'static str>,
}

/// Every refusal, as something a person can act on.
///
/// The mapping is total and each arm is specific: a shared "something went wrong" is what makes a
/// seat limit indistinguishable from a typo, and both of those cost a support ticket.
#[must_use]
pub fn explain(refusal: &Refusal) -> FieldError {
    match refusal {
        Refusal::NoSeatsAvailable { purchased, occupied, reserved } => FieldError {
            field: Field::Seats,
            message: format!(
                "All {purchased} {seat} on this plan are spoken for — {occupied} in use, {reserved} held by \
                 invitations nobody has accepted yet.",
                seat = plural(*purchased, "seat", "seats"),
            ),
            remedy: Some(if *reserved > 0 {
                format!(
                    "Add seats, or revoke one of the {reserved} pending {invitation} to free one up.",
                    invitation = plural(*reserved, "invitation", "invitations"),
                )
            } else {
                "Add seats to invite anyone else.".to_owned()
            }),
            remedy_href: Some("/settings#seats"),
        },
        Refusal::DomainNotAllowed { domain } => FieldError {
            field: Field::Email,
            message: format!(
                "This organization only invites addresses at a domain it has verified, and {domain} is not one."
            ),
            remedy: Some(format!("Verify {domain} under Domains, or invite this person at an address you already own.")),
            remedy_href: Some("/domains"),
        },
        Refusal::InvitationNotOpen => FieldError {
            field: Field::Invitation,
            message: "That invitation is no longer open — it has already been accepted, revoked, or it expired."
                .to_owned(),
            remedy: Some("Send a fresh invitation to the same address.".to_owned()),
            remedy_href: Some("/invitations"),
        },
        Refusal::InvitationAddressMismatch => FieldError {
            field: Field::Email,
            message: "That invitation was issued to a different email address.".to_owned(),
            remedy: Some(
                "Sign in with the address the invitation was sent to, or ask an admin to invite the address \
                 you are using."
                    .to_owned(),
            ),
            remedy_href: None,
        },
        Refusal::LastOwner => FieldError {
            field: Field::Member,
            message: "An organization has to keep at least one owner, and this is the last one.".to_owned(),
            remedy: Some("Make someone else an owner first, then change or remove this account.".to_owned()),
            remedy_href: Some("/members"),
        },
        Refusal::RoleEscalation { actor, granted } => FieldError {
            field: Field::Role,
            message: format!("{article} {actor} cannot grant the {granted} role.", article = article(actor)),
            remedy: Some("Ask an owner to make this change.".to_owned()),
            remedy_href: Some("/members"),
        },
        Refusal::InvalidEmail => FieldError {
            field: Field::Email,
            message: "That does not look like an email address we could deliver to.".to_owned(),
            remedy: Some("It should look like name@example.com — check for a stray space or a missing dot.".to_owned()),
            remedy_href: None,
        },
    }
}

/// "A" or "An", so `admin` and `owner` read as English rather than as template output.
fn article(word: &str) -> &'static str {
    match word.as_bytes().first() {
        Some(b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U') => "An",
        _ => "A",
    }
}

fn plural(count: u32, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 {
        one
    } else {
        many
    }
}

// ---------------------------------------------------------------------------------------------
// Status presentation
// ---------------------------------------------------------------------------------------------

/// A run's state. Local to presentation: the canonical set lives in the interfaces contracts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Succeeded => "succeeded",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }
}

/// How a status is signalled: a word, a shape, and a class — never a colour on its own.
///
/// The `glyph` is a text character rather than an icon font so it survives a stylesheet that did
/// not load, and it is marked `aria-hidden` in the template because `label` already says it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusBadge {
    pub label: &'static str,
    pub glyph: &'static str,
    pub class: &'static str,
}

#[must_use]
pub const fn status_badge(status: RunStatus) -> StatusBadge {
    match status {
        RunStatus::Queued => StatusBadge {
            label: "Queued",
            glyph: "◷",
            class: "badge badge-queued",
        },
        RunStatus::Running => StatusBadge {
            label: "Running",
            glyph: "▸",
            class: "badge badge-running",
        },
        RunStatus::Succeeded => StatusBadge {
            label: "Succeeded",
            glyph: "✓",
            class: "badge badge-ok",
        },
        RunStatus::Failed => StatusBadge {
            label: "Failed",
            glyph: "✕",
            class: "badge badge-fail",
        },
        RunStatus::Cancelled => StatusBadge {
            label: "Cancelled",
            glyph: "⊘",
            class: "badge badge-muted",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marketing_surface_offers_only_the_marketing_pages() {
        assert_eq!(page_for(Face::Marketing, "/", false), Page::MarketingHome);
        assert_eq!(
            page_for(Face::Marketing, "/pricing", false),
            Page::MarketingPricing
        );
        assert_eq!(page_for(Face::Marketing, "/runs", true), Page::NotFound);
        assert_eq!(page_for(Face::Marketing, "/members", true), Page::NotFound);
    }

    #[test]
    fn each_login_surface_lands_on_its_own_door() {
        assert_eq!(page_for(Face::User, "/", false), Page::UserSignIn);
        assert_eq!(page_for(Face::Org, "/", false), Page::OrgSignIn);
        // An org visitor is never handed the individual sign-in, and vice versa.
        assert_eq!(page_for(Face::Org, "/signup", false), Page::NotFound);
        assert_eq!(page_for(Face::User, "/members", true), Page::NotFound);
    }

    #[test]
    fn the_two_doors_are_reachable_from_each_other() {
        assert_eq!(
            page_for(Face::User, "/for-your-team", false),
            Page::UserSwitchToOrg
        );
        assert_eq!(page_for(Face::Org, "/new", false), Page::OrgCreate);
    }

    #[test]
    fn signed_in_landings_differ_from_anonymous_ones() {
        assert_eq!(page_for(Face::User, "/", true), Page::UserSettings);
        assert_eq!(page_for(Face::Org, "/", true), Page::OrgMembers);
        assert_eq!(page_for(Face::App, "/", true), Page::AppDashboard);
    }

    #[test]
    fn private_org_pages_fall_back_to_the_org_sign_in_when_anonymous() {
        for path in ["/members", "/invitations", "/domains", "/sso", "/settings"] {
            assert_eq!(
                page_for(Face::Org, path, false),
                Page::OrgSignIn,
                "path {path}"
            );
            assert!(
                page_for(Face::Org, path, true).is_public().eq(&false),
                "path {path} is private"
            );
        }
    }

    #[test]
    fn the_app_shell_renders_nothing_to_an_anonymous_request() {
        for path in [
            "/",
            "/runs",
            "/runs/01HZ",
            "/runners",
            "/caches",
            "/settings",
        ] {
            assert_eq!(
                page_for(Face::App, path, false),
                Page::NotFound,
                "path {path}"
            );
        }
    }

    #[test]
    fn the_mobile_surface_is_a_login_door_then_the_app_shell() {
        assert_eq!(page_for(Face::Mobile, "/", false), Page::UserSignIn);
        assert_eq!(page_for(Face::Mobile, "/runs", false), Page::NotFound);
        assert_eq!(page_for(Face::Mobile, "/", true), Page::AppDashboard);
        assert_eq!(page_for(Face::Mobile, "/runs", true), Page::AppRuns);
        assert!(Face::Mobile.is_small_screen());
        assert!(!Face::App.is_small_screen());
    }

    #[test]
    fn run_detail_captures_the_identifier_and_refuses_junk() {
        assert_eq!(
            page_for(Face::App, "/runs/01HZY7Q0J8", true),
            Page::AppRunDetail("01HZY7Q0J8".to_owned())
        );
        for bad in [
            "/runs/a/b",
            "/runs/../secret",
            "/runs/a%2fb",
            &format!("/runs/{}", "a".repeat(65)),
        ] {
            assert_eq!(page_for(Face::App, bad, true), Page::NotFound, "path {bad}");
        }
    }

    #[test]
    fn trailing_slashes_and_queries_do_not_change_the_page() {
        // `/runs/` is the collection, not a detail page with an empty identifier.
        assert_eq!(page_for(Face::App, "/runs/", true), Page::AppRuns);
        assert_eq!(
            page_for(Face::App, "/runs?status=failed", true),
            Page::AppRuns
        );
        assert_eq!(
            page_for(Face::Marketing, "/#top", false),
            Page::MarketingHome
        );
    }

    #[test]
    fn public_and_private_pages_are_labelled() {
        assert!(Page::MarketingHome.is_public());
        assert!(Page::UserSignIn.is_public());
        assert!(Page::OrgCreate.is_public());
        assert!(!Page::AppRuns.is_public());
        assert!(!Page::OrgMembers.is_public());
        assert!(!Page::AppRunDetail("x".into()).is_public());
    }

    // -- refusals ------------------------------------------------------------------------------

    fn every_refusal() -> Vec<Refusal> {
        vec![
            Refusal::NoSeatsAvailable {
                purchased: 5,
                occupied: 4,
                reserved: 1,
            },
            Refusal::DomainNotAllowed {
                domain: "contractor.example".into(),
            },
            Refusal::InvitationNotOpen,
            Refusal::InvitationAddressMismatch,
            Refusal::LastOwner,
            Refusal::RoleEscalation {
                actor: "admin",
                granted: "owner",
            },
            Refusal::InvalidEmail,
        ]
    }

    #[test]
    fn every_refusal_has_a_specific_actionable_message() {
        let mut seen: Vec<String> = Vec::new();
        for refusal in every_refusal() {
            let error = explain(&refusal);
            assert!(
                error.message.len() > 24,
                "{refusal:?} has a stub message: {}",
                error.message
            );
            assert!(
                error.message.ends_with('.'),
                "{refusal:?} is not a sentence"
            );
            assert!(
                !error
                    .message
                    .to_lowercase()
                    .contains("something went wrong"),
                "{refusal:?} fell back to a generic message"
            );
            assert!(
                !seen.contains(&error.message),
                "{refusal:?} reuses another message"
            );
            seen.push(error.message);
        }
        assert_eq!(seen.len(), 7, "one message per OnboardingError variant");
    }

    #[test]
    fn each_refusal_points_at_the_control_that_caused_it() {
        assert_eq!(explain(&Refusal::InvalidEmail).field, Field::Email);
        assert_eq!(
            explain(&Refusal::DomainNotAllowed {
                domain: "x.test".into()
            })
            .field,
            Field::Email
        );
        assert_eq!(
            explain(&Refusal::RoleEscalation {
                actor: "member",
                granted: "admin"
            })
            .field,
            Field::Role
        );
        assert_eq!(
            explain(&Refusal::NoSeatsAvailable {
                purchased: 1,
                occupied: 1,
                reserved: 0
            })
            .field,
            Field::Seats
        );
        assert_eq!(explain(&Refusal::LastOwner).field, Field::Member);
        assert_eq!(
            explain(&Refusal::InvitationNotOpen).field,
            Field::Invitation
        );
        assert_eq!(
            explain(&Refusal::InvitationAddressMismatch).field,
            Field::Email
        );
    }

    #[test]
    fn the_seat_message_reports_the_actual_ledger_and_pluralises() {
        let full = explain(&Refusal::NoSeatsAvailable {
            purchased: 12,
            occupied: 9,
            reserved: 3,
        });
        assert!(full.message.contains("All 12 seats"), "{}", full.message);
        assert!(full.message.contains("9 in use"), "{}", full.message);
        assert!(
            full.message.contains("3 held by invitations"),
            "{}",
            full.message
        );
        assert!(full
            .remedy
            .unwrap()
            .contains("revoke one of the 3 pending invitations"));

        let single = explain(&Refusal::NoSeatsAvailable {
            purchased: 1,
            occupied: 1,
            reserved: 0,
        });
        assert!(single.message.contains("All 1 seat "), "{}", single.message);
        assert_eq!(
            single.remedy.as_deref(),
            Some("Add seats to invite anyone else.")
        );
    }

    #[test]
    fn the_domain_message_names_the_domain_that_was_refused() {
        let error = explain(&Refusal::DomainNotAllowed {
            domain: "contractor.example".into(),
        });
        assert!(error.message.contains("contractor.example"));
        assert_eq!(error.remedy_href, Some("/domains"));
    }

    #[test]
    fn the_escalation_message_names_both_roles() {
        let error = explain(&Refusal::RoleEscalation {
            actor: "admin",
            granted: "owner",
        });
        assert_eq!(error.message, "An admin cannot grant the owner role.");
        let member = explain(&Refusal::RoleEscalation {
            actor: "member",
            granted: "admin",
        });
        assert_eq!(member.message, "A member cannot grant the admin role.");
    }

    #[test]
    fn every_field_has_a_distinct_input_and_error_anchor() {
        let fields = [
            Field::Email,
            Field::Role,
            Field::Seats,
            Field::Domain,
            Field::Invitation,
            Field::Member,
        ];
        let mut ids: Vec<&str> = Vec::new();
        for field in fields {
            assert!(!ids.contains(&field.input_id()));
            ids.push(field.input_id());
            assert!(!ids.contains(&field.error_id()));
            ids.push(field.error_id());
        }
        assert_eq!(ids.len(), 12);
    }

    #[test]
    fn status_is_never_signalled_by_colour_alone() {
        for status in [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::Succeeded,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            let badge = status_badge(status);
            assert!(!badge.label.is_empty(), "{status:?} has no word");
            assert!(!badge.glyph.is_empty(), "{status:?} has no shape");
            assert!(badge.class.starts_with("badge "), "{status:?} has no class");
        }
        assert_ne!(
            status_badge(RunStatus::Failed).glyph,
            status_badge(RunStatus::Succeeded).glyph
        );
        assert_ne!(
            status_badge(RunStatus::Failed).label,
            status_badge(RunStatus::Succeeded).label
        );
    }
}
