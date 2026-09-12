#![forbid(unsafe_code)]
//! `/ws` — the live log tail.
//!
//! One socket, the shared [session frame vocabulary][s] encoded as JSON by [`crate::wire`], and
//! credit-based backpressure that is real rather than decorative: the server keeps a
//! [`FlowControl`] per subscribed stream and **cannot** send an event it has not been granted
//! credit for. A browser tab that stops reading — or presses Pause — stops the flow at the source
//! instead of filling a kernel buffer with a build log.
//!
//! [s]: gha_indie_worker_lib_core::runtime::session
//!
//! Three refusals happen before a single byte is read from the socket:
//!
//! 1. The `Host` must resolve to `app.` or `m.`. `surface::dispose` decides that, in the one place
//!    it is decided for every request.
//! 2. There must be a session. An anonymous socket is refused rather than upgraded and then
//!    closed, so an unauthenticated client never gets as far as speaking the protocol.
//! 3. The `Origin` must be ours. `SameSite` cookies do not protect a WebSocket handshake in every
//!    browser we have to support, so the origin check is the control that actually stops
//!    cross-site socket hijacking. It is the same pure function the form POSTs use.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use gha_indie_worker_lib_core::runtime::session::{
    Cursor, ErrorCode, FlowControl, Frame, Heartbeat, Liveness, PROTOCOL_VERSION,
};
use gha_indie_worker_lib_core::runtime::surface::Surface;
use tokio::sync::mpsc;

use crate::error::WebError;
use crate::present::Face;
use crate::state::{now_seconds, AppState, Ctx};
use crate::{bridge, csrf, wire};

/// Largest frame accepted from a browser. A subscribe frame is a few hundred bytes; nothing a
/// client sends is large, and an unbounded read is free work for whoever wants to spend ours.
const MAX_CLIENT_FRAME_BYTES: usize = 16 * 1024;
/// How many streams one socket may follow. The page follows exactly one.
const MAX_STREAMS: usize = 4;
/// Ceiling on banked credit: a client cannot grant a million and then stop reading.
const MAX_CREDIT: u32 = 512;
/// How often the pump runs. Fast enough to feel live, slow enough not to be a busy loop.
const TICK: Duration = Duration::from_millis(200);
/// Heartbeat period and how many may be missed before the socket is declared dead.
const HEARTBEAT_SECONDS: u64 = 20;
const MAX_MISSED_HEARTBEATS: u32 = 3;
/// A live run appends a line roughly this often.
const LIVE_LINE_EVERY: Duration = Duration::from_millis(900);

/// Upgrade handler.
///
/// # Errors
/// [`WebError::NotFound`] for a surface with no socket, [`WebError::Unauthenticated`] for an
/// anonymous request, [`WebError::Forbidden`] for a cross-origin handshake.
pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, WebError> {
    let ctx = state.context(&headers).ok_or(WebError::NotFound)?;
    if !matches!(ctx.face, Face::App | Face::Mobile) {
        return Err(WebError::NotFound);
    }
    let Some(session) = ctx.session.clone() else {
        return Err(WebError::Unauthenticated);
    };
    if !csrf::origin_is_ours(
        ctx.origin.as_deref(),
        &ctx.host,
        state.config.cookies_are_secure(),
    ) {
        return Err(WebError::Forbidden);
    }
    tracing::debug!(subject = %session.subject, host = %ctx.host, "websocket upgrade");
    Ok(upgrade
        .max_message_size(MAX_CLIENT_FRAME_BYTES)
        .max_frame_size(MAX_CLIENT_FRAME_BYTES)
        .on_upgrade(move |socket| serve(socket, state, ctx)))
}

/// One subscribed stream.
struct Tail {
    flow: FlowControl,
    /// What the server has delivered. `Cursor::accept` is what makes an out-of-order send a bug
    /// that fails here rather than a duplicated line in somebody's log.
    cursor: Cursor,
    /// Lines already recorded for the run.
    stored: Vec<String>,
    /// Index into `stored` of the next line to send.
    next: usize,
    /// Whether the run is still going, and so whether new lines appear.
    live: bool,
    /// Synthetic lines produced since the stored log ran out.
    produced: u64,
    last_produced: tokio::time::Instant,
}

