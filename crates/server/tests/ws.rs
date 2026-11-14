//! WebSocket tests: auth handshake, live delivery, acks, catch-up, TTL.

mod common;

use std::time::Duration;

use common::{TestApp, login, register};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use thrum_core::protocol::{ClientFrame, ServerFrame};
use tokio_tungstenite::tungstenite::Message as WsMessage;

const PW: &str = "long enough password";
/// Generous upper bound for any awaited server frame.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Short wait used to prove that *nothing* arrives.
const QUIET: Duration = Duration::from_millis(400);

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn ws_url(base: &str) -> String {
    format!("{}/v1/ws", base.replacen("http://", "ws://", 1))
}

/// Register and log in a user; returns the session object.
async fn user(app: &TestApp, username: &str) -> Value {
    register(app, username, PW).await;
    login(app, username, PW, "laptop").await
}

/// Connect and authenticate; returns after consuming the AuthOk frame.
async fn connect_authed(base: &str, token: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(base))
        .await
        .expect("connect");
    send_frame(
        &mut ws,
        &ClientFrame::Auth {
            token: token.to_string(),
        },
    )
    .await;
    match next_frame(&mut ws).await {
        Some(ServerFrame::AuthOk { device_id }) => {
            assert!(!device_id.is_empty(), "device id present");
        }
        other => panic!("expected auth_ok, got {other:?}"),
    }
    ws
}

async fn send_frame(ws: &mut Ws, frame: &ClientFrame) {
    let text = serde_json::to_string(frame).expect("serialize frame");
    ws.send(WsMessage::text(text)).await.expect("send frame");
}

/// Next parsed server frame, or None when the server closes the connection.
async fn next_frame(ws: &mut Ws) -> Option<ServerFrame> {
    loop {
        let message = tokio::time::timeout(TIMEOUT, ws.next())
            .await
            .ok()?? // timeout or stream end
            .ok()?; // transport error
        match message {
            WsMessage::Text(text) => {
                if let Ok(frame) = serde_json::from_str::<ServerFrame>(&text) {
                    return Some(frame);
                }
                return None;
            }
            WsMessage::Close(_) => return None,
            _ => continue, // pings, pongs, binary
        }
    }
}

/// Create a conversation between alice and bob; returns its id.
async fn conversation_id(app: &TestApp, alice_token: &str) -> String {
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice_token)
        .json(&json!({ "members": ["bob"] }))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 201, "create conversation");
    let body: Value = response.json().await.expect("body");
    body["id"].as_str().expect("id").to_string()
}

/// Send a payload from alice to the conversation.
async fn send(app: &TestApp, alice_token: &str, conversation: &str, payload: &[u8]) -> String {
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{conversation}/messages")))
        .bearer_auth(alice_token)
        .body(payload.to_vec())
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 201, "send message");
    let body: Value = response.json().await.expect("body");
    body["id"].as_str().expect("id").to_string()
}

#[tokio::test]
async fn bad_auth_closes_the_connection() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;

    // Wrong token.
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&app.base))
        .await
        .expect("connect");
    send_frame(
        &mut ws,
        &ClientFrame::Auth {
            token: "not-a-token".to_string(),
        },
    )
    .await;
    assert!(
        next_frame(&mut ws).await.is_none(),
        "server must close on a bad token"
    );

    // First frame is not an auth frame at all.
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&app.base))
        .await
        .expect("connect");
    send_frame(
        &mut ws,
        &ClientFrame::Ack {
            message_id: "whatever".to_string(),
        },
    )
    .await;
    assert!(
        next_frame(&mut ws).await.is_none(),
        "server must close when the first frame is not auth"
    );

    // A valid token works.
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&app.base))
        .await
        .expect("connect");
    send_frame(
        &mut ws,
        &ClientFrame::Auth {
            token: alice["token"].as_str().expect("token").to_string(),
        },
    )
    .await;
    assert!(matches!(
        next_frame(&mut ws).await,
        Some(ServerFrame::AuthOk { .. })
    ));
}

