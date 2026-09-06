#![forbid(unsafe_code)]
//! Form plumbing shared by every surface: the CSRF field, validation-error blocks, and the
//! decision about whether a POST answers with a fragment or a redirect.
//!
//! Every form in this product is a real `<form method="post" action="…">`. htmx is layered on top
//! with `hx-post`/`hx-target`, so a person with script disabled, a flaky network, or a browser we
//! did not anticipate gets a full page reload and the same outcome. That is not a nicety: it is
//! why `build.rs` can leave htmx out entirely and the product still works.
//!
//! Errors are rendered **next to the control that caused them**, into a live region whose `id` the
//! input names in `aria-describedby`. `crate::present::explain` decides which control that is, for
//! every `OnboardingError` there is.

use axum::http::HeaderMap;
use maud::{html, Markup};

use crate::present::{Field, FieldError};

/// The hidden CSRF field every state-changing form carries.
#[must_use]
pub fn csrf_field(token: &str) -> Markup {
    html! {
        input type="hidden" name="csrf_token" value=(token);
    }
}

/// An empty live region for a field's errors. Rendered even when there is nothing to say, so that
/// htmx has a stable target to swap into and a screen reader has a region that already exists when
/// the message arrives.
#[must_use]
pub fn error_slot(field: Field) -> Markup {
    html! {
        p class="field-error" id=(field.error_id()) role="alert" aria-live="polite" {}
    }
}

/// One rendered validation error, in the shape [`error_slot`] leaves room for.
#[must_use]
pub fn error_block(error: &FieldError) -> Markup {
    html! {
        p class="field-error" id=(error.field.error_id()) role="alert" aria-live="polite" {
            span class="message" { (error.message) }
            @if let Some(remedy) = &error.remedy {
                span class="remedy" {
                    (remedy)
                    @if let Some(href) = error.remedy_href {
                        " "
                        a href=(href) { "Go there" }
                    }
                }
            }
        }
    }
}

/// What a form has to say to the person who just submitted it.
///
/// A panel renders with one of these, and every slot in it asks the same value where its message
/// should go. That is what keeps a swapped fragment from carrying a second element with an `id`
/// the page already has — a duplicate `id` breaks `aria-describedby`, and a screen reader then
/// announces the wrong thing, or nothing.
#[derive(Clone, Copy, Debug, Default)]
pub enum Feedback<'a> {
    /// Nothing happened; render empty slots.
    #[default]
    Quiet,
    /// It worked. The message goes in `field`'s slot.
    Done(Field, &'a str),
    /// It was refused. The message goes in the slot `explain` chose.
    Refused(&'a FieldError),
}

impl Feedback<'_> {
    /// Render `field`'s slot: the message if this feedback belongs there, an empty live region
    /// otherwise. Every slot on a panel calls this, so exactly one of them ever has content.
    #[must_use]
    pub fn slot(self, field: Field) -> Markup {
        match self {
            Feedback::Done(target, message) if target == field => confirmation(field, message),
            Feedback::Refused(error) if error.field == field => error_block(error),
            _ => error_slot(field),
        }
    }
}

/// A confirmation, in the same slot an error would have used, so success and failure do not land
/// in different places on the page.
#[must_use]
pub fn confirmation(field: Field, message: &str) -> Markup {
    html! {
        p class="notice notice-ok" id=(field.error_id()) role="status" aria-live="polite" {
            (message)
        }
    }
}

/// Whether this request came from htmx and therefore wants a fragment rather than a redirect.
///
/// The header is a hint from the client, not a permission: every handler does the same work and
/// makes the same authorization decisions either way, and only the *shape* of the answer changes.
#[must_use]
pub fn wants_fragment(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::present::{explain, Refusal};

    #[test]
    fn an_error_block_lands_in_the_slot_the_input_points_at() {
        let error = explain(&Refusal::InvalidEmail);
        let slot = error_slot(error.field).into_string();
        let block = error_block(&error).into_string();
        assert!(slot.contains("id=\"invite-email-error\""));
        assert!(block.contains("id=\"invite-email-error\""));
        assert!(block.contains("role=\"alert\""));
        assert!(block.contains(&error.message));
    }

    #[test]
    fn every_refusal_renders_a_message_and_a_way_out() {
        for refusal in [
            Refusal::NoSeatsAvailable {
                purchased: 2,
                occupied: 2,
                reserved: 0,
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
        ] {
            let error = explain(&refusal);
            let html = error_block(&error).into_string();
            assert!(
                html.contains("class=\"message\""),
                "{refusal:?} rendered no message"
            );
            if error.remedy.is_some() {
                assert!(
                    html.contains("class=\"remedy\""),
                    "{refusal:?} rendered no remedy"
                );
            }
        }
    }

    #[test]
    fn markup_escapes_whatever_a_person_typed() {
        let error = explain(&Refusal::DomainNotAllowed {
            domain: "<script>alert(1)</script>".into(),
        });
        let html = error_block(&error).into_string();
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn exactly_one_slot_on_a_panel_ever_carries_a_message() {
        let error = explain(&Refusal::InvalidEmail);
        let feedback = Feedback::Refused(&error);
        let fields = [
            Field::Email,
            Field::Role,
            Field::Seats,
            Field::Domain,
            Field::Invitation,
            Field::Member,
        ];
        let filled: Vec<Field> = fields
            .into_iter()
            .filter(|field| {
                feedback
                    .slot(*field)
                    .into_string()
                    .contains("class=\"message\"")
            })
            .collect();
        assert_eq!(filled, vec![Field::Email]);

        let done = Feedback::Done(Field::Invitation, "Invitation sent.");
        let filled: Vec<Field> = fields
            .into_iter()
            .filter(|field| done.slot(*field).into_string().contains("Invitation sent."))
            .collect();
        assert_eq!(filled, vec![Field::Invitation]);

        // And a quiet panel renders every slot empty, so htmx and the screen reader both have a
        // region that already exists when a message eventually arrives.
        for field in fields {
            let markup = Feedback::Quiet.slot(field).into_string();
            assert!(markup.contains(field.error_id()), "{field:?}");
            assert!(!markup.contains("class=\"message\""), "{field:?}");
        }
    }

    #[test]
    fn a_panel_never_repeats_an_element_id() {
        // The six slots are the only elements a panel renders with a generated id, and they are
        // distinct by construction. This is the assertion that a duplicate would break.
        let mut ids: Vec<&str> = Vec::new();
        for field in [
            Field::Email,
            Field::Role,
            Field::Seats,
            Field::Domain,
            Field::Invitation,
            Field::Member,
        ] {
            assert!(!ids.contains(&field.error_id()));
            ids.push(field.error_id());
        }
        assert_eq!(ids.len(), 6);
    }

    #[test]
    fn the_htmx_hint_is_read_exactly() {
        let mut headers = HeaderMap::new();
        assert!(!wants_fragment(&headers));
        headers.insert("hx-request", "true".parse().unwrap());
        assert!(wants_fragment(&headers));
        headers.insert("hx-request", "TRUE".parse().unwrap());
        assert!(wants_fragment(&headers));
        headers.insert("hx-request", "false".parse().unwrap());
        assert!(!wants_fragment(&headers));
    }
}
