//! Receiving messages over the server's WebSocket.
//!
//! This module only transports relay frames; interpreting the opaque
//! payloads (MLS processing) is the caller's job via the `interpret`
//! callback. A message is acknowledged, so the server stops queueing it,
//! only once `interpret` returns `Ok`. An `Err` means the message could not
//! be handled yet (the network failed, say): it stays unacknowledged, and
//! the connection stops there so later messages are not applied out of
//! order. The server offers it again on the next connection.

use std::collections::HashSet;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use thrum_core::protocol::{ClientFrame, ServerFrame, decode_payload};
use tokio_tungstenite::tungstenite::Message;

use crate::session::Session;

/// How `recv` behaves.
#[derive(Debug, Clone, Copy)]
pub struct RecvOptions {
    /// Stop after the first delivered message.
    pub once: bool,
    /// Give up after this many seconds without traffic.
    pub timeout_secs: u64,
}

/// Connect to the server, authenticate, interpret one payload at a time,
/// print what the interpreter returns, and acknowledge each message it
/// handled. Returns the interpreter's error, with the message left
/// queued, when it could not handle one.
///
/// With `once`, the call returns after the server confirms the first
/// acknowledgement, which makes scripts and tests deterministic.
pub async fn recv(
    session: &Session,
    options: &RecvOptions,
    interpret: impl AsyncFnMut(&str, &str, &str, &[u8]) -> Result<Option<String>, String>,
    emit: impl FnMut(String),
) -> Result<(), String> {
    run(session, options, false, interpret, emit).await
}

/// How long [`drain`] waits for the server to finish sending the queue.
const DRAIN_TIMEOUT_SECS: u64 = 10;

/// Apply everything queued for this device, then return: once the server
/// says the queue is sent and has confirmed every acknowledgement. Use this
/// before anything that needs a current group state.
pub async fn drain(
    session: &Session,
    interpret: impl AsyncFnMut(&str, &str, &str, &[u8]) -> Result<Option<String>, String>,
    emit: impl FnMut(String),
) -> Result<(), String> {
    let options = RecvOptions {
        once: false,
        timeout_secs: DRAIN_TIMEOUT_SECS,
    };
    run(session, &options, true, interpret, emit).await
}

async fn run(
    session: &Session,
    options: &RecvOptions,
    until_caught_up: bool,
    mut interpret: impl AsyncFnMut(&str, &str, &str, &[u8]) -> Result<Option<String>, String>,
    mut emit: impl FnMut(String),
) -> Result<(), String> {
    let url = ws_url(&session.server)?;
    let stream = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|error| format!("websocket connect failed: {error}"))?
        .0;
    let (mut sink, mut source) = stream.split();

    let auth = ClientFrame::Auth {
        token: session.token.clone(),
    };
    send_frame(&mut sink, &auth).await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(options.timeout_secs);
    let mut seen = HashSet::new();
    let mut awaiting_ack: Option<String> = None;
    let mut unconfirmed = HashSet::new();
    let mut caught_up = false;
    loop {
        if until_caught_up && caught_up && unconfirmed.is_empty() {
            return Ok(());
        }
        let incoming = tokio::time::timeout_at(deadline, source.next()).await;
        match incoming {
            Err(_elapsed) => return Ok(()), // quiet for the whole timeout
            Ok(None) => return Ok(()),      // server closed the connection
            Ok(Some(Err(error))) => return Err(format!("websocket error: {error}")),
            Ok(Some(Ok(Message::Text(text)))) => {
                let Ok(frame) = serde_json::from_str::<ServerFrame>(text.as_str()) else {
                    continue; // ignore frames we do not understand
                };
                match frame {
                    ServerFrame::AuthOk { .. } => {}
                    ServerFrame::Message {
                        conversation_id,
                        message_id,
                        sender_username,
                        payload_base64,
                        ..
                    } => {
                        if !seen.insert(message_id.clone()) {
                            continue; // live and queued copies can overlap
                        }
                        let line = interpret_payload(
                            &mut interpret,
                            &conversation_id,
                            &message_id,
                            &sender_username,
                            &payload_base64,
                        )
                        .await
                        .map_err(|error| format!("{error}; the message stays queued"))?;
                        if let Some(line) = line {
                            emit(line);
                        }
                        send_frame(
                            &mut sink,
                            &ClientFrame::Ack {
                                message_id: message_id.clone(),
                            },
                        )
                        .await?;
                        if until_caught_up {
                            unconfirmed.insert(message_id.clone());
                        }
                        if options.once {
                            awaiting_ack = Some(message_id);
                        }
                    }
                    ServerFrame::AckOk { message_id } => {
                        unconfirmed.remove(&message_id);
                        if options.once && awaiting_ack.as_deref() == Some(message_id.as_str()) {
                            return Ok(());
                        }
                    }
                    ServerFrame::CaughtUp => caught_up = true,
                }
            }
            Ok(Some(Ok(_))) => {} // binary, ping and pong frames
        }
    }
}

/// Decode and interpret one base64 payload. A payload that is not valid
/// base64 can never be handled, so it counts as handled with nothing to show.
async fn interpret_payload(
    interpret: &mut impl AsyncFnMut(&str, &str, &str, &[u8]) -> Result<Option<String>, String>,
    conversation_id: &str,
    message_id: &str,
    sender_username: &str,
    payload_base64: &str,
) -> Result<Option<String>, String> {
    let Some(payload) = decode_payload(payload_base64) else {
        return Ok(None);
    };
    interpret(conversation_id, message_id, sender_username, &payload).await
}

async fn send_frame(
    sink: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    frame: &ClientFrame,
) -> Result<(), String> {
    let text = serde_json::to_string(frame).expect("client frames serialize");
    sink.send(Message::text(text))
        .await
        .map_err(|error| format!("websocket send failed: {error}"))
}

/// Turn the server base URL into its WebSocket URL.
fn ws_url(base: &str) -> Result<String, String> {
    let mut url = url::Url::parse(base).map_err(|error| format!("bad server url: {error}"))?;
    let ws_scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => return Err(format!("unsupported scheme {other}")),
    };
    url.set_scheme(ws_scheme)
        .map_err(|_| "cannot switch scheme".to_string())?;
    url.set_path("/v1/ws");
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_matches_http_scheme() {
        assert_eq!(
            ws_url("http://127.0.0.1:8080").expect("valid"),
            "ws://127.0.0.1:8080/v1/ws"
        );
        assert_eq!(
            ws_url("https://example.com").expect("valid"),
            "wss://example.com/v1/ws"
        );
        assert!(ws_url("ftp://example.com").is_err());
    }

    #[tokio::test]
    async fn interpret_payload_decodes_base64() {
        let encoded = thrum_core::protocol::encode_payload(b"plain");
        let line = interpret_payload(
            &mut async |_conversation: &str, _message_id: &str, _sender: &str, payload: &[u8]| {
                Ok(Some(String::from_utf8_lossy(payload).into_owned()))
            },
            "conv",
            "m1",
            "alice",
            &encoded,
        )
        .await;
        assert_eq!(line, Ok(Some("plain".to_string())));
    }
}
