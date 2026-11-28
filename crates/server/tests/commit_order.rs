//! One commit per epoch: a commit sent with `?commit_epoch=N` is taken only
//! if no other commit took epoch N first, a retry of the last commit taken
//! gets its id again, and a commit goes back to its sender too.

mod common;

use common::{TestApp, login, register};
use serde_json::{Value, json};

const PW: &str = "long enough password";

async fn user(app: &TestApp, username: &str) -> Value {
    register(app, username, PW).await;
    login(app, username, PW, "laptop").await
}

async fn conversation(app: &TestApp, token: &str, members: &[&str]) -> String {
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(token)
        .json(&json!({ "members": members }))
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 201);
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Send a payload, as a commit for `epoch` when one is given. Returns the
/// status and the message id when there is one.
async fn send(
    app: &TestApp,
    session: &Value,
    conversation: &str,
    payload: &[u8],
    epoch: Option<u64>,
) -> (u16, Option<String>) {
    let mut path = format!("/v1/conversations/{conversation}/messages");
    if let Some(epoch) = epoch {
        path.push_str(&format!("?commit_epoch={epoch}"));
    }
    let response = app
        .post_bytes(&path, session["token"].as_str(), payload)
        .await;
    let status = response.status().as_u16();
    let id = response
        .json::<Value>()
        .await
        .ok()
        .and_then(|body| body["id"].as_str().map(str::to_string));
    (status, id)
}

/// The devices a message is queued for.
async fn queued_for(app: &TestApp, message_id: &str) -> Vec<String> {
    let mut devices: Vec<String> =
        sqlx::query_scalar("SELECT device_id FROM deliveries WHERE message_id = $1")
            .bind(message_id)
            .fetch_all(&app.state.db)
            .await
            .unwrap();
    devices.sort();
    devices
}

fn device(session: &Value) -> String {
    session["device_id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_second_commit_for_the_same_epoch_is_refused() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conv = conversation(&app, alice["token"].as_str().unwrap(), &["bob"]).await;

    let (status, first) = send(&app, &alice, &conv, b"alice's commit", Some(3)).await;
    assert_eq!(status, 201);
    let (status, _) = send(&app, &bob, &conv, b"bob's commit", Some(3)).await;
    assert_eq!(status, 409, "epoch 3 is taken");
    let (status, _) = send(&app, &bob, &conv, b"bob's older commit", Some(2)).await;
    assert_eq!(status, 409, "epoch 2 is behind");
    let (status, next) = send(&app, &bob, &conv, b"bob's next commit", Some(4)).await;
    assert_eq!(status, 201, "epoch 4 follows");
    assert_ne!(first, next);
}

#[tokio::test]
async fn a_retried_commit_gets_its_id_again_and_is_queued_once() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conv = conversation(&app, alice["token"].as_str().unwrap(), &["bob"]).await;

    let (_, first) = send(&app, &alice, &conv, b"commit", Some(0)).await;
    let (status, again) = send(&app, &alice, &conv, b"commit", Some(0)).await;
    assert_eq!(status, 201);
    assert_eq!(first, again, "a retry is the same message");
    let messages: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE conversation_id = $1")
            .bind(&conv)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(messages, 1);
    let _ = bob;
}

#[tokio::test]
async fn a_commit_goes_back_to_its_sender_and_a_message_does_not() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conv = conversation(&app, alice["token"].as_str().unwrap(), &["bob"]).await;

    let (_, commit) = send(&app, &alice, &conv, b"commit", Some(0)).await;
    let mut both = vec![device(&alice), device(&bob)];
    both.sort();
    assert_eq!(queued_for(&app, &commit.unwrap()).await, both);
    let (_, message) = send(&app, &alice, &conv, b"hello", None).await;
    assert_eq!(
        queued_for(&app, &message.unwrap()).await,
        vec![device(&bob)]
    );
}

#[tokio::test]
async fn a_commit_without_an_epoch_does_not_hold_back_later_ones() {
    // Older clients send commits as plain messages; the server cannot see
    // those, so a commit for a later epoch than it expects is taken.
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let conv = conversation(&app, alice["token"].as_str().unwrap(), &[]).await;
    assert_eq!(send(&app, &alice, &conv, b"one", Some(0)).await.0, 201);
    assert_eq!(send(&app, &alice, &conv, b"unmarked", None).await.0, 201);
    assert_eq!(send(&app, &alice, &conv, b"three", Some(2)).await.0, 201);
}

#[tokio::test]
async fn only_members_send_commits() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let mallory = user(&app, "mallory").await;
    let conv = conversation(&app, alice["token"].as_str().unwrap(), &[]).await;
    assert_eq!(send(&app, &mallory, &conv, b"commit", Some(0)).await.0, 404);
    assert_eq!(send(&app, &alice, &conv, b"commit", Some(0)).await.0, 201);
}
