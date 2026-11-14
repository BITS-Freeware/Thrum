mod common;

use common::{TestApp, register};
use serde_json::json;
use thrum_server::config::Config;

#[tokio::test]
async fn register_is_rate_limited() {
    let app = TestApp::spawn_with(Config {
        register_per_minute: 2,
        ..Config::default()
    })
    .await;
    for username in ["alice", "bob"] {
        let response = app
            .post_json(
                "/v1/accounts",
                &json!({ "username": username, "password": "correct horse battery" }),
            )
            .await;
        assert_eq!(response.status(), 201, "{username}");
    }
    let response = app
        .post_json(
            "/v1/accounts",
            &json!({ "username": "carol", "password": "correct horse battery" }),
        )
        .await;
    assert_eq!(response.status(), 429);
}

#[tokio::test]
async fn login_is_rate_limited() {
    let app = TestApp::spawn_with(Config {
        login_per_minute: 2,
        ..Config::default()
    })
    .await;
    register(&app, "alice", "correct horse battery").await;
    for _ in 0..2 {
        let response = app
            .post_json(
                "/v1/sessions",
                &json!({ "username": "alice", "password": "correct horse battery" }),
            )
            .await;
        assert_eq!(response.status(), 200);
    }
    let response = app
        .post_json(
            "/v1/sessions",
            &json!({ "username": "alice", "password": "correct horse battery" }),
        )
        .await;
    assert_eq!(response.status(), 429);
}
