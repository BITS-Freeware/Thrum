mod common;

use common::{TestApp, login, register};
use serde_json::json;

#[tokio::test]
async fn register_login_logout_roundtrip() {
    let app = TestApp::spawn().await;

    let user = register(&app, "alice", "correct horse battery").await;
    assert_eq!(user["username"], "alice");

    // The same name in another case is a duplicate.
    let response = app
        .post_json(
            "/v1/accounts",
            &json!({ "username": "ALICE", "password": "another password here" }),
        )
        .await;
    assert_eq!(response.status(), 409);

    // Logging in works with any case and answers with the canonical name.
    let session = login(&app, "ALICE", "correct horse battery", "phone").await;
    assert_eq!(session["username"], "alice");
    let token = session["token"].as_str().expect("token").to_string();
    assert!(token.len() >= 32, "token should be long enough");
    assert!(
        session["device_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );

    // Wrong password and unknown user are rejected identically.
    let response = app
        .post_json(
            "/v1/sessions",
            &json!({ "username": "alice", "password": "wrong password" }),
        )
        .await;
    assert_eq!(response.status(), 401);
    let response = app
        .post_json(
            "/v1/sessions",
            &json!({ "username": "nobody", "password": "wrong password" }),
        )
        .await;
    assert_eq!(response.status(), 401);

    // The new session token authorizes requests...
    let response = app.get("/v1/devices", Some(&token)).await;
    assert_eq!(response.status(), 200);

    // ...until logout revokes it.
    let response = app.delete("/v1/sessions/current", Some(&token)).await;
    assert_eq!(response.status(), 204);
    let response = app.get("/v1/devices", Some(&token)).await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn invalid_usernames_and_passwords_are_rejected() {
    let app = TestApp::spawn().await;
    for username in [
        "",
        "has space",
        "no-dash",
        "_underscore",
        "\u{fc}nicode",
        &"a".repeat(33),
    ] {
        let response = app
            .post_json(
                "/v1/accounts",
                &json!({ "username": username, "password": "valid password" }),
            )
            .await;
        assert_eq!(response.status(), 400, "username {username:?}");
    }
    for password in ["short", "", "bad\0pass", &"x".repeat(1025)] {
        let response = app
            .post_json(
                "/v1/accounts",
                &json!({ "username": "alice", "password": password }),
            )
            .await;
        assert_eq!(response.status(), 400, "password of len {}", password.len());
    }
}

#[tokio::test]
async fn malformed_bodies_are_client_errors() {
    let app = TestApp::spawn().await;
    let response = app
        .client
        .post(app.url("/v1/accounts"))
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .expect("request");
    let status = response.status().as_u16();
    assert!(
        status == 400 || status == 422,
        "malformed body should be a client error, got {status}"
    );
}

/// A signed-out or revoked device keeps no hash of its old token.
#[tokio::test]
async fn ended_sessions_keep_no_token_hash() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let laptop = login(&app, "alice", "correct horse battery", "laptop").await;
    let phone = login(&app, "alice", "correct horse battery", "phone").await;
    let laptop_token = laptop["token"].as_str().unwrap();
    let phone_token = phone["token"].as_str().unwrap();
    let logout = app.delete("/v1/sessions/current", Some(laptop_token)).await;
    assert_eq!(logout.status(), 204);
    let revoke = app
        .delete(
            &format!("/v1/devices/{}", phone["device_id"].as_str().unwrap()),
            // The phone revokes itself; any signed-in device could.
            Some(phone_token),
        )
        .await;
    assert_eq!(revoke.status(), 204);
    for token in [laptop_token, phone_token] {
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices WHERE token_hash = $1")
            .bind(thrum_server::auth::token_hash(token))
            .fetch_one(&app.state.db)
            .await
            .unwrap();
        assert_eq!(kept, 0);
        assert_eq!(app.get("/v1/devices", Some(token)).await.status(), 401);
    }
}