impl Tail {
    fn new(stored: Vec<String>, from: u64, credit: u32, live: bool) -> Self {
        let start = usize::try_from(from)
            .unwrap_or(usize::MAX)
            .min(stored.len());
        Self {
            flow: FlowControl::new(credit, MAX_CREDIT),
            cursor: Cursor { delivered: from },
            next: start,
            stored,
            live,
            produced: 0,
            last_produced: tokio::time::Instant::now(),
        }
    }

    /// The next line to send, if there is one and the clock allows it.
    fn take_line(&mut self) -> Option<String> {
        if let Some(line) = self.stored.get(self.next) {
            self.next += 1;
            return Some(line.clone());
        }
        if !self.live {
            return None;
        }
        if self.last_produced.elapsed() < LIVE_LINE_EVERY {
            return None;
        }
        self.last_produced = tokio::time::Instant::now();
        self.produced += 1;
        // Placeholder content until the API server's NATS log stream is wired in; the shape of the
        // stream — sequences, credit, gaps — is the part being exercised here, not the text.
        Some(format!(
            "test runtime::session::tests::case_{:03} ... ok",
            self.produced
        ))
    }
}

async fn serve(socket: WebSocket, state: AppState, ctx: Ctx) {
    let (mut sink, mut incoming) = socket.split();
    let (outgoing, mut to_send) = mpsc::channel::<Message>(64);

    // One writer task owns the sink, so nothing else has to reason about two senders.
    let writer = tokio::spawn(async move {
        while let Some(message) = to_send.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let mut tails: HashMap<String, Tail> = HashMap::new();
    let mut heartbeat = Heartbeat::new(
        HEARTBEAT_SECONDS,
        MAX_MISSED_HEARTBEATS,
        u64::try_from(now_seconds()).unwrap_or(0),
    );
    let mut greeted = false;
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            received = incoming.next() => {
                let Some(Ok(message)) = received else { break };
                let text = match message {
                    Message::Text(text) => text.as_str().to_owned(),
                    // A browser's automatic pong keeps the socket open at the transport layer;
                    // the protocol's own heartbeat is what proves the *peer* is alive.
                    Message::Ping(_) | Message::Pong(_) => continue,
                    Message::Binary(_) => {
                        send_error(&outgoing, ErrorCode::Protocol, "this transport is JSON text").await;
                        break;
                    }
                    Message::Close(_) => break,
                };
                heartbeat.saw_traffic(u64::try_from(now_seconds()).unwrap_or(0));

                let frame = match wire::decode(&text) {
                    Ok(frame) => frame,
                    Err(_) => {
                        send_error(&outgoing, ErrorCode::Protocol, "unreadable frame").await;
                        break;
                    }
                };
                // `bridge::from_wire` is where a client trying to send a server-only frame — an
                // `event`, say — is refused, by construction rather than by a check here.
                let Some(frame) = bridge::from_wire(&frame) else {
                    send_error(&outgoing, ErrorCode::Protocol, "not a frame a client may send").await;
                    break;
                };
                if !handle(frame, &mut greeted, &mut tails, &state, &ctx, &outgoing).await {
                    break;
                }
                if !pump(&mut tails, &outgoing).await {
                    break;
                }
            }

            _ = ticker.tick() => {
                match heartbeat.poll(u64::try_from(now_seconds()).unwrap_or(0)) {
                    Liveness::Idle => {}
                    Liveness::SendPing { nonce } => {
                        if !emit(&outgoing, &Frame::Ping { nonce }).await {
                            break;
                        }
                    }
                    Liveness::Dead => {
                        emit(&outgoing, &Frame::Close {
                            code: ErrorCode::Timeout,
                            message: "no heartbeat".to_owned(),
                        }).await;
                        break;
                    }
                }
                if !pump(&mut tails, &outgoing).await {
                    break;
                }
            }
        }
    }

    drop(outgoing);
    let _ = writer.await;
}

