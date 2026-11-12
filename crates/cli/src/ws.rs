//! Receiving messages over the server's WebSocket.

use std::collections::HashSet;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use thrum_core::protocol::{ClientFrame, ServerFrame};
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

/// Connect to the server, authenticate, print one line per message, and
/// acknowledge each message so the server stops queueing it.
///
/// With `once`, the call returns after the server confirms the first
/// acknowledgement, which makes scripts and tests deterministic.
pub async fn recv(
    session: &Session,
    options: &RecvOptions,
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
    loop {
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
                        let text = display_text(&payload_base64);
                        emit(format!(
                            "message {conversation_id} {sender_username} {text}"
                        ));
                        send_frame(
                            &mut sink,
                            &ClientFrame::Ack {
                                message_id: message_id.clone(),
                            },
                        )
                        .await?;
                        if options.once {
                            awaiting_ack = Some(message_id);
                        }
                    }
                    ServerFrame::AckOk { message_id } => {
                        if options.once && awaiting_ack.as_deref() == Some(message_id.as_str()) {
                            return Ok(());
                        }
                    }
                }
            }
            Ok(Some(Ok(_))) => {} // binary, ping and pong frames
        }
    }
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

/// Decode a payload for display; binary payloads become a short tag.
fn display_text(payload_base64: &str) -> String {
    match thrum_core::protocol::decode_payload(payload_base64) {
        Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        None => format!("<undecodable {payload_base64}>"),
    }
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
}
