//! WebSocket delivery: authenticate, catch up, then stream both ways.

use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use thrum_core::protocol::{ClientFrame, ServerFrame};
use tokio::sync::mpsc;

use crate::accounts::now;
use crate::auth::{self, AuthedDevice};
use crate::blobs;
use crate::frames::{parse_client_frame, render_server_frame};
use crate::messages;
use crate::state::AppState;

/// How long the server waits for the auth frame after connecting.
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);

/// `GET /v1/ws`: upgrade to a delivery WebSocket.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let authed = match authenticate_socket(&mut socket, &state).await {
        Some(authed) => authed,
        None => return, // socket dropped or auth failed; connection closes
    };

    // Register before catch-up so nothing is missed in between. A message
    // delivered live right in this window can also come from the queue, so
    // clients deduplicate by message id. The hub holds the only sender into
    // this connection's channel: when a newer connection of the device
    // replaces it, or the device is ended, the channel closes and so does
    // this socket.
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<ServerFrame>();
    let connection_id = state.hub.register(&authed.device_id, outbound_tx);

    let auth_ok = ServerFrame::AuthOk {
        device_id: authed.device_id.clone(),
    };
    if socket
        .send(WsMessage::text(render_server_frame(&auth_ok)))
        .await
        .is_err()
    {
        state.hub.remove(&authed.device_id, connection_id);
        return;
    }
    // Drop messages nobody can still fetch, then send what is left one
    // page at a time, so a long backlog is never held in memory at once.
    let keep_older_than = now() - state.cfg.queue_ttl_seconds;
    let _ = messages::cleanup_expired(&state.db, keep_older_than).await;
    let mut cursor = None;
    loop {
        let Ok((page, next)) =
            messages::undelivered_page(&state.db, state.backend, &authed.device_id, cursor).await
        else {
            break;
        };
        for frame in &page {
            if socket
                .send(WsMessage::text(render_server_frame(frame)))
                .await
                .is_err()
            {
                state.hub.remove(&authed.device_id, connection_id);
                return;
            }
        }
        if (page.len() as i64) < messages::CATCH_UP_PAGE {
            break;
        }
        cursor = next;
    }

    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            outbound = outbound_rx.recv() => match outbound {
                Some(frame) => {
                    if sink.send(WsMessage::text(render_server_frame(&frame))).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            inbound = stream.next() => match inbound {
                Some(Ok(WsMessage::Text(text))) => {
                    match handle_client_frame(&state, &authed, text.as_str()).await {
                        ClientAction::Hangup => break,
                        ClientAction::Continue { reply } => {
                            if let Some(frame) = reply {
                                let text = render_server_frame(&frame);
                                if sink.send(WsMessage::text(text)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {} // pings/pongs and binary frames are ignored
            },
        }
    }
    state.hub.remove(&authed.device_id, connection_id);
}

/// Expect the first frame to be a valid auth frame. Any other behavior
/// (wrong token, wrong frame, timeout, disconnect) closes the connection.
async fn authenticate_socket(socket: &mut WebSocket, state: &AppState) -> Option<AuthedDevice> {
    let first = tokio::time::timeout(AUTH_TIMEOUT, socket.recv())
        .await
        .ok()??;
    let WsMessage::Text(text) = first.ok()? else {
        return None;
    };
    let ClientFrame::Auth { token } = parse_client_frame(text.as_str())? else {
        return None;
    };
    auth::authenticate_token(state, &token).await.ok()
}

/// What to do after one client frame.
enum ClientAction {
    /// Close the connection.
    Hangup,
    /// Keep going, optionally replying with one frame.
    Continue { reply: Option<ServerFrame> },
}

/// Handle one client frame.
async fn handle_client_frame(state: &AppState, authed: &AuthedDevice, text: &str) -> ClientAction {
    let Some(frame) = parse_client_frame(text) else {
        return ClientAction::Hangup;
    };
    match frame {
        ClientFrame::Auth { .. } => ClientAction::Hangup, // already authenticated
        ClientFrame::Ack { message_id } => {
            // Database trouble is rare; the client will reconnect and resend.
            let reply = if messages::mark_delivered(&state.db, &message_id, &authed.device_id)
                .await
                .is_ok()
            {
                Some(ServerFrame::AckOk { message_id })
            } else {
                None
            };
            ClientAction::Continue { reply }
        }
    }
}

/// Sweep away expired undelivered messages and expired blobs once a minute.
pub fn spawn_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            // Clean up expired undelivered messages; one failed pass is not
            // fatal.
            let keep_older_than = now() - state.cfg.queue_ttl_seconds;
            let _ = messages::cleanup_expired(&state.db, keep_older_than).await;
            // Clean up expired blobs: rows and stored objects.
            blobs::sweep_expired(&state.db, state.blob_store.as_ref()).await;
        }
    });
}