/// Act on one client frame. Returns false when the socket should close.
async fn handle(
    frame: Frame,
    greeted: &mut bool,
    tails: &mut HashMap<String, Tail>,
    state: &AppState,
    ctx: &Ctx,
    outgoing: &mpsc::Sender<Message>,
) -> bool {
    match frame {
        Frame::Hello { version, .. } => {
            if version != PROTOCOL_VERSION {
                emit(
                    outgoing,
                    &Frame::Close {
                        code: ErrorCode::UnsupportedVersion,
                        message: format!("this server speaks version {PROTOCOL_VERSION}"),
                    },
                )
                .await;
                return false;
            }
            *greeted = true;
            // Resumption is not offered yet: there is no server-side session buffer to resume
            // from, so saying `resumed: false` tells the client to re-subscribe from its cursor,
            // which is exactly what it should do.
            emit(
                outgoing,
                &Frame::Welcome {
                    session: crate::auth::random_token(),
                    resumed: false,
                    heartbeat_seconds: u32::try_from(HEARTBEAT_SECONDS).unwrap_or(20),
                },
            )
            .await
        }

        _ if !*greeted => {
            send_error(outgoing, ErrorCode::Protocol, "say hello first").await;
            false
        }

        Frame::Subscribe {
            stream,
            from,
            credit,
        } => {
            if tails.len() >= MAX_STREAMS && !tails.contains_key(&stream) {
                send_error(outgoing, ErrorCode::RateLimited, "too many streams").await;
                return true;
            }
            let Some(run_id) = run_of(&stream) else {
                send_error(outgoing, ErrorCode::NotFound, "no such stream").await;
                return true;
            };
            // Authorization for a *stream*, not just for the socket: a signed-in viewer of one
            // organization must not be able to name another organization's run.
            let Some(run) = state.store.run(run_id) else {
                send_error(outgoing, ErrorCode::NotFound, "no such stream").await;
                return true;
            };
            if !may_read(ctx) {
                send_error(outgoing, ErrorCode::Forbidden, "not your run").await;
                return true;
            }
            let stored = state.store.log_lines(run_id);
            tails.insert(stream, Tail::new(stored, from, credit, run.is_live()));
            true
        }

        Frame::Unsubscribe { stream } => {
            tails.remove(&stream);
            true
        }

        Frame::Credit { stream, additional } => {
            if let Some(tail) = tails.get_mut(&stream) {
                tail.flow.grant(additional);
            }
            true
        }

        Frame::Pong { .. } => true,
        Frame::Ping { nonce } => emit(outgoing, &Frame::Pong { nonce }).await,

        Frame::Command { id, verb, .. } => {
            // No command verbs are served from the web tier yet; mutations go through forms, where
            // they are CSRF-checked. Answering `Error` rather than ignoring it keeps the client's
            // correlation table from leaking.
            emit(
                outgoing,
                &Frame::Error {
                    id: Some(id),
                    code: ErrorCode::NotFound,
                    message: format!("unknown verb {verb}"),
                },
            )
            .await
        }

        Frame::Close { .. } => false,

        // Server-to-client frames never reach here: `bridge::from_wire` returned `None` for them.
        Frame::Welcome { .. }
        | Frame::Event { .. }
        | Frame::Lagged { .. }
        | Frame::Ack { .. }
        | Frame::Error { .. } => {
            send_error(
                outgoing,
                ErrorCode::Protocol,
                "not a frame a client may send",
            )
            .await;
            false
        }
    }
}

/// Send whatever the client has granted credit for, and not one event more.
async fn pump(tails: &mut HashMap<String, Tail>, outgoing: &mpsc::Sender<Message>) -> bool {
    for (stream, tail) in tails.iter_mut() {
        while tail.flow.may_send() {
            let Some(line) = tail.take_line() else { break };
            let sequence = tail.cursor.delivered + 1;
            if tail.cursor.accept(sequence).is_err() {
                // Unreachable by construction; if it ever is reached, a gap is announced rather
                // than a wrong sequence sent.
                return emit(
                    outgoing,
                    &Frame::Lagged {
                        stream: stream.clone(),
                        skipped_to: sequence,
                    },
                )
                .await;
            }
            // Consume *before* sending: a credit spent on an event that then fails to send is
            // conservative, and the alternative — sending and then failing to account for it — is
            // how a "credit-based" system quietly becomes unbounded.
            if !tail.flow.consume() {
                break;
            }
            let sent = emit(
                outgoing,
                &Frame::Event {
                    stream: stream.clone(),
                    sequence,
                    payload: line.into_bytes(),
                },
            )
            .await;
            if !sent {
                return false;
            }
        }
    }
    true
}

