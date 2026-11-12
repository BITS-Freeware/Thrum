//! Tests for data export and account deletion.

mod common;

use common::{TestApp, login, register};

async fn two_users(app: &TestApp) -> (String, String) {
    register(app, "alice", "correct horse battery").await;
    let token_a = login(app, "alice", "correct horse battery", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();

    register(app, "bob", "battery staple").await;
    let token_b = login(app, "bob", "battery staple", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();

    (token_a, token_b)
}

async fn create_conv(app: &TestApp, token: &str, name: &str) -> String {
    let resp = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "members": [], "name": name }))
        .send()
        .await
        .expect("create conv");
    let conv: serde_json::Value = resp.json().await.expect("conv body");
    conv["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn export_empty_account() {
    let app = TestApp::spawn().await;
    let (token, _bob) = two_users(&app).await;

    let resp = app
        .client
        .get(app.url("/v1/account/export"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("export request");
    assert_eq!(resp.status(), 200);
    let doc: serde_json::Value = resp.json().await.expect("export body");
    assert_eq!(doc["format"], "geist3-export");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["account"]["username"], "alice");
    // Login creates a device, so it's not empty.
    assert!(!doc["devices"].as_array().unwrap().is_empty()); // login creates a device
    assert!(doc["conversations"].as_array().unwrap().is_empty());
    assert!(doc["messages"].as_array().unwrap().is_empty());
    assert!(doc["blobs"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn export_requires_auth() {
    let app = TestApp::spawn().await;
    let resp = app.get("/v1/account/export", None).await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn delete_account_wrong_password() {
    let app = TestApp::spawn().await;
    let (token, _) = two_users(&app).await;

    let resp = app
        .client
        .delete(app.url("/v1/account"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "password": "wrong" }))
        .send()
        .await
        .expect("delete request");
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn delete_account_removes_everything() {
    let app = TestApp::spawn().await;
    let (token_a, token_b) = two_users(&app).await;

    // Create a conversation with bob.
    let conv_id = create_conv(&app, &token_a, "test").await;
    app.client
        .post(app.url(&format!("/v1/conversations/{}/members", conv_id)))
        .bearer_auth(&token_a)
        .json(&serde_json::json!({ "members": ["bob"] }))
        .send()
        .await
        .expect("add members");

    // Delete alice's account.
    let resp = app
        .client
        .delete(app.url("/v1/account"))
        .bearer_auth(&token_a)
        .json(&serde_json::json!({ "password": "correct horse battery" }))
        .send()
        .await
        .expect("delete request");
    assert_eq!(resp.status(), 204);

    // Bob should now own the conversation (ownership transferred).
    let convs = app
        .client
        .get(app.url("/v1/conversations"))
        .bearer_auth(&token_b)
        .send()
        .await
        .expect("list convs");
    let convs: Vec<serde_json::Value> = convs.json().await.expect("convs body");
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0]["id"], conv_id);
    assert_eq!(convs[0]["admin"], "bob");

    // Alice's token is dead.
    let resp = app
        .client
        .get(app.url("/v1/account/export"))
        .bearer_auth(&token_a)
        .send()
        .await
        .expect("export after delete");
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn delete_account_transfers_ownership() {
    let app = TestApp::spawn().await;
    let (token_a, token_b) = two_users(&app).await;

    // Alice creates a conversation with bob (Alice is owner).
    let conv_id = create_conv(&app, &token_a, "test").await;
    app.client
        .post(app.url(&format!("/v1/conversations/{}/members", conv_id)))
        .bearer_auth(&token_a)
        .json(&serde_json::json!({ "members": ["bob"] }))
        .send()
        .await
        .expect("add members");

    // Delete alice's account — should transfer ownership to bob.
    let resp = app
        .client
        .delete(app.url("/v1/account"))
        .bearer_auth(&token_a)
        .json(&serde_json::json!({ "password": "correct horse battery" }))
        .send()
        .await
        .expect("delete request");
    assert_eq!(resp.status(), 204);

    // Bob should now own the conversation.
    let convs = app
        .client
        .get(app.url("/v1/conversations"))
        .bearer_auth(&token_b)
        .send()
        .await
        .expect("list convs");
    let convs: Vec<serde_json::Value> = convs.json().await.expect("convs body");
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0]["id"], conv_id);
    assert_eq!(convs[0]["admin"], "bob");
}
