#![forbid(unsafe_code)]

//! The ores-chat widget, in two shapes.
//!
//! * **Visitor** — on the `app.` marketing home, for someone who has not signed
//!   in. Sales context; no account data is ever attached.
//! * **Customer** — on signed-in dashboards. Support context; the session's
//!   bearer is forwarded by the server, never by the page.
//!
//! Both open a session through the api-server (`POST /v1/chat/sessions`) via
//! [`crate::chat`]; the browser never talks to ores-chat directly, so no
//! third-party origin appears in the CSP and no third-party script is loaded.
//!
//! Transport is htmx: `hx-ext="ws"` streams when the relay is available, and the
//! same container polls every few seconds when it is not.

use maud::{html, Markup};

use crate::csrf::CSRF_FIELD;
use crate::ui::layout::PageContext;

/// Who the widget is talking to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatAudience {
    /// Signed-out visitor on a marketing surface. Sales.
    Visitor,
    /// Signed-in customer on a product surface. Support.
    Customer,
}

impl ChatAudience {
    /// The `surface` value sent to `POST /v1/chat/sessions`.
    #[must_use]
    pub const fn api_surface(self) -> &'static str {
        match self {
            Self::Visitor => "visitor",
            Self::Customer => "customer",
        }
    }

    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Visitor => "Talk to us",
            Self::Customer => "Support",
        }
    }

    #[must_use]
    pub const fn prompt(self) -> &'static str {
        match self {
            Self::Visitor => "Questions about runners, plans or pricing? Ask here.",
            Self::Customer => "Describe what you were doing when it went wrong. We can see your run ids.",
        }
    }
}

/// Chooses the widget for the current page: customers get support, everyone
/// else gets sales.
#[must_use]
pub fn audience_for(context: &PageContext) -> ChatAudience {
    if context.is_signed_in() {
        ChatAudience::Customer
    } else {
        ChatAudience::Visitor
    }
}

/// The collapsed-by-default widget. `<details>` means it works with no
/// JavaScript at all, and needs no click handler to open.
#[must_use]
pub fn widget(context: &PageContext, audience: ChatAudience) -> Markup {
    let start_path = format!("/chat/session?surface={}", audience.api_surface());
    html! {
        details class="chat" data-audience=(audience.api_surface()) {
            summary { span { (audience.summary()) } span class="live-dot" aria-hidden="true" {} }
            div class="chat-body" {
                p { (audience.prompt()) }
                div id="chat-log" class="chat-log" role="log" aria-live="polite"
                    hx-get=(start_path) hx-trigger="toggle once from:closest details" hx-swap="innerHTML" {
                    span class="text-link" { "Not connected yet." }
                }
                form method="post" action="/chat/messages"
                     hx-post="/chat/messages" hx-target="#chat-log" hx-swap="beforeend" {
                    input type="hidden" name=(CSRF_FIELD) value=(context.csrf_token);
                    input type="hidden" name="surface" value=(audience.api_surface());
                    div class="field" {
                        label for="chat-message" { "Message" }
                        textarea id="chat-message" name="message" placeholder="Type your message" {}
                    }
                    button type="submit" class="button" { "Send" }
                }
            }
        }
    }
}

/// The streaming variant of the log container, used once a session id exists.
#[must_use]
pub fn live_log(session_id: &str, messages: &[(String, String)]) -> Markup {
    let ws_path = format!("/ws?chat_session={session_id}");
    let poll_path = format!("/chat/messages?session={session_id}");
    html! {
        div id="chat-log" class="chat-log" role="log" aria-live="polite"
            hx-ext="ws" ws-connect=(ws_path)
            hx-get=(poll_path) hx-trigger="every 5s" hx-swap="innerHTML" {
            (message_list(messages))
        }
    }
}

/// Rendered messages. `(author, body)`.
#[must_use]
pub fn message_list(messages: &[(String, String)]) -> Markup {
    html! {
        @for (author, body) in messages {
            p { strong { (author) } ": " (body) }
        }
        @if messages.is_empty() {
            span class="text-link" { "No messages yet." }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::Surface;
    use crate::session::{Actor, ActorSource};

    fn signed_in(context: &mut PageContext) {
        context.actor = Some(Actor {
            sub: "u1".into(),
            email: Some("a@example.com".into()),
            org_id: None,
            roles: vec![],
            scopes: vec![],
            acr: None,
            source: ActorSource::Session,
            access_token: "tok".into(),
        });
    }

    #[test]
    fn visitors_get_the_sales_widget_and_customers_the_support_widget() {
        let mut context = PageContext::for_tests(Surface::App);
        assert_eq!(audience_for(&context), ChatAudience::Visitor);
        signed_in(&mut context);
        assert_eq!(audience_for(&context), ChatAudience::Customer);
    }

    #[test]
    fn the_widget_names_the_api_surface_it_will_open() {
        let context = PageContext::for_tests(Surface::App);
        let html = widget(&context, ChatAudience::Visitor).into_string();
        assert!(html.contains(r#"data-audience="visitor""#));
        assert!(html.contains(r#"hx-get="/chat/session?surface=visitor""#));

        let support = widget(&context, ChatAudience::Customer).into_string();
        assert!(support.contains(r#"data-audience="customer""#));
        assert!(support.contains("surface=customer"));
    }

    #[test]
    fn the_widget_never_loads_a_third_party_script() {
        let html = widget(&PageContext::for_tests(Surface::App), ChatAudience::Visitor).into_string();
        assert!(!html.contains("<script"));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn sending_a_message_carries_the_csrf_token() {
        let html = widget(&PageContext::for_tests(Surface::App), ChatAudience::Visitor).into_string();
        assert!(html.contains(r#"name="csrf_token" value="tok3n""#));
    }

    #[test]
    fn the_live_log_streams_with_a_polling_fallback() {
        let html = live_log("cs_1", &[]).into_string();
        assert!(html.contains(r#"ws-connect="/ws?chat_session=cs_1""#));
        assert!(html.contains(r#"hx-get="/chat/messages?session=cs_1""#));
        assert!(html.contains("No messages yet."));
    }

    #[test]
    fn message_bodies_are_escaped() {
        let html = message_list(&[("a".to_owned(), "<script>x</script>".to_owned())]).into_string();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
