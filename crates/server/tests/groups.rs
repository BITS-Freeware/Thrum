//! Tests for M4 conversation features: group names, the admin role, invite
//! links and blocking.

mod common;

use common::{TestApp, login, register};
use serde_json::{Value, json};

const PW: &str = "long enough password";

/// Register a user with one device and return the session object.
async fn user(app: &TestApp, name: &str) -> Value {
    register(app, name, PW).await;
    login(app, name, PW, "laptop").await
}

/// `alice` creates a named conversation with the listed members; she is its
/// admin. Returns the conversation id.
async fn conversation(app: &TestApp, alice: &Value, members: &[&str]) -> String {
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({ "members": members, "name": "crew" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn create_records_name_and_admin() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let _bob = user(&app, "bob").await;

    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({ "members": ["bob"], "name": "  crew  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["name"], "crew", "the name is trimmed");
    assert_eq!(body["admin"], "alice");
}

#[tokio::test]
async fn rename_is_admin_only() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let id = conversation(&app, &alice, &["bob"]).await;

    // A member who is not the admin cannot rename.
    let response = app
        .patch(
            &format!("/v1/conversations/{id}/name"),
            Some(bob["token"].as_str().unwrap()),
            &json!({ "name": "bob's crew" }),
        )
        .await;
    assert_eq!(response.status(), 403);

    // The admin can rename; the list then reflects name and admin.
    let response = app
        .patch(
            &format!("/v1/conversations/{id}/name"),
            Some(alice["token"].as_str().unwrap()),
            &json!({ "name": "new crew" }),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["name"], "new crew");
    assert_eq!(body["admin"], "alice");

    let response = app
        .get("/v1/conversations", Some(alice["token"].as_str().unwrap()))
        .await;
    let list: Vec<Value> = response
        .json::<Value>()
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let found = list
        .iter()
        .find(|c| c["id"].as_str() == Some(id.as_str()))
        .unwrap();
    assert_eq!(found["name"], "new crew");
}

#[tokio::test]
async fn rename_rejects_bad_names() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let id = conversation(&app, &alice, &[]).await;

    let long = "x".repeat(300);
    let response = app
        .patch(
            &format!("/v1/conversations/{id}/name"),
            Some(alice["token"].as_str().unwrap()),
            &json!({ "name": long }),
        )
        .await;
    assert_eq!(response.status(), 400);

    let response = app
        .patch(
            &format!("/v1/conversations/{id}/name"),
            Some(alice["token"].as_str().unwrap()),
            &json!({ "name": "bad\u{7}name" }),
        )
        .await;
    assert_eq!(response.status(), 400);

    let response = app
        .patch(
            "/v1/conversations/no-such-conversation/name",
            Some(alice["token"].as_str().unwrap()),
            &json!({ "name": "crew" }),
        )
        .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn invite_roundtrip() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let carol = user(&app, "carol").await;
    let dave = user(&app, "dave").await;
    let id = conversation(&app, &alice, &["bob"]).await;

    // A member who is not the admin cannot mint an invite.
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{id}/invites")))
        .bearer_auth(bob["token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    // The admin mints one; carol redeems it and becomes a member.
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{id}/invites")))
        .bearer_auth(alice["token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let token: String = response.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!token.is_empty());

    let response = app
        .client
        .post(app.url("/v1/invites/redeem"))
        .bearer_auth(carol["token"].as_str().unwrap())
        .json(&json!({ "token": token }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], id.as_str());
    let members: Vec<String> = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap().to_string())
        .collect();
    assert!(members.contains(&"carol".to_string()));

    // A revoked token is dead: redeeming it answers 404.
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{id}/invites")))
        .bearer_auth(alice["token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    let token: String = response.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let response = app
        .delete(
            &format!("/v1/conversations/{id}/invites/{token}"),
            Some(alice["token"].as_str().unwrap()),
        )
        .await;
    assert_eq!(response.status(), 204);
    let response = app
        .client
        .post(app.url("/v1/invites/redeem"))
        .bearer_auth(dave["token"].as_str().unwrap())
        .json(&json!({ "token": token }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let _ = bob;
}

#[tokio::test]
async fn blocks_prevent_new_shared_conversations() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let _carol = user(&app, "carol").await;

    let response = app
        .client
        .post(app.url("/v1/blocks"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({ "username": "bob" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);

    // Neither side can create a conversation containing both.
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({ "members": ["bob"], "name": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(bob["token"].as_str().unwrap())
        .json(&json!({ "members": ["alice"], "name": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    // A blocked user cannot redeem the blocker's invites either.
    let id = conversation(&app, &alice, &[]).await;
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{id}/invites")))
        .bearer_auth(alice["token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    let token: String = response.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let response = app
        .client
        .post(app.url("/v1/invites/redeem"))
        .bearer_auth(bob["token"].as_str().unwrap())
        .json(&json!({ "token": token }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    // Lifting the block restores both paths.
    let response = app
        .delete("/v1/blocks/bob", Some(alice["token"].as_str().unwrap()))
        .await;
    assert_eq!(response.status(), 204);
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({ "members": ["bob"], "name": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
}