#[tokio::test]
async fn live_delivery_and_ack_stop_queueing() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conversation = conversation_id(&app, alice["token"].as_str().expect("token")).await;
    let mut bob_ws = connect_authed(&app.base, bob["token"].as_str().expect("token")).await;

    let message_id = send(
        &app,
        alice["token"].as_str().expect("token"),
        &conversation,
        b"hello bob",
    )
    .await;
    match next_frame(&mut bob_ws).await {
        Some(ServerFrame::Message {
            conversation_id: got_conversation,
            message_id: got_message,
            sender_username,
            payload_base64,
            ..
        }) => {
            assert_eq!(got_conversation, conversation);
            assert_eq!(got_message, message_id);
            assert_eq!(sender_username, "alice");
            assert_eq!(decode(&payload_base64), b"hello bob");
        }
        other => panic!("expected a live message, got {other:?}"),
    }

    // Acknowledge; the server confirms and stops queueing.
    send_frame(
        &mut bob_ws,
        &ClientFrame::Ack {
            message_id: message_id.clone(),
        },
    )
    .await;
    match next_frame(&mut bob_ws).await {
        Some(ServerFrame::AckOk { message_id: acked }) => assert_eq!(acked, message_id),
        other => panic!("expected ack_ok, got {other:?}"),
    }
    drop(bob_ws);

    // A fresh connection gets nothing: the message was delivered.
    let mut bob_ws = connect_authed(&app.base, bob["token"].as_str().expect("token")).await;
    assert!(
        tokio::time::timeout(QUIET, next_frame(&mut bob_ws))
            .await
            .is_err(),
        "acked messages must not be replayed"
    );
}

#[tokio::test]
async fn catch_up_replays_undelivered_messages() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conversation = conversation_id(&app, alice["token"].as_str().expect("token")).await;

    let first = send(
        &app,
        alice["token"].as_str().expect("token"),
        &conversation,
        b"one",
    )
    .await;
    let second = send(
        &app,
        alice["token"].as_str().expect("token"),
        &conversation,
        b"two",
    )
    .await;

    // Bob comes online and receives everything he missed.
    let mut bob_ws = connect_authed(&app.base, bob["token"].as_str().expect("token")).await;
    let mut got = Vec::new();
    for _ in 0..2 {
        match next_frame(&mut bob_ws).await {
            Some(ServerFrame::Message {
                message_id,
                payload_base64,
                ..
            }) => {
                got.push((message_id, decode(&payload_base64)));
            }
            other => panic!("expected a queued message, got {other:?}"),
        }
    }
    let mut expected = vec![
        (first.clone(), b"one".to_vec()),
        (second.clone(), b"two".to_vec()),
    ];
    expected.sort();
    got.sort();
    assert_eq!(got, expected, "both queued messages are replayed");

    // Acks clear the queue.
    for (message_id, _) in &got {
        send_frame(
            &mut bob_ws,
            &ClientFrame::Ack {
                message_id: message_id.clone(),
            },
        )
        .await;
        assert!(matches!(
            next_frame(&mut bob_ws).await,
            Some(ServerFrame::AckOk { .. })
        ));
    }
    drop(bob_ws);

    let mut bob_ws = connect_authed(&app.base, bob["token"].as_str().expect("token")).await;
    assert!(
        tokio::time::timeout(QUIET, next_frame(&mut bob_ws))
            .await
            .is_err(),
        "nothing may be left in the queue"
    );
}

#[tokio::test]
async fn expired_undelivered_messages_are_dropped() {
    let cfg = thrum_server::config::Config {
        queue_ttl_seconds: 1,
        ..Default::default()
    };
    let app = TestApp::spawn_with(cfg).await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conversation = conversation_id(&app, alice["token"].as_str().expect("token")).await;

    send(
        &app,
        alice["token"].as_str().expect("token"),
        &conversation,
        b"gone soon",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(2_000)).await;

    // The message outlived its TTL: the connection replays nothing.
    let mut bob_ws = connect_authed(&app.base, bob["token"].as_str().expect("token")).await;
    assert!(
        tokio::time::timeout(QUIET, next_frame(&mut bob_ws))
            .await
            .is_err(),
        "expired messages must be dropped, not delivered"
    );
}

#[tokio::test]
async fn sender_is_not_delivered_own_message() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conversation = conversation_id(&app, alice["token"].as_str().expect("token")).await;
    let mut alice_ws = connect_authed(&app.base, alice["token"].as_str().expect("token")).await;

    send(
        &app,
        alice["token"].as_str().expect("token"),
        &conversation,
        b"from me",
    )
    .await;
    assert!(
        tokio::time::timeout(QUIET, next_frame(&mut alice_ws))
            .await
            .is_err(),
        "the sender's own devices must not receive the message"
    );

    // But bob, connected live, does.
    let mut bob_ws = connect_authed(&app.base, bob["token"].as_str().expect("token")).await;
    tokio::time::sleep(QUIET).await; // ensure bob registered before the send below
    send(
        &app,
        alice["token"].as_str().expect("token"),
        &conversation,
        b"to bob",
    )
    .await;
    assert!(matches!(
        next_frame(&mut bob_ws).await,
        Some(ServerFrame::Message { .. })
    ));
}

/// Decode a base64 payload back to bytes; panics on malformed input.
fn decode(encoded: &str) -> Vec<u8> {
    thrum_core::protocol::decode_payload(encoded).expect("valid base64")
}
