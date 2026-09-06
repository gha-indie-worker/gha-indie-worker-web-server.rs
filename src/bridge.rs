#![forbid(unsafe_code)]
//! The seam between `gha-indie-worker-lib-core`'s domain types and this crate's presentation
//! mirrors.
//!
//! Every conversion here is a total `match` on a lib-core enum. That is the entire purpose of the
//! file: [`crate::present`] must stay dependency-free so it can be compiled and tested on its own,
//! and the price of that is one mirror per enum. The price is worth paying only if forgetting a
//! variant is impossible — which it is, because adding a variant to `OnboardingError` or `Surface`
//! makes these `match` expressions non-exhaustive and the crate stops compiling.

use gha_indie_worker_lib_core::runtime::session::{ErrorCode, Frame};
use gha_indie_worker_lib_core::runtime::surface::Surface;
use gha_indie_worker_lib_core::runtime::tenancy::{OnboardingError, Role};

use crate::present::{Face, Refusal};
use crate::wire::WireFrame;

/// The product face a surface is rendered as, or `None` for a surface this binary must not serve.
#[must_use]
pub const fn face(surface: Surface) -> Option<Face> {
    Some(match surface {
        Surface::Marketing => Face::Marketing,
        Surface::User => Face::User,
        Surface::Org => Face::Org,
        Surface::App => Face::App,
        Surface::Mobile => Face::Mobile,
        // `api.` and `auth.` are other binaries; `admin.` and `admin-api.` are another *plane*.
        // `surface::dispose` already answers 404 for all four — this arm exists so that a new
        // surface in lib-core cannot be silently rendered here by default.
        Surface::Api | Surface::Auth | Surface::Admin | Surface::AdminApi => return None,
    })
}

/// A tenancy refusal, in the shape [`crate::present::explain`] can talk about.
#[must_use]
pub fn refusal(error: &OnboardingError) -> Refusal {
    match error {
        OnboardingError::NoSeatsAvailable {
            purchased,
            occupied,
            reserved,
        } => Refusal::NoSeatsAvailable {
            purchased: *purchased,
            occupied: *occupied,
            reserved: *reserved,
        },
        OnboardingError::DomainNotAllowed { domain } => Refusal::DomainNotAllowed {
            domain: domain.clone(),
        },
        OnboardingError::InvitationNotOpen => Refusal::InvitationNotOpen,
        OnboardingError::InvitationAddressMismatch => Refusal::InvitationAddressMismatch,
        OnboardingError::LastOwner => Refusal::LastOwner,
        OnboardingError::RoleEscalation { actor, granted } => Refusal::RoleEscalation {
            actor: role_word(*actor),
            granted: role_word(*granted),
        },
        OnboardingError::InvalidEmail => Refusal::InvalidEmail,
    }
}

/// `Role::as_str` returns a `&'static str` already; this exists so the presentation mirror never
/// has to know the domain type.
#[must_use]
pub const fn role_word(role: Role) -> &'static str {
    role.as_str()
}

// ---------------------------------------------------------------------------------------------
// Session frames <-> the JSON wire format
// ---------------------------------------------------------------------------------------------

