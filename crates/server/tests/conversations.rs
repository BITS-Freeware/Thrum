//! Conversation endpoint tests: create, list, send, membership.

mod common;

use common::{TestApp, login, register};
use serde_json::{Value, json};

const PW: &str = "long enough password";

/// Register and log in a user; returns the session object.
async fn user(app: &TestApp, username: &str) -> Value {
    register(app, username, PW).await;
    login(app, username, PW, "laptop").await
}

/// Create a conversation for `token` with the given member names.
async fn create_conversation(app: &TestApp, token: &str, members: &[&str]) -> reqwest::Response {
    app.client
        .post(app.url("/v1/conversations"))
        .bearer_auth(token)
        .json(&json!({ "members": members }))
        .send()
        .await
        .expect("request")
}

/// Send a raw payload to a conversation.
async fn send_message(
    app: &TestApp,
    token: &str,
    conversation: &str,
    payload: &[u8],
) -> reqwest::Response {
    app.client
        .post(app.url(&format!("/v1/conversations/{conversation}/messages")))
        .bearer_auth(token)
        .body(payload.to_vec())
        .send()
        .await
        .expect("request")
}

/// List the caller's conversations.
async fn list_conversations(app: &TestApp, token: &str) -> reqwest::Response {
    app.get("/v1/conversations", Some(token)).await
}

#[tokio::test]
async fn conversations_create_list_roundtrip() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let carol = user(&app, "carol").await;
    let alice_token = alice["token"].as_str().expect("token");

    let response = create_conversation(&app, alice_token, &["bob"]).await;
    assert_eq!(response.status(), 201, "create conversation");
    let created: Value = response.json().await.expect("body");
    let conversation_id = created["id"].as_str().expect("id").to_string();
    assert_eq!(
        created["members"],
        json!(["alice", "bob"]),
        "members are canonical and sorted"
    );

    // Listing shows the conversation for both members, nobody else.
    let alice_list: Value = list_conversations(&app, alice_token)
        .await
        .json()
        .await
        .expect("body");
    assert_eq!(alice_list.as_array().expect("array").len(), 1);
    assert_eq!(alice_list[0]["id"], json!(conversation_id));
    let bob_list: Value = list_conversations(&app, bob["token"].as_str().expect("token"))
        .await
        .json()
        .await
        .expect("body");
    assert_eq!(bob_list.as_array().expect("array").len(), 1);
    let carol_list: Value = list_conversations(&app, carol["token"].as_str().expect("token"))
        .await
        .json()
        .await
        .expect("body");
    assert_eq!(carol_list.as_array().expect("array").len(), 0);

    // Duplicate and self membership collapse into one row each.
    let response = create_conversation(&app, alice_token, &["bob", "bob", "alice"]).await;
    assert_eq!(response.status(), 201);
    let second: Value = response.json().await.expect("body");
    assert_eq!(second["members"], json!(["alice", "bob"]));
}

#[tokio::test]
async fn unknown_members_are_rejected() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let response =
        create_conversation(&app, alice["token"].as_str().expect("token"), &["ghost"]).await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.expect("body");
    assert_eq!(body["error"], json!("unknown_member"));
}

#[tokio::test]
async fn sending_requires_membership_and_a_real_conversation() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    user(&app, "bob").await;
    let carol = user(&app, "carol").await;
    let alice_token = alice["token"].as_str().expect("token");
    let created: Value = create_conversation(&app, alice_token, &["bob"])
        .await
        .json()
        .await
        .expect("body");
    let conversation_id = created["id"].as_str().expect("id").to_string();

    // A stranger cannot send into someone else's conversation.
    let response = send_message(
        &app,
        carol["token"].as_str().expect("token"),
        &conversation_id,
        b"intrusion",
    )
    .await;
    assert_eq!(response.status(), 404);

    // A made-up conversation id is a 404 too.
    let response = send_message(
        &app,
        alice_token,
        "00000000-0000-0000-0000-000000000000",
        b"nowhere",
    )
    .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn send_returns_a_message_id() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    user(&app, "bob").await;
    let created: Value =
        create_conversation(&app, alice["token"].as_str().expect("token"), &["bob"])
            .await
            .json()
            .await
            .expect("body");
    let conversation_id = created["id"].as_str().expect("id").to_string();
    let response = send_message(
        &app,
        alice["token"].as_str().expect("token"),
        &conversation_id,
        b"opaque payload",
    )
    .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("body");
    assert!(
        body["id"].as_str().expect("id").len() >= 32,
        "message id looks like a uuid: {body}"
    );
}

#[tokio::test]
async fn empty_and_oversized_messages_are_rejected() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    user(&app, "bob").await;
    let created: Value =
        create_conversation(&app, alice["token"].as_str().expect("token"), &["bob"])
            .await
            .json()
            .await
            .expect("body");
    let conversation_id = created["id"].as_str().expect("id").to_string();
    let token = alice["token"].as_str().expect("token");

    let response = send_message(&app, token, &conversation_id, b"").await;
    assert_eq!(response.status(), 400);

    let oversized = vec![b'x'; 256 * 1024 + 1];
    let response = send_message(&app, token, &conversation_id, &oversized).await;
    assert_eq!(response.status(), 413);

    // Exactly at the limit is accepted.
    let at_limit = vec![b'x'; 256 * 1024];
    let response = send_message(&app, token, &conversation_id, &at_limit).await;
    assert_eq!(response.status(), 201);
}
