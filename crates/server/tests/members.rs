//! Tests for the membership endpoints and key-package enumeration.

mod common;

use common::{TestApp, login, register};
use serde_json::{Value, json};

const PW: &str = "long enough password";

/// Register a user with one device and return the session object.
async fn user(app: &TestApp, name: &str) -> Value {
    register(app, name, PW).await;
    login(app, name, PW, "laptop").await
}

#[tokio::test]
async fn add_members_roundtrip() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let alice_token = alice["token"].as_str().unwrap();

    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice_token)
        .json(&json!({"members": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let id: String = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .into();

    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{id}/members")))
        .bearer_auth(alice_token)
        .json(&json!({"members": ["bob"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "add bob");

    let response = app.get("/v1/conversations", Some(alice_token)).await;
    let list: Vec<Value> = response
        .json::<Value>()
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let members: Vec<String> = list
        .iter()
        .find(|c| c["id"].as_str() == Some(id.as_str()))
        .map(|c| {
            c["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap();
    assert_eq!(members, ["alice", "bob"]);
    let _ = bob;
}

#[tokio::test]
async fn only_a_member_can_add_members() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let carol = user(&app, "carol").await;

    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({"members": ["bob"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let id: String = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .into();

    // Carol is not in the conversation: adding herself must fail.
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{id}/members")))
        .bearer_auth(carol["token"].as_str().unwrap())
        .json(&json!({"members": ["carol"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    // Bob, once a member, can add carol.
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{id}/members")))
        .bearer_auth(bob["token"].as_str().unwrap())
        .json(&json!({"members": ["carol"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn add_unknown_member_is_rejected() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({"members": []}))
        .send()
        .await
        .unwrap();
    let id: String = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .into();
    let response = app
        .client
        .post(app.url(&format!("/v1/conversations/{id}/members")))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({"members": ["nobody"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"].as_str().unwrap(), "unknown_member");
}

#[tokio::test]
async fn remove_member_roundtrip() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;

    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({"members": ["bob"]}))
        .send()
        .await
        .unwrap();
    let id: String = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .into();

    let response = app
        .delete(
            &format!("/v1/conversations/{id}/members/bob"),
            Some(alice["token"].as_str().unwrap()),
        )
        .await;
    assert_eq!(response.status(), 204);

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
    let members: Vec<String> = list
        .iter()
        .find(|c| c["id"].as_str() == Some(id.as_str()))
        .map(|c| {
            c["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap();
    assert_eq!(members, ["alice"]);
    let _ = bob;
}

#[tokio::test]
async fn remove_unknown_member_is_not_found() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .json(&json!({"members": []}))
        .send()
        .await
        .unwrap();
    let id: String = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .into();
    let response = app
        .delete(
            &format!("/v1/conversations/{id}/members/nobody"),
            Some(alice["token"].as_str().unwrap()),
        )
        .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn key_package_count_tracks_uploads_and_fetches() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let alice_token = alice["token"].as_str().unwrap();
    let alice_device = alice["device_id"].as_str().unwrap();

    let response = app.get("/v1/key-packages/count", Some(alice_token)).await;
    assert_eq!(response.json::<Value>().await.unwrap()["count"], json!(0));

    // Key packages are uploaded as raw octets, one per request.
    for i in 0..3 {
        let response = app
            .client
            .post(app.url("/v1/key-packages"))
            .bearer_auth(alice_token)
            .body(format!("kp-{i}-{}", "x".repeat(16)))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 201, "upload {i}");
    }

    let response = app.get("/v1/key-packages/count", Some(alice_token)).await;
    assert_eq!(response.json::<Value>().await.unwrap()["count"], json!(3));

    // A fetch consumes one. Fetching is unauthenticated: key packages are
    // public, single-use MLS material.
    let response = app
        .get(&format!("/v1/devices/{alice_device}/key-package"), None)
        .await;
    assert_eq!(response.status(), 200);

    let response = app.get("/v1/key-packages/count", Some(alice_token)).await;
    assert_eq!(response.json::<Value>().await.unwrap()["count"], json!(2));
}

#[tokio::test]
async fn key_package_devices_lists_only_devices_with_stock() {
    let app = TestApp::spawn().await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let alice_device = alice["device_id"].as_str().unwrap();

    let response = app
        .get(
            "/v1/users/alice/key-package-devices",
            Some(bob["token"].as_str().unwrap()),
        )
        .await;
    assert_eq!(
        response.json::<Value>().await.unwrap()["devices"],
        json!([])
    );

    app.client
        .post(app.url("/v1/key-packages"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .body("kp")
        .send()
        .await
        .unwrap();
    let response = app
        .get(
            "/v1/users/alice/key-package-devices",
            Some(bob["token"].as_str().unwrap()),
        )
        .await;
    assert_eq!(
        response.json::<Value>().await.unwrap()["devices"],
        json!([alice_device])
    );
}
