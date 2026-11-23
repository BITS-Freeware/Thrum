//! Who may end up in a conversation together, what a member's export
//! holds, and what account deletion leaves for members still offline.

mod common;

use std::time::Duration;

use base64::Engine as _;
use common::{TestApp, login, register};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use thrum_core::protocol::{ClientFrame, ServerFrame};
use tokio_tungstenite::tungstenite::Message as WsMessage;

const PW: &str = "long enough password";

async fn user(app: &TestApp, name: &str) -> Value {
    register(app, name, PW).await;
    login(app, name, PW, "laptop").await
}

fn tok(session: &Value) -> &str {
    session["token"].as_str().unwrap()
}

async fn create(app: &TestApp, token: &str, members: &[&str]) -> reqwest::Response {
    app.client
        .post(app.url("/v1/conversations"))
        .bearer_auth(token)
        .json(&json!({ "members": members, "name": "crew" }))
        .send()
        .await
        .unwrap()
}

async fn conversation(app: &TestApp, token: &str, members: &[&str]) -> String {
    let response = create(app, token, members).await;
    assert_eq!(response.status(), 201);
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn block(app: &TestApp, token: &str, who: &str) {
    let response = app
        .client
        .post(app.url("/v1/blocks"))
        .bearer_auth(token)
        .json(&json!({ "username": who }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
}

async fn add(app: &TestApp, token: &str, conv: &str, members: &[&str]) -> reqwest::Response {
    app.client
        .post(app.url(&format!("/v1/conversations/{conv}/members")))
        .bearer_auth(token)
        .json(&json!({ "members": members }))
        .send()
        .await
        .unwrap()
}

async fn invite(app: &TestApp, token: &str, conv: &str, body: Value) -> String {
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{conv}/invites")))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    response.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn redeem(app: &TestApp, token: &str, invite: &str) -> reqwest::Response {
    app.client
        .post(app.url("/v1/invites/redeem"))
        .bearer_auth(token)
        .json(&json!({ "token": invite }))
        .send()
        .await
        .unwrap()
}

async fn send(app: &TestApp, token: &str, conv: &str, payload: &[u8]) -> String {
    let response = app
        .post_bytes(
            &format!("/v1/conversations/{conv}/messages"),
            Some(token),
            payload,
        )
        .await;
    assert_eq!(response.status(), 201);
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn members(app: &TestApp, token: &str, conv: &str) -> Vec<String> {
    let list: Vec<Value> = app
        .get("/v1/conversations", Some(token))
        .await
        .json()
        .await
        .unwrap();
    list.iter()
        .find(|c| c["id"] == conv)
        .map(|c| {
            c["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Decision 0007: a block between any two members refuses the change,
/// whoever makes it: create, add and redeem.
#[tokio::test]
async fn a_block_between_any_two_members_is_honoured_on_every_path() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let carol = user(&app, "carol").await;
    block(&app, tok(&alice), "bob").await;

    let created = create(&app, tok(&carol), &["alice", "bob"]).await;
    assert_eq!(created.status(), 403, "create");

    let conv = conversation(&app, tok(&carol), &["alice"]).await;
    assert_eq!(
        add(&app, tok(&carol), &conv, &["bob"]).await.status(),
        403,
        "add"
    );

    let token = invite(&app, tok(&carol), &conv, json!({})).await;
    assert_eq!(
        redeem(&app, tok(&bob), &token).await.status(),
        403,
        "redeem"
    );
    assert_eq!(members(&app, tok(&alice), &conv).await, ["alice", "carol"]);
}

/// A member the admin removed cannot come back through an invite; the
/// admin can still add them again.
#[tokio::test]
async fn a_removed_member_cannot_rejoin_by_invite() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conv = conversation(&app, tok(&alice), &[]).await;
    let token = invite(&app, tok(&alice), &conv, json!({ "max_uses": 0 })).await;
    assert_eq!(redeem(&app, tok(&bob), &token).await.status(), 200);
    let removed = app
        .delete(
            &format!("/v1/conversations/{conv}/members/bob"),
            Some(tok(&alice)),
        )
        .await;
    assert_eq!(removed.status(), 204);

    assert_eq!(redeem(&app, tok(&bob), &token).await.status(), 403);
    assert_eq!(members(&app, tok(&alice), &conv).await, ["alice"]);
    assert_eq!(add(&app, tok(&alice), &conv, &["bob"]).await.status(), 200);
    assert_eq!(members(&app, tok(&alice), &conv).await, ["alice", "bob"]);
}

/// A member's export holds the messages they sent and the ones addressed
/// to their devices, not what others were sent before they joined.
#[tokio::test]
async fn an_export_holds_only_the_callers_own_traffic() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let _bob = user(&app, "bob").await;
    let carol = user(&app, "carol").await;
    let conv = conversation(&app, tok(&alice), &["bob"]).await;
    let before = send(&app, tok(&alice), &conv, b"ciphertext for bob only").await;
    assert_eq!(
        add(&app, tok(&alice), &conv, &["carol"]).await.status(),
        200
    );
    let after = send(&app, tok(&alice), &conv, b"for bob and carol").await;
    let own = send(&app, tok(&carol), &conv, b"from carol").await;

    let doc: Value = app
        .get("/v1/account/export", Some(tok(&carol)))
        .await
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = doc["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&before.as_str()), "{ids:?}");
    assert!(ids.contains(&after.as_str()), "{ids:?}");
    assert!(ids.contains(&own.as_str()), "{ids:?}");
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(app: &TestApp, token: &str) -> Ws {
    let url = format!("{}/v1/ws", app.base.replacen("http://", "ws://", 1));
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let auth = serde_json::to_string(&ClientFrame::Auth {
        token: token.to_string(),
    })
    .unwrap();
    ws.send(WsMessage::text(auth)).await.unwrap();
    ws
}

async fn frames(ws: &mut Ws) -> Vec<ServerFrame> {
    let mut out = Vec::new();
    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(Duration::from_millis(800), ws.next()).await
    {
        if let WsMessage::Text(text) = message
            && let Ok(frame) = serde_json::from_str::<ServerFrame>(&text)
        {
            out.push(frame);
        }
    }
    out
}

/// Deleting an account keeps the messages it sent that members have not
/// received yet, so a member who was offline still gets the commits it
/// needs, under the sender's name.
#[tokio::test]
async fn deletion_keeps_what_offline_members_have_not_received() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let carol = user(&app, "carol").await;
    let conv = conversation(&app, tok(&alice), &["bob", "carol"]).await;
    let sent = send(&app, tok(&alice), &conv, b"a commit bob needs").await;
    let deleted = app
        .client
        .delete(app.url("/v1/account"))
        .bearer_auth(tok(&alice))
        .json(&json!({ "password": PW }))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);

    let mut ws = connect(&app, tok(&bob)).await;
    let got = frames(&mut ws).await;
    let message = got
        .iter()
        .find_map(|frame| match frame {
            ServerFrame::Message {
                message_id,
                sender_username,
                payload_base64,
                ..
            } if message_id == &sent => Some((sender_username.clone(), payload_base64.clone())),
            _ => None,
        })
        .expect("bob still gets alice's message");
    assert_eq!(message.0, "alice");
    let payload = base64::engine::general_purpose::STANDARD
        .decode(message.1)
        .unwrap();
    assert_eq!(payload, b"a commit bob needs");

    // Once everyone has it, it goes, and nothing else of alice's remains.
    let ack = serde_json::to_string(&ClientFrame::Ack {
        message_id: sent.clone(),
    })
    .unwrap();
    ws.send(WsMessage::text(ack)).await.unwrap();
    let mut carol_ws = connect(&app, tok(&carol)).await;
    let _ = frames(&mut carol_ws).await;
    carol_ws
        .send(WsMessage::text(
            serde_json::to_string(&ClientFrame::Ack { message_id: sent }).unwrap(),
        ))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let users: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE lower(username) = 'alice'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(users, 0);
}