/// A `session::Frame` in the shape [`crate::wire`] can encode.
///
/// Total by construction: a frame added to lib-core stops this `match` compiling, which is the
/// only guarantee worth having that a new frame cannot silently become unsendable to a browser.
#[must_use]
pub fn to_wire(frame: &Frame) -> WireFrame {
    match frame {
        Frame::Hello { version, resume } => WireFrame::Hello {
            version: *version,
            resume: resume.clone(),
        },
        Frame::Welcome {
            session,
            resumed,
            heartbeat_seconds,
        } => WireFrame::Welcome {
            session: session.clone(),
            resumed: *resumed,
            heartbeat_seconds: *heartbeat_seconds,
        },
        Frame::Ping { nonce } => WireFrame::Ping { nonce: *nonce },
        Frame::Pong { nonce } => WireFrame::Pong { nonce: *nonce },
        Frame::Subscribe {
            stream,
            from,
            credit,
        } => WireFrame::Subscribe {
            stream: stream.clone(),
            from: *from,
            credit: *credit,
        },
        Frame::Unsubscribe { stream } => WireFrame::Unsubscribe {
            stream: stream.clone(),
        },
        Frame::Credit { stream, additional } => WireFrame::Credit {
            stream: stream.clone(),
            additional: *additional,
        },
        Frame::Event {
            stream,
            sequence,
            payload,
        } => WireFrame::Event {
            stream: stream.clone(),
            sequence: *sequence,
            payload: payload.clone(),
        },
        Frame::Lagged { stream, skipped_to } => WireFrame::Lagged {
            stream: stream.clone(),
            skipped_to: *skipped_to,
        },
        Frame::Command { id, verb, payload } => WireFrame::Command {
            id: *id,
            verb: verb.clone(),
            payload: payload.clone(),
        },
        Frame::Ack { id, payload } => WireFrame::Ack {
            id: *id,
            payload: payload.clone(),
        },
        Frame::Error { id, code, message } => WireFrame::Error {
            id: *id,
            code: code.as_str().to_owned(),
            message: message.clone(),
        },
        Frame::Close { code, message } => WireFrame::Close {
            code: code.as_str().to_owned(),
            message: message.clone(),
        },
    }
}

/// The inverse. `None` for a frame a browser has no business sending — `Event`, `Lagged`,
/// `Welcome` and `Ack` are server-to-client only, and accepting one from a client would let a page
/// inject lines into its own log view and, worse, into whatever reads it next.
#[must_use]
pub fn from_wire(frame: &WireFrame) -> Option<Frame> {
    Some(match frame {
        WireFrame::Hello { version, resume } => Frame::Hello {
            version: *version,
            resume: resume.clone(),
        },
        WireFrame::Ping { nonce } => Frame::Ping { nonce: *nonce },
        WireFrame::Pong { nonce } => Frame::Pong { nonce: *nonce },
        WireFrame::Subscribe {
            stream,
            from,
            credit,
        } => Frame::Subscribe {
            stream: stream.clone(),
            from: *from,
            credit: *credit,
        },
        WireFrame::Unsubscribe { stream } => Frame::Unsubscribe {
            stream: stream.clone(),
        },
        WireFrame::Credit { stream, additional } => Frame::Credit {
            stream: stream.clone(),
            additional: *additional,
        },
        WireFrame::Command { id, verb, payload } => Frame::Command {
            id: *id,
            verb: verb.clone(),
            payload: payload.clone(),
        },
        WireFrame::Close { code, message } => Frame::Close {
            code: error_code(code)?,
            message: message.clone(),
        },
        WireFrame::Welcome { .. }
        | WireFrame::Event { .. }
        | WireFrame::Lagged { .. }
        | WireFrame::Ack { .. }
        | WireFrame::Error { .. } => return None,
    })
}

/// Every stable error code, so `error_code` can be a lookup rather than a second hand-written
/// match that could disagree with `ErrorCode::as_str`.
pub const ERROR_CODES: [ErrorCode; 9] = [
    ErrorCode::UnsupportedVersion,
    ErrorCode::Unauthorized,
    ErrorCode::Forbidden,
    ErrorCode::Protocol,
    ErrorCode::RateLimited,
    ErrorCode::NotFound,
    ErrorCode::GoingAway,
    ErrorCode::Timeout,
    ErrorCode::Internal,
];