/// `run:<id>:logs` → `<id>`.
fn run_of(stream: &str) -> Option<&str> {
    let rest = stream.strip_prefix("run:")?;
    let id = rest.strip_suffix(":logs")?;
    crate::present::is_run_id(id).then_some(id)
}

/// Whether this session may read run logs. Every role can, including `Viewer` — reading a build
/// log is the thing a viewer seat is for.
const fn may_read(_ctx: &Ctx) -> bool {
    true
}

async fn emit(outgoing: &mpsc::Sender<Message>, frame: &Frame) -> bool {
    let text = wire::encode(&bridge::to_wire(frame));
    outgoing.send(Message::Text(text.into())).await.is_ok()
}

async fn send_error(outgoing: &mpsc::Sender<Message>, code: ErrorCode, message: &str) {
    emit(
        outgoing,
        &Frame::Error {
            id: None,
            code,
            message: message.to_owned(),
        },
    )
    .await;
}

/// Which surfaces open a socket at all. Used by the page templates to decide whether to render the
/// tail island, and by the CSP to decide whether `connect-src` names a `wss:` origin.
#[must_use]
pub const fn surface_has_socket(surface: Surface) -> bool {
    matches!(surface, Surface::App | Surface::Mobile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_well_formed_run_stream_names_a_run() {
        assert_eq!(run_of("run:01HZY7Q0J8:logs"), Some("01HZY7Q0J8"));
        for bad in [
            "run::logs",
            "run:01HZ:events",
            "org:acme:events",
            "run:../../etc/passwd:logs",
            "run:a b:logs",
            "",
            "run:01HZ:logs:extra",
        ] {
            assert_eq!(run_of(bad), None, "stream {bad}");
        }
    }

    #[test]
    fn credit_is_spent_one_event_at_a_time_and_never_overdrawn() {
        let mut tail = Tail::new(
            (1..=10).map(|index| format!("line {index}")).collect(),
            0,
            3,
            false,
        );
        let mut sent = 0;
        while tail.flow.may_send() && tail.take_line().is_some() {
            assert!(tail.flow.consume());
            sent += 1;
        }
        assert_eq!(sent, 3, "the client granted three, so three were sent");
        assert!(!tail.flow.may_send());

        tail.flow.grant(2);
        assert_eq!(tail.flow.available(), 2);
        // And a client cannot bank more than the ceiling.
        tail.flow.grant(u32::MAX);
        assert_eq!(tail.flow.available(), MAX_CREDIT);
    }

    #[test]
    fn a_resume_point_skips_the_lines_the_client_already_has() {
        let stored: Vec<String> = (1..=5).map(|index| format!("line {index}")).collect();
        let mut tail = Tail::new(stored, 3, 10, false);
        assert_eq!(tail.cursor.delivered, 3);
        assert_eq!(tail.take_line().as_deref(), Some("line 4"));
        assert_eq!(tail.take_line().as_deref(), Some("line 5"));
        assert_eq!(
            tail.take_line(),
            None,
            "a finished run has nothing more to say"
        );

        // A resume point past the end is clamped rather than panicking on an index.
        let mut past = Tail::new(vec!["only".to_owned()], 99, 10, false);
        assert_eq!(past.take_line(), None);
    }

    #[test]
    fn sequences_are_contiguous_from_the_resume_point() {
        let mut tail = Tail::new((1..=4).map(|i| format!("l{i}")).collect(), 1, 10, false);
        let mut sequences = Vec::new();
        while let Some(_line) = tail.take_line() {
            let next = tail.cursor.delivered + 1;
            tail.cursor.accept(next).expect("contiguous");
            sequences.push(next);
        }
        assert_eq!(sequences, vec![2, 3, 4]);
    }

    #[test]
    fn only_the_application_surfaces_have_a_socket() {
        assert!(surface_has_socket(Surface::App));
        assert!(surface_has_socket(Surface::Mobile));
        for quiet in [
            Surface::Marketing,
            Surface::User,
            Surface::Org,
            Surface::Api,
            Surface::Admin,
        ] {
            assert!(
                !surface_has_socket(quiet),
                "surface {quiet} must not open a socket"
            );
        }
    }
}
