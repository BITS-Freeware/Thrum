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

/// Next parsed server frame, skipping the `caught_up` marker that ends the
/// queue, or None when the server closes the connection.
async fn next_frame(ws: &mut Ws) -> Option<ServerFrame> {
    loop {
        match next_frame_or_marker(ws).await {
            Some(ServerFrame::CaughtUp) => continue,
            other => return other,
        }
    }
}

/// Wait for the marker that ends an empty queue, so what follows is live.
/// Sending before it can deliver a message twice, live and from the queue.
async fn caught_up(ws: &mut Ws) {
    assert_eq!(next_frame_or_marker(ws).await, Some(ServerFrame::CaughtUp));
}

/// Next parsed server frame, the `caught_up` marker included.
async fn next_frame_or_marker(ws: &mut Ws) -> Option<ServerFrame> {
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
    caught_up(&mut bob_ws).await;

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
async fn the_queue_ends_with_a_caught_up_marker() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let alice_token = alice["token"].as_str().expect("token");
    let bob_token = bob["token"].as_str().expect("token");

    // Nothing queued: the marker follows authentication at once.
    let mut bob_ws = connect_authed(&app.base, bob_token).await;
    assert_eq!(
        next_frame_or_marker(&mut bob_ws).await,
        Some(ServerFrame::CaughtUp)
    );
    drop(bob_ws);

    // Queued messages come first, then the marker, then live traffic.
    let conversation = conversation_id(&app, alice_token).await;
    let first = send(&app, alice_token, &conversation, b"one").await;
    let mut bob_ws = connect_authed(&app.base, bob_token).await;
    match next_frame_or_marker(&mut bob_ws).await {
        Some(ServerFrame::Message { message_id, .. }) => assert_eq!(message_id, first),
        other => panic!("expected the queued message, got {other:?}"),
    }
    assert_eq!(
        next_frame_or_marker(&mut bob_ws).await,
        Some(ServerFrame::CaughtUp)
    );
    let live = send(&app, alice_token, &conversation, b"two").await;
    match next_frame_or_marker(&mut bob_ws).await {
        Some(ServerFrame::Message { message_id, .. }) => assert_eq!(message_id, live),
        other => panic!("expected the live message, got {other:?}"),
    }
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
    // "from me" is queued for bob; once the marker follows it, he is live.
    match next_frame_or_marker(&mut bob_ws).await {
        Some(ServerFrame::Message { .. }) => {}
        other => panic!("expected the queued message, got {other:?}"),
    }
    caught_up(&mut bob_ws).await;
    send(
        &app,
        alice["token"].as_str().expect("token"),
        &conversation,
        b"to bob",
    )
    .await;
    match next_frame(&mut bob_ws).await {
        Some(ServerFrame::Message { payload_base64, .. }) => {
            assert_eq!(decode(&payload_base64), b"to bob");
        }
        other => panic!("expected the live message, got {other:?}"),
    }
}

/// Decode a base64 payload back to bytes; panics on malformed input.
fn decode(encoded: &str) -> Vec<u8> {
    thrum_core::protocol::decode_payload(encoded).expect("valid base64")
}

/// True when the server closes the socket within `wait`; false when it is
/// still open (nothing arrives) or a frame other than `caught_up` arrives.
async fn closed_within(ws: &mut Ws, wait: Duration) -> bool {
    loop {
        match tokio::time::timeout(wait, ws.next()).await {
            Err(_) => return false,
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(WsMessage::Close(_)))) => return true,
            Ok(Some(Ok(WsMessage::Text(text))))
                if serde_json::from_str::<ServerFrame>(&text).ok()
                    == Some(ServerFrame::CaughtUp) =>
            {
                continue;
            }
            Ok(Some(Ok(WsMessage::Text(_)))) => return false,
            Ok(Some(Ok(_))) => continue,
        }
    }
}

/// A second socket for one device replaces the first: the server closes
/// the first, and the second keeps getting live frames even after the
/// first's close is processed.
#[tokio::test]
async fn a_second_socket_replaces_the_first_and_stays_live() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conv = conversation_id(&app, alice["token"].as_str().unwrap()).await;
    let token = bob["token"].as_str().unwrap();
    let mut first = connect_authed(&app.base, token).await;
    let mut second = connect_authed(&app.base, token).await;
    caught_up(&mut second).await;
    assert!(
        closed_within(&mut first, TIMEOUT).await,
        "the replaced socket stays open"
    );
    let _ = first.close(None).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let id = send(&app, alice["token"].as_str().unwrap(), &conv, b"live").await;
    match next_frame(&mut second).await {
        Some(ServerFrame::Message { message_id, .. }) => assert_eq!(message_id, id),
        other => panic!("the live socket got {other:?}"),
    }
}

/// Revoking a device, logging it out, or deleting its account closes its
/// open socket.
#[tokio::test]
async fn ending_a_device_closes_its_socket() {
    let app = TestApp::spawn().await;
    let bob = user(&app, "bob").await;
    let phone = login(&app, "bob", PW, "phone").await;
    let tablet = login(&app, "bob", PW, "tablet").await;

    let mut phone_ws = connect_authed(&app.base, phone["token"].as_str().unwrap()).await;
    let revoke = app
        .delete(
            &format!("/v1/devices/{}", phone["device_id"].as_str().unwrap()),
            bob["token"].as_str(),
        )
        .await;
    assert_eq!(revoke.status(), 204);
    assert!(
        closed_within(&mut phone_ws, TIMEOUT).await,
        "revoked device's socket"
    );

    let mut tablet_ws = connect_authed(&app.base, tablet["token"].as_str().unwrap()).await;
    let logout = app
        .delete("/v1/sessions/current", tablet["token"].as_str())
        .await;
    assert!(logout.status().is_success());
    assert!(
        closed_within(&mut tablet_ws, TIMEOUT).await,
        "logged-out device's socket"
    );

    let mut laptop_ws = connect_authed(&app.base, bob["token"].as_str().unwrap()).await;
    let deleted = app
        .client
        .delete(app.url("/v1/account"))
        .bearer_auth(bob["token"].as_str().unwrap())
        .json(&json!({ "password": PW }))
        .send()
        .await
        .expect("request");
    assert_eq!(deleted.status(), 204);
    assert!(
        closed_within(&mut laptop_ws, TIMEOUT).await,
        "deleted account's socket"
    );
}