#[must_use]
pub fn error_code(word: &str) -> Option<ErrorCode> {
    ERROR_CODES.into_iter().find(|code| code.as_str() == word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::present::explain;

    #[test]
    fn only_the_five_product_surfaces_have_a_face() {
        assert_eq!(face(Surface::Marketing), Some(Face::Marketing));
        assert_eq!(face(Surface::User), Some(Face::User));
        assert_eq!(face(Surface::Org), Some(Face::Org));
        assert_eq!(face(Surface::App), Some(Face::App));
        assert_eq!(face(Surface::Mobile), Some(Face::Mobile));
        for refused in [
            Surface::Api,
            Surface::Auth,
            Surface::Admin,
            Surface::AdminApi,
        ] {
            assert_eq!(
                face(refused),
                None,
                "surface {refused} must not be rendered here"
            );
        }
    }

    /// One case per `OnboardingError` variant, each asserted to reach the message written for it.
    /// If lib-core grows a variant this test still compiles — but `refusal` does not, which is the
    /// guard that matters.
    #[test]
    fn every_onboarding_error_reaches_its_own_message() {
        let cases = [
            OnboardingError::NoSeatsAvailable {
                purchased: 3,
                occupied: 2,
                reserved: 1,
            },
            OnboardingError::DomainNotAllowed {
                domain: "contractor.example".into(),
            },
            OnboardingError::InvitationNotOpen,
            OnboardingError::InvitationAddressMismatch,
            OnboardingError::LastOwner,
            OnboardingError::RoleEscalation {
                actor: Role::Admin,
                granted: Role::Owner,
            },
            OnboardingError::InvalidEmail,
        ];
        let mut messages = Vec::new();
        for error in &cases {
            let message = explain(&refusal(error)).message;
            assert!(
                !messages.contains(&message),
                "two variants share a message: {message}"
            );
            messages.push(message);
        }
        assert_eq!(messages.len(), 7);

        // Spot-check that the domain data survives the crossing.
        let seats = explain(&refusal(&OnboardingError::NoSeatsAvailable {
            purchased: 3,
            occupied: 2,
            reserved: 1,
        }));
        assert!(seats.message.contains("All 3 seats"));
        let escalation = explain(&refusal(&OnboardingError::RoleEscalation {
            actor: Role::Member,
            granted: Role::Admin,
        }));
        assert_eq!(escalation.message, "A member cannot grant the admin role.");
    }

    #[test]
    fn every_frame_survives_the_crossing_to_json_and_back() {
        let samples = [
            Frame::Hello {
                version: 1,
                resume: Some("sess".into()),
            },
            Frame::Ping { nonce: 9 },
            Frame::Pong { nonce: 9 },
            Frame::Subscribe {
                stream: "run:01H:logs".into(),
                from: 3,
                credit: 64,
            },
            Frame::Unsubscribe {
                stream: "run:01H:logs".into(),
            },
            Frame::Credit {
                stream: "run:01H:logs".into(),
                additional: 32,
            },
            Frame::Command {
                id: 1,
                verb: "cancel".into(),
                payload: b"{}".to_vec(),
            },
            Frame::Close {
                code: ErrorCode::GoingAway,
                message: "deploy".into(),
            },
        ];
        for frame in samples {
            let text = crate::wire::encode(&to_wire(&frame));
            let decoded = crate::wire::decode(&text).expect("our own encoding decodes");
            assert_eq!(
                from_wire(&decoded),
                Some(frame.clone()),
                "frame {frame:?} via {text}"
            );
        }
    }

    #[test]
    fn a_client_cannot_send_a_server_only_frame() {
        for frame in [
            Frame::Welcome {
                session: "s".into(),
                resumed: false,
                heartbeat_seconds: 20,
            },
            Frame::Event {
                stream: "s".into(),
                sequence: 1,
                payload: b"x".to_vec(),
            },
            Frame::Lagged {
                stream: "s".into(),
                skipped_to: 9,
            },
            Frame::Ack {
                id: 1,
                payload: Vec::new(),
            },
            Frame::Error {
                id: None,
                code: ErrorCode::Internal,
                message: String::new(),
            },
        ] {
            let wire = to_wire(&frame);
            assert_eq!(
                from_wire(&wire),
                None,
                "frame {frame:?} must not be accepted from a client"
            );
        }
    }

    #[test]
    fn error_codes_round_trip_and_an_invented_one_does_not() {
        for code in ERROR_CODES {
            assert_eq!(error_code(code.as_str()), Some(code));
        }
        assert_eq!(error_code("teapot"), None);
        assert_eq!(error_code(""), None);
        // A `close` frame naming a code we do not speak is refused rather than coerced.
        let bogus = WireFrame::Close {
            code: "teapot".into(),
            message: String::new(),
        };
        assert_eq!(from_wire(&bogus), None);
    }
}
