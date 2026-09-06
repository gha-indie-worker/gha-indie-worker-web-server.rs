#![forbid(unsafe_code)]

//! Form scaffolding.
//!
//! [`form`] is the only way a page in this crate should open a `<form>`, because
//! it is what guarantees the CSRF hidden field is present. The shell already
//! puts the token on `hx-headers`, so an htmx submit carries it in a header; the
//! hidden field is what keeps the form working with JavaScript disabled, where
//! the handler verifies the field instead.
//!
//! Every field renders a real `<label for>` bound to a real `id`, so the form is
//! usable by screen readers and by a keyboard alone.

use maud::{html, Markup};

use crate::csrf::CSRF_FIELD;

/// How the form submits.
#[derive(Clone, Debug)]
pub struct FormAction {
    pub action: String,
    /// htmx target selector; `None` swaps the whole body (a full navigation).
    pub target: Option<String>,
    /// htmx swap strategy.
    pub swap: &'static str,
}

impl FormAction {
    #[must_use]
    pub fn post(action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            target: None,
            swap: "outerHTML",
        }
    }

    #[must_use]
    pub fn targeting(mut self, selector: impl Into<String>) -> Self {
        self.target = Some(selector.into());
        self
    }

    #[must_use]
    pub fn swapping(mut self, swap: &'static str) -> Self {
        self.swap = swap;
        self
    }
}

/// Opens a POST form that always carries the CSRF token.
#[must_use]
pub fn form(action: &FormAction, csrf_token: &str, body: Markup) -> Markup {
    let target = action.target.clone().unwrap_or_else(|| "body".to_owned());
    html! {
        form method="post" action=(action.action)
             hx-post=(action.action) hx-target=(target) hx-swap=(action.swap) {
            input type="hidden" name=(CSRF_FIELD) value=(csrf_token);
            (body)
        }
    }
}

/// A text-like input. `kind` is any HTML input type (`text`, `email`, `url`, …).
#[must_use]
pub fn text_field(id: &str, name: &str, label: &str, kind: &str, value: &str, hint: &str, required: bool) -> Markup {
    html! {
        div class="field" {
            label for=(id) { (label) }
            @if required {
                input id=(id) name=(name) type=(kind) value=(value) required="required" autocomplete="off";
            } @else {
                input id=(id) name=(name) type=(kind) value=(value) autocomplete="off";
            }
            @if !hint.is_empty() {
                span class="hint" { (hint) }
            }
        }
    }
}

/// A multi-line input, used for CSV invite pastes and DNS records.
#[must_use]
pub fn textarea_field(id: &str, name: &str, label: &str, value: &str, hint: &str, placeholder: &str) -> Markup {
    html! {
        div class="field" {
            label for=(id) { (label) }
            textarea id=(id) name=(name) placeholder=(placeholder) { (value) }
            @if !hint.is_empty() {
                span class="hint" { (hint) }
            }
        }
    }
}

/// A `<select>`. `options` is `(value, label)`.
#[must_use]
pub fn select_field(id: &str, name: &str, label: &str, options: &[(&str, &str)], selected: &str) -> Markup {
    html! {
        div class="field" {
            label for=(id) { (label) }
            select id=(id) name=(name) {
                @for (value, text) in options {
                    @if *value == selected {
                        option value=(value) selected="selected" { (text) }
                    } @else {
                        option value=(value) { (text) }
                    }
                }
            }
        }
    }
}

/// A number input with explicit bounds — seat counts, retention days.
#[must_use]
pub fn number_field(id: &str, name: &str, label: &str, value: i64, min: i64, max: i64, hint: &str) -> Markup {
    html! {
        div class="field" {
            label for=(id) { (label) }
            input id=(id) name=(name) type="number" value=(value) min=(min) max=(max) required="required";
            @if !hint.is_empty() {
                span class="hint" { (hint) }
            }
        }
    }
}

/// A field-level validation message.
#[must_use]
pub fn field_error(message: &str) -> Markup {
    html! {
        @if !message.is_empty() {
            span class="error" role="alert" { (message) }
        }
    }
}

/// The submit row, optionally with a secondary "back" link.
#[must_use]
pub fn actions(submit_label: &str, back_href: &str, back_label: &str) -> Markup {
    html! {
        div class="row" {
            button type="submit" class="button" { (submit_label) }
            @if !back_label.is_empty() {
                a class="button secondary" href=(back_href) { (back_label) }
            }
            span class="htmx-indicator text-link" { "Working…" }
        }
    }
}

/// A grouped set of fields.
#[must_use]
pub fn fieldset(legend: &str, body: Markup) -> Markup {
    html! {
        fieldset {
            legend { (legend) }
            (body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_form_carries_the_csrf_field() {
        let html = form(&FormAction::post("/onboarding/create"), "tok3n", html! {}).into_string();
        assert!(html.contains(r#"name="csrf_token" value="tok3n""#));
        assert!(html.contains(r#"method="post""#));
        assert!(html.contains(r#"action="/onboarding/create""#));
        assert!(html.contains(r#"hx-post="/onboarding/create""#));
    }

    #[test]
    fn a_form_without_an_explicit_target_swaps_the_body() {
        let html = form(&FormAction::post("/x"), "t", html! {}).into_string();
        assert!(html.contains(r#"hx-target="body""#));
        let targeted = form(
            &FormAction::post("/x").targeting("#panel").swapping("innerHTML"),
            "t",
            html! {},
        )
        .into_string();
        assert!(targeted.contains(r#"hx-target="#panel""#));
        assert!(targeted.contains(r#"hx-swap="innerHTML""#));
    }

    #[test]
    fn labels_are_bound_to_their_controls() {
        let html = text_field("org-name", "name", "Organization name", "text", "", "", true).into_string();
        assert!(html.contains(r#"<label for="org-name">Organization name</label>"#));
        assert!(html.contains(r#"id="org-name" name="name" type="text""#));
        assert!(html.contains(r#"required="required""#));
    }

    #[test]
    fn an_optional_field_is_not_marked_required() {
        let html = text_field("x", "x", "X", "text", "", "hint text", false).into_string();
        assert!(!html.contains("required"));
        assert!(html.contains(r#"<span class="hint">hint text</span>"#));
    }

    #[test]
    fn select_marks_exactly_one_option_selected() {
        let html = select_field(
            "role",
            "role",
            "Role",
            &[("member", "Member"), ("owner", "Owner")],
            "owner",
        )
        .into_string();
        assert_eq!(html.matches("selected=\"selected\"").count(), 1);
        assert!(html.contains(r#"<option value="owner" selected="selected">Owner</option>"#));
    }

    #[test]
    fn prefilled_values_are_escaped() {
        let html = text_field("a", "a", "A", "text", r#"" onfocus="alert(1)"#, "", false).into_string();
        assert!(!html.contains(r#"onfocus="alert(1)"#));
        assert!(html.contains("&quot;"));
    }

    #[test]
    fn a_textarea_escapes_its_body() {
        let html = textarea_field("csv", "csv", "Invites", "</textarea><script>x</script>", "", "").into_string();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;/textarea&gt;"));
    }

    #[test]
    fn the_number_field_states_its_bounds() {
        let html = number_field("seats", "seats", "Seats", 10, 1, 500, "").into_string();
        assert!(html.contains(r#"value="10" min="1" max="500""#));
    }
}
