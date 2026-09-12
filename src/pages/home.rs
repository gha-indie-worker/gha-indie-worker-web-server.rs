#![forbid(unsafe_code)]
//! `www.` and the apex — the landing page.
//!
//! The central job of this page is one decision: **two doors, equal weight.** Most products bury
//! "for teams" behind a "Contact sales" link and make the individual sign-up the only real button,
//! or the reverse. Both are the same mistake: the visitor knows which of the two they are, and the
//! page's only job is to let them say so without reading a paragraph first.
//!
//! So the doors are a two-column grid of identical cards — same size, same border, same call to
//! action, neither styled as a secondary action — and they stack in source order on a narrow
//! screen, so a screen reader hears the same two options in the same order a sighted visitor sees.
//! Each card says who it is for, what you get, and where it takes you, in that order.

use maud::{html, Markup};

use crate::present::Page;
use crate::state::{AppState, Ctx};

use super::layout::surface_url;

/// Render the marketing surface.
#[must_use]
pub fn render(state: &AppState, ctx: &Ctx, page: &Page) -> Markup {
    let apex = &state.config.apex;
    let secure = state.config.cookies_are_secure();
    match page {
        Page::MarketingHome => landing(apex, secure),
        Page::MarketingPricing => pricing(apex, secure),
        _ => super::not_found(ctx),
    }
}

fn landing(apex: &str, secure: bool) -> Markup {
    let org = surface_url(apex, secure, "org", "/new");
    let org_signin = surface_url(apex, secure, "org", "/");
    let user = surface_url(apex, secure, "user", "/signup");
    let user_signin = surface_url(apex, secure, "user", "/");
    html! {
        section class="hero stack" {
            p class="eyebrow" { "Self-hosted GitHub Actions runners" }
            h1 { "Your own runners, without the babysitting." }
            p class="lede" {
                "GHA Indie Worker keeps a pool of your own machines registered, patched, warm and \
                 honest about what they are doing — and gives you the build log while it is still \
                 running, not after it has finished."
            }
        }

        // The two doors. Identical markup, identical styling, in the order the visitor is most
        // likely to need: the team case first, because it is the one that needs a decision made
        // *before* anyone signs up, and the individual case is a single click either way.
        section class="stack" aria-labelledby="doors-heading" {
            h2 id="doors-heading" class="visually-hidden" { "Choose how you want to start" }
            div class="doors" {
                a class="door" href=(org) {
                    p class="eyebrow" { "For your team" }
                    h2 { "Set this up for my company" }
                    p class="muted" {
                        "An organization with seats, invitations, roles and a verified domain. \
                         Everyone's runs in one place."
                    }
                    ul {
                        li { "Invite people by email, or let anyone at your domain join" }
                        li { "Owner, admin, member and viewer roles" }
                        li { "One bill, seats you can move between people" }
                    }
                    span class="door-go" aria-hidden="true" { "Create an organization →" }
                }
                a class="door" href=(user) {
                    p class="eyebrow" { "For yourself" }
                    h2 { "Just me, for now" }
                    p class="muted" {
                        "A personal workspace you own outright. No organization, no seats, nothing \
                         to administer."
                    }
                    ul {
                        li { "Sign in with a link — no password to lose" }
                        li { "Your own repositories and your own runners" }
                        li { "Move to an organization later without starting again" }
                    }
                    span class="door-go" aria-hidden="true" { "Create a personal account →" }
                }
            }
            p class="faint" {
                "Already have an account? "
                a href=(org_signin) { "Sign in to your organization" }
                " or "
                a href=(user_signin) { "sign in as yourself" }
                "."
            }
        }

        section class="stack" aria-labelledby="proof-heading" {
            h2 id="proof-heading" { "What you actually get" }
            div class="proof" {
                div class="item" {
                    h3 { "Live logs, not a spinner" }
                    p {
                        "The log tail is a real stream with credit-based backpressure, so a huge \
                         build does not drown your browser and a dropped connection resumes where \
                         it stopped instead of starting over."
                    }
                }
                div class="item" {
                    h3 { "Runners that report in" }
                    p {
                        "Every runner says what it is, what it is running, and when it was last \
                         heard from. A runner that has gone quiet is shown as quiet, not as idle."
                    }
                }
                div class="item" {
                    h3 { "Caches you can see" }
                    p {
                        "Cache hits and sizes per repository, so the answer to \"why is CI slow \
                         today\" is on a page rather than in a support thread."
                    }
                }
                div class="item" {
                    h3 { "Nothing you cannot leave" }
                    p {
                        "Standard GitHub Actions runners on machines you control. Turn this off \
                         and the runners keep working."
                    }
                }
            }
        }
    }
}

fn pricing(apex: &str, secure: bool) -> Markup {
    let org = surface_url(apex, secure, "org", "/new");
    let user = surface_url(apex, secure, "user", "/signup");
    html! {
        section class="stack" {
            h1 { "Pricing" }
            p class="lede" {
                "Per seat, per month. A seat is a person who can see your builds — including a \
                 viewer, because reading a private build log is the thing worth paying for."
            }
            div class="doors" {
                div class="door" {
                    p class="eyebrow" { "For yourself" }
                    h2 { "Personal" }
                    p class="muted" { "One person, unlimited runners you host yourself." }
                    ul {
                        li { "Everything in the product" }
                        li { "No seat accounting to think about" }
                    }
                    p class="door-go" { a class="button button-primary" href=(user) { "Start" } }
                }
                div class="door" {
                    p class="eyebrow" { "For your team" }
                    h2 { "Organization" }
                    p class="muted" { "Seats, roles, domain verification, one invoice." }
                    ul {
                        li { "Everything in Personal, per seat" }
                        li { "Invitations hold a seat until they are accepted or revoked" }
                        li { "Single sign-on is on the roadmap, not in this release" }
                    }
                    p class="door-go" { a class="button button-primary" href=(org) { "Create an organization" } }
                }
            }
            p class="faint" {
                "Seat counts are shown live on your organization's "
                "\u{201c}Seats and invitations\u{201d} page, computed from members and open \
                 invitations rather than from a counter that can drift."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_doors_are_the_same_shape_and_neither_is_secondary() {
        let markup = landing("indiebuild.dev", true).into_string();
        let doors = markup.matches("class=\"door\"").count();
        assert_eq!(doors, 2, "the landing page offers exactly two doors");
        // Neither door is styled as a lesser action: there is no `button-quiet` and no
        // `button-primary` singling one of them out.
        assert!(!markup.contains("button-quiet"));
        assert!(!markup.contains("button-primary"));
        assert!(markup.contains("https://org.indiebuild.dev/new"));
        assert!(markup.contains("https://user.indiebuild.dev/signup"));
    }

    #[test]
    fn the_team_door_comes_first_in_the_source_so_it_does_on_a_phone_too() {
        let markup = landing("indiebuild.dev", true).into_string();
        let team = markup.find("For your team").expect("the team door");
        let solo = markup.find("For yourself").expect("the individual door");
        assert!(
            team < solo,
            "the doors must not be visually reordered away from source order"
        );
    }

    #[test]
    fn both_sign_in_paths_are_offered_to_someone_who_already_has_an_account() {
        let markup = landing("indiebuild.dev", true).into_string();
        assert!(markup.contains("https://org.indiebuild.dev/"));
        assert!(markup.contains("https://user.indiebuild.dev/"));
        assert!(markup.contains("Sign in to your organization"));
    }
}
