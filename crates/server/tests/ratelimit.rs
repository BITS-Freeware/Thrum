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

#[tokio::test]
async fn login_is_limited_per_account_and_backs_off() {
    let app = TestApp::spawn_with(Config {
        login_per_minute: 100,
        login_per_account_per_minute: 2,
        login_failures_before_backoff: 100,
        ..Config::default()
    })
    .await;
    register(&app, "alice", "correct horse battery").await;
    register(&app, "bob", "correct horse battery").await;
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
    let response = app
        .post_json(
            "/v1/sessions",
            &json!({ "username": "bob", "password": "correct horse battery" }),
        )
        .await;
    assert_eq!(response.status(), 200);

    let locked = TestApp::spawn_with(Config {
        login_per_minute: 100,
        login_per_account_per_minute: 100,
        login_failures_before_backoff: 2,
        login_backoff_base_seconds: 60,
        login_backoff_cap_seconds: 60,
        ..Config::default()
    })
    .await;
    register(&locked, "alice", "correct horse battery").await;
    for _ in 0..2 {
        let response = locked
            .post_json(
                "/v1/sessions",
                &json!({ "username": "alice", "password": "wrong password" }),
            )
            .await;
        assert_eq!(response.status(), 401);
    }
    let response = locked
        .post_json(
            "/v1/sessions",
            &json!({ "username": "alice", "password": "correct horse battery" }),
        )
        .await;
    assert_eq!(response.status(), 429);
}

#[tokio::test]
async fn forwarded_client_ip_is_used_only_for_a_trusted_proxy() {
    let app = TestApp::spawn_with(Config {
        register_per_minute: 1,
        trusted_proxies: vec!["127.0.0.1".to_string()],
        ..Config::default()
    })
    .await;
    let first = app
        .client
        .post(app.url("/v1/accounts"))
        .header("x-forwarded-for", "203.0.113.8")
        .json(&json!({ "username": "alice", "password": "correct horse battery" }))
        .send()
        .await
        .expect("register");
    assert_eq!(first.status(), 201);
    let blocked = app
        .client
        .post(app.url("/v1/accounts"))
        .header("x-forwarded-for", "203.0.113.8")
        .json(&json!({ "username": "bob", "password": "correct horse battery" }))
        .send()
        .await
        .expect("register");
    assert_eq!(blocked.status(), 429);
    let other = app
        .client
        .post(app.url("/v1/accounts"))
        .header("x-forwarded-for", "203.0.113.9")
        .json(&json!({ "username": "carol", "password": "correct horse battery" }))
        .send()
        .await
        .expect("register");
    assert_eq!(other.status(), 201);

    let plain = TestApp::spawn_with(Config {
        register_per_minute: 1,
        ..Config::default()
    })
    .await;
    let ok = plain
        .client
        .post(plain.url("/v1/accounts"))
        .header("x-forwarded-for", "198.51.100.1")
        .json(&json!({ "username": "alice", "password": "correct horse battery" }))
        .send()
        .await
        .expect("register");
    assert_eq!(ok.status(), 201);
    let spoofed = plain
        .client
        .post(plain.url("/v1/accounts"))
        .header("x-forwarded-for", "198.51.100.2")
        .json(&json!({ "username": "bob", "password": "correct horse battery" }))
        .send()
        .await
        .expect("register");
    assert_eq!(spoofed.status(), 429);
}
