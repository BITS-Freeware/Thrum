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

async fn login_from(app: &TestApp, ip: &str, username: &str, password: &str) -> u16 {
    app.client
        .post(app.url("/v1/sessions"))
        .header("x-forwarded-for", ip)
        .json(&json!({ "username": username, "password": password }))
        .send()
        .await
        .expect("login")
        .status()
        .as_u16()
}

/// Wrong guesses from one address lock that address out of the account,
/// not the owner logging in from elsewhere.
#[tokio::test]
async fn a_lockout_does_not_shut_out_the_owner_at_another_address() {
    let app = TestApp::spawn_with(Config {
        trusted_proxies: vec!["127.0.0.1".to_string()],
        ..Config::default()
    })
    .await;
    register(&app, "alice", "correct horse battery").await;
    for _ in 0..5 {
        assert_eq!(
            login_from(&app, "203.0.113.7", "alice", "attacker guess!").await,
            401
        );
    }
    assert_eq!(
        login_from(&app, "203.0.113.7", "alice", "correct horse battery").await,
        429
    );
    assert_eq!(
        login_from(&app, "198.51.100.4", "alice", "correct horse battery").await,
        200
    );
}

/// Failures spread over many addresses still stop at an account-wide
/// ceiling.
#[tokio::test]
async fn failures_from_many_addresses_hit_an_account_ceiling() {
    let app = TestApp::spawn_with(Config {
        trusted_proxies: vec!["127.0.0.1".to_string()],
        login_failures_per_account_per_hour: 6,
        ..Config::default()
    })
    .await;
    register(&app, "alice", "correct horse battery").await;
    for n in 0..6 {
        let ip = format!("203.0.113.{}", n + 1);
        assert_eq!(login_from(&app, &ip, "alice", "attacker guess!").await, 401);
    }
    assert_eq!(
        login_from(&app, "198.51.100.4", "alice", "attacker guess!").await,
        429
    );
}

/// Addresses in one IPv6 /64 share their limits.
#[tokio::test]
async fn one_ipv6_prefix_shares_a_bucket() {
    let app = TestApp::spawn_with(Config {
        register_per_minute: 1,
        trusted_proxies: vec!["127.0.0.1".to_string()],
        ..Config::default()
    })
    .await;
    let register_from = async |ip: &str, name: &str| {
        app.client
            .post(app.url("/v1/accounts"))
            .header("x-forwarded-for", ip)
            .json(&json!({ "username": name, "password": "correct horse battery" }))
            .send()
            .await
            .expect("register")
            .status()
            .as_u16()
    };
    assert_eq!(register_from("2001:db8:1:2::1", "alice").await, 201);
    assert_eq!(register_from("2001:db8:1:2:ffff::9", "bob").await, 429);
    assert_eq!(register_from("2001:db8:1:3::1", "carol").await, 201);
}

/// Wrong passwords on account deletion count toward the same lockout as
/// login, so a stolen token is not an unlimited password oracle.
#[tokio::test]
async fn deletion_password_guesses_count_toward_the_lockout() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let session = common::login(&app, "alice", "correct horse battery", "laptop").await;
    let token = session["token"].as_str().unwrap();
    let delete = async |password: &str| {
        app.client
            .delete(app.url("/v1/account"))
            .bearer_auth(token)
            .json(&json!({ "password": password }))
            .send()
            .await
            .expect("delete")
            .status()
            .as_u16()
    };
    for _ in 0..5 {
        assert_eq!(delete("a wrong guess").await, 401);
    }
    assert_eq!(delete("correct horse battery").await, 429);
    let login = app
        .post_json(
            "/v1/sessions",
            &json!({ "username": "alice", "password": "correct horse battery" }),
        )
        .await;
    assert_eq!(login.status(), 429);
}

/// Login and registration bodies are small; a huge username is refused
/// before it reaches any limiter.
#[tokio::test]
async fn oversized_login_bodies_are_refused() {
    let app = TestApp::spawn().await;
    let name = "x".repeat(64 * 1024);
    for path in ["/v1/sessions", "/v1/accounts"] {
        let response = app
            .post_json(
                path,
                &json!({ "username": name, "password": "correct horse battery" }),
            )
            .await;
        assert_eq!(response.status(), 413, "{path}");
    }
}
