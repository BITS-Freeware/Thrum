//! Limits on what one account can make the server hold or do: group size
//! on every path, sends and queued bytes, devices, uploads, and key-package
//! stock under concurrency.

mod common;

use std::time::Duration;

use common::{TestApp, key_package, last_resort_package, login, register};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use thrum_core::protocol::{ClientFrame, ServerFrame};
use thrum_server::config::Config;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const PW: &str = "long enough password";

/// Limits that would get in the way of tests that are not about them.
fn relaxed() -> Config {
    Config {
        register_per_minute: 100_000,
        login_per_minute: 100_000,
        login_per_account_per_minute: 100_000,
        key_package_fetch_per_caller_per_minute: 100_000,
        key_package_fetch_per_caller_device_per_minute: 100_000,
        key_package_fetch_per_device_per_minute: 100_000,
        messages_per_device_per_minute: 100_000,
        ..Config::default()
    }
}

async fn user(app: &TestApp, name: &str) -> Value {
    register(app, name, PW).await;
    login(app, name, PW, "laptop").await
}

fn tok(session: &Value) -> String {
    session["token"].as_str().unwrap().to_string()
}

async fn conversation(app: &TestApp, token: &str, members: &[&str]) -> String {
    let response = app
        .client
        .post(app.url("/v1/conversations"))
        .bearer_auth(token)
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

async fn send(app: &TestApp, token: &str, conv: &str, payload: &[u8]) -> reqwest::Response {
    app.post_bytes(
        &format!("/v1/conversations/{conv}/messages"),
        Some(token),
        payload,
    )
    .await
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

async fn members(app: &TestApp, conv: &str) -> i64 {
    count(
        app,
        &format!("SELECT COUNT(*) FROM conversation_members WHERE conversation_id = '{conv}'"),
    )
    .await
}

// ── Member cap ──────────────────────────────────────────────────────

/// An unlimited invite cannot grow a group past the operator's cap.
#[tokio::test]
async fn invite_redemption_respects_the_member_cap() {
    let app = TestApp::spawn_with(Config {
        max_group_members: 3,
        ..relaxed()
    })
    .await;
    let admin = user(&app, "admin").await;
    let conv = conversation(&app, &tok(&admin), &[]).await;
    let token = invite(&app, &tok(&admin), &conv, json!({ "max_uses": 0 })).await;
    let mut statuses = Vec::new();
    for n in 0..4 {
        let joiner = user(&app, &format!("joiner{n}")).await;
        statuses.push(redeem(&app, &tok(&joiner), &token).await.status().as_u16());
    }
    assert_eq!(statuses, [200, 200, 400, 400]);
    assert_eq!(members(&app, &conv).await, 3);
}

/// Redeeming an invite you have already used changes nothing and does not
/// spend one of its uses.
#[tokio::test]
async fn redeeming_again_does_not_spend_a_use() {
    let app = TestApp::spawn_with(relaxed()).await;
    let admin = user(&app, "admin").await;
    let bob = user(&app, "bob").await;
    let carol = user(&app, "carol").await;
    let conv = conversation(&app, &tok(&admin), &[]).await;
    let token = invite(&app, &tok(&admin), &conv, json!({ "max_uses": 2 })).await;
    assert_eq!(redeem(&app, &tok(&bob), &token).await.status(), 200);
    assert_eq!(redeem(&app, &tok(&bob), &token).await.status(), 200);
    assert_eq!(redeem(&app, &tok(&carol), &token).await.status(), 200);
    assert_eq!(members(&app, &conv).await, 3);
}

/// Adds and redemptions that overlap still stop at the cap.
#[tokio::test]
async fn overlapping_joins_stay_within_the_member_cap() {
    let app = TestApp::spawn_with(Config {
        max_group_members: 3,
        ..relaxed()
    })
    .await;
    let admin = user(&app, "admin").await;
    let mut tokens = Vec::new();
    for n in 0..40 {
        tokens.push(tok(&user(&app, &format!("u{n}")).await));
    }
    for round in 0..10 {
        let conv = conversation(&app, &tok(&admin), &["u39"]).await;
        let link = invite(&app, &tok(&admin), &conv, json!({ "max_uses": 0 })).await;
        let mut tasks = Vec::new();
        for n in 0..3 {
            let (base, client, admin, conv) = (
                app.base.clone(),
                app.client.clone(),
                tok(&admin),
                conv.clone(),
            );
            let who = format!("u{}", round * 3 + n);
            tasks.push(tokio::spawn(async move {
                client
                    .post(format!("{base}/v1/conversations/{conv}/members"))
                    .bearer_auth(&admin)
                    .json(&json!({ "members": [who] }))
                    .send()
                    .await
                    .unwrap()
                    .status()
            }));
        }
        for token in &tokens[30..33] {
            let (base, client, token, link) = (
                app.base.clone(),
                app.client.clone(),
                token.clone(),
                link.clone(),
            );
            tasks.push(tokio::spawn(async move {
                client
                    .post(format!("{base}/v1/invites/redeem"))
                    .bearer_auth(&token)
                    .json(&json!({ "token": link }))
                    .send()
                    .await
                    .unwrap()
                    .status()
            }));
        }
        let mut ok = 0;
        for task in tasks {
            if task.await.unwrap() == 200 {
                ok += 1;
            }
        }
        assert_eq!(ok, 1, "round {round}");
        assert_eq!(members(&app, &conv).await, 3, "round {round}");
    }
}

// ── Sends and the queue ─────────────────────────────────────────────

/// One device can only send so many messages a minute.
#[tokio::test]
async fn sends_are_limited_per_device() {
    let app = TestApp::spawn_with(Config {
        messages_per_device_per_minute: 5,
        ..relaxed()
    })
    .await;
    let alice = user(&app, "alice").await;
    let conv = conversation(&app, &tok(&alice), &[]).await;
    for _ in 0..5 {
        assert_eq!(send(&app, &tok(&alice), &conv, b"hi").await.status(), 201);
    }
    assert_eq!(send(&app, &tok(&alice), &conv, b"hi").await.status(), 429);
    // Another device of the same account has its own allowance.
    let phone = login(&app, "alice", PW, "phone").await;
    assert_eq!(send(&app, &tok(&phone), &conv, b"hi").await.status(), 201);
}

/// What an account has sent that is still waiting for someone is capped,
/// so a member cannot fill the server's disk for a device that stays
/// offline. Once it is delivered, there is room again.
#[tokio::test]
async fn queued_bytes_per_account_are_capped() {
    let app = TestApp::spawn_with(Config {
        queued_bytes_per_account: 1000,
        ..relaxed()
    })
    .await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conv = conversation(&app, &tok(&alice), &["bob"]).await;
    assert_eq!(
        send(&app, &tok(&alice), &conv, &[1u8; 600]).await.status(),
        201
    );
    let refused = send(&app, &tok(&alice), &conv, &[2u8; 600]).await;
    assert_eq!(refused.status(), 429);
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["error"], "queue_full");
    // Parallel sends cannot pass the cap together.
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let (base, client, token, conv) = (
            app.base.clone(),
            app.client.clone(),
            tok(&alice),
            conv.clone(),
        );
        tasks.push(tokio::spawn(async move {
            client
                .post(format!("{base}/v1/conversations/{conv}/messages"))
                .bearer_auth(&token)
                .body(vec![3u8; 300])
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        if task.await.unwrap() == 201 {
            accepted += 1;
        }
    }
    assert_eq!(accepted, 1, "600 + 300 fits, a second 300 does not");

    let mut ws = connect(&app, &tok(&bob)).await;
    let got = frames(&mut ws, 3).await;
    for frame in got {
        if let ServerFrame::Message { message_id, .. } = frame {
            ack(&mut ws, &message_id).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        send(&app, &tok(&alice), &conv, &[4u8; 900]).await.status(),
        201
    );
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

/// Read frames until `want` message frames arrived or the socket goes
/// quiet.
async fn frames(ws: &mut Ws, want: usize) -> Vec<ServerFrame> {
    let mut out = Vec::new();
    let mut messages = 0;
    while messages < want
        && let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::from_millis(1500), ws.next()).await
    {
        if let WsMessage::Text(text) = message
            && let Ok(frame) = serde_json::from_str::<ServerFrame>(&text)
        {
            if matches!(frame, ServerFrame::Message { .. }) {
                messages += 1;
            }
            out.push(frame);
        }
    }
    out
}

async fn ack(ws: &mut Ws, message_id: &str) {
    let ack = serde_json::to_string(&ClientFrame::Ack {
        message_id: message_id.to_string(),
    })
    .unwrap();
    ws.send(WsMessage::text(ack)).await.unwrap();
}

/// A backlog longer than one catch-up page arrives whole and in order.
#[tokio::test]
async fn catch_up_delivers_a_long_backlog_in_order() {
    let app = TestApp::spawn_with(relaxed()).await;
    let alice = user(&app, "alice").await;
    let bob = user(&app, "bob").await;
    let conv = conversation(&app, &tok(&alice), &["bob"]).await;
    let mut sent = Vec::new();
    for n in 0..250u32 {
        let response = send(&app, &tok(&alice), &conv, format!("m{n}").as_bytes()).await;
        assert_eq!(response.status(), 201);
        sent.push(
            response.json::<Value>().await.unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let mut ws = connect(&app, &tok(&bob)).await;
    let got: Vec<String> = frames(&mut ws, 250)
        .await
        .into_iter()
        .filter_map(|frame| match frame {
            ServerFrame::Message { message_id, .. } => Some(message_id),
            _ => None,
        })
        .collect();
    assert_eq!(got, sent);
}

/// An export holds every message the caller may see, however many.
#[tokio::test]
async fn export_holds_a_long_history() {
    let app = TestApp::spawn_with(relaxed()).await;
    let alice = user(&app, "alice").await;
    let _bob = user(&app, "bob").await;
    let one = conversation(&app, &tok(&alice), &["bob"]).await;
    let two = conversation(&app, &tok(&alice), &["bob"]).await;
    let mut sent = Vec::new();
    for n in 0..230u32 {
        let conv = if n % 3 == 0 { &two } else { &one };
        let response = send(&app, &tok(&alice), conv, format!("m{n}").as_bytes()).await;
        assert_eq!(response.status(), 201);
        sent.push(
            response.json::<Value>().await.unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let doc: Value = app
        .get("/v1/account/export", Some(&tok(&alice)))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(doc["format"], "thrum-export");
    assert_eq!(doc["account"]["username"], "alice");
    assert_eq!(doc["conversations"].as_array().unwrap().len(), 2);
    let mut ids: Vec<String> = doc["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    sent.sort();
    assert_eq!(ids, sent);
}

// ── Devices ─────────────────────────────────────────────────────────

/// An account can only hold so many signed-in devices; logging one out
/// makes room.
#[tokio::test]
async fn active_devices_per_account_are_capped() {
    let app = TestApp::spawn_with(Config {
        max_devices_per_account: 3,
        ..relaxed()
    })
    .await;
    register(&app, "alice", PW).await;
    let first = login(&app, "alice", PW, "one").await;
    login(&app, "alice", PW, "two").await;
    login(&app, "alice", PW, "three").await;
    let refused = app
        .post_json(
            "/v1/sessions",
            &json!({ "username": "alice", "password": PW, "device_label": "four" }),
        )
        .await;
    assert_eq!(refused.status(), 409);
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["error"], "too_many_devices");
    // A wrong password still answers as a wrong password.
    let wrong = app
        .post_json(
            "/v1/sessions",
            &json!({ "username": "alice", "password": "not the password", "device_label": "x" }),
        )
        .await;
    assert_eq!(wrong.status(), 401);

    let out = app.delete("/v1/sessions/current", Some(&tok(&first))).await;
    assert_eq!(out.status(), 204);
    login(&app, "alice", PW, "four").await;
}

/// A device label is short text, not storage.
#[tokio::test]
async fn long_device_labels_are_refused() {
    let app = TestApp::spawn_with(relaxed()).await;
    register(&app, "alice", PW).await;
    login(&app, "alice", PW, &"x".repeat(64)).await;
    let refused = app
        .post_json(
            "/v1/sessions",
            &json!({ "username": "alice", "password": PW, "device_label": "x".repeat(65) }),
        )
        .await;
    assert_eq!(refused.status(), 400);
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["error"], "device_label_too_long");
}

// ── Uploads ─────────────────────────────────────────────────────────

/// Parallel uploads cannot pass the daily quota together.
#[tokio::test]
async fn parallel_uploads_stay_within_the_daily_quota() {
    let app = TestApp::spawn_with(Config {
        daily_upload_bytes: 1_000_000,
        ..relaxed()
    })
    .await;
    let alice = user(&app, "alice").await;
    let mut tasks = Vec::new();
    for n in 0..20u32 {
        let (base, client, token) = (app.base.clone(), app.client.clone(), tok(&alice));
        tasks.push(tokio::spawn(async move {
            client
                .post(format!("{base}/v1/blobs"))
                .bearer_auth(&token)
                .header("x-thrum-blob-id", format!("{:032x}", 100 + n))
                .body(vec![7u8; 400_000])
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    let mut created = 0;
    for task in tasks {
        if task.await.unwrap() == 201 {
            created += 1;
        }
    }
    assert_eq!(created, 2);
    let stored = count(
        &app,
        "SELECT CAST(COALESCE(SUM(size), 0) AS BIGINT) FROM blobs",
    )
    .await;
    assert_eq!(stored, 800_000);
}

// ── Key packages ────────────────────────────────────────────────────

async fn upload_kp(app: &TestApp, token: &str, body: Vec<u8>, last_resort: bool) -> u16 {
    let mut request = app
        .client
        .post(app.url("/v1/key-packages"))
        .bearer_auth(token)
        .body(body);
    if last_resort {
        request = request.header("x-thrum-key-package-role", "last-resort");
    }
    request.send().await.unwrap().status().as_u16()
}

async fn fetch_kp(app: &TestApp, token: &str, device: &str) -> (u16, Vec<u8>) {
    let response = app
        .get(&format!("/v1/devices/{device}/key-package"), Some(token))
        .await;
    let status = response.status().as_u16();
    (status, response.bytes().await.unwrap().to_vec())
}

/// Parallel uploads from one device stop at the one-time stock cap.
#[tokio::test]
async fn parallel_key_package_uploads_stay_within_the_cap() {
    let app = TestApp::spawn_with(Config {
        key_package_max_per_device: 5,
        ..relaxed()
    })
    .await;
    let owner = user(&app, "owner").await;
    let mut tasks = Vec::new();
    for _ in 0..30 {
        let (base, client, token, body) = (
            app.base.clone(),
            app.client.clone(),
            tok(&owner),
            key_package(&owner),
        );
        tasks.push(tokio::spawn(async move {
            client
                .post(format!("{base}/v1/key-packages"))
                .bearer_auth(&token)
                .body(body)
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM key_packages WHERE last_resort = 0"
        )
        .await,
        5
    );
}

/// Parallel last-resort uploads leave exactly one, the newest wins, and a
/// fetch serves it.
#[tokio::test]
async fn parallel_last_resort_uploads_leave_one() {
    let app = TestApp::spawn_with(relaxed()).await;
    let owner = user(&app, "owner").await;
    for _round in 0..10 {
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let (base, client, token, body) = (
                app.base.clone(),
                app.client.clone(),
                tok(&owner),
                last_resort_package(&owner),
            );
            tasks.push(tokio::spawn(async move {
                client
                    .post(format!("{base}/v1/key-packages"))
                    .bearer_auth(&token)
                    .header("x-thrum-key-package-role", "last-resort")
                    .body(body)
                    .send()
                    .await
                    .unwrap()
                    .status()
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), 201);
        }
        assert_eq!(
            count(
                &app,
                "SELECT COUNT(*) FROM key_packages WHERE last_resort = 1"
            )
            .await,
            1
        );
    }
}

/// Concurrent fetches take one-time packages while any remain, and never
/// the same one twice.
#[tokio::test]
async fn concurrent_fetches_use_one_time_stock_first() {
    let app = TestApp::spawn_with(relaxed()).await;
    let owner = user(&app, "owner").await;
    let fetcher = user(&app, "fetcher").await;
    let device = owner["device_id"].as_str().unwrap().to_string();
    let resort = last_resort_package(&owner);
    assert_eq!(
        upload_kp(&app, &tok(&owner), resort.clone(), true).await,
        201
    );
    for round in 0..5 {
        for _ in 0..8 {
            assert_eq!(
                upload_kp(&app, &tok(&owner), key_package(&owner), false).await,
                201
            );
        }
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (base, client, token, device) = (
                app.base.clone(),
                app.client.clone(),
                tok(&fetcher),
                device.clone(),
            );
            tasks.push(tokio::spawn(async move {
                client
                    .get(format!("{base}/v1/devices/{device}/key-package"))
                    .bearer_auth(&token)
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap()
                    .to_vec()
            }));
        }
        let mut seen = std::collections::HashSet::new();
        for task in tasks {
            let body = task.await.unwrap();
            assert_ne!(
                body, resort,
                "round {round}: last resort while stock remained"
            );
            assert!(seen.insert(body), "round {round}: one package served twice");
        }
    }
}

/// One caller cannot spend a device's whole fetch allowance: the limit
/// that refuses is per caller and device, and past the device's own
/// ceiling a fetch gets the last-resort package without using up stock.
#[tokio::test]
async fn a_stranger_cannot_spend_a_devices_fetch_limit() {
    let app = TestApp::spawn_with(Config {
        key_package_fetch_per_caller_device_per_minute: 2,
        key_package_fetch_per_device_per_minute: 3,
        ..relaxed()
    })
    .await;
    let owner = user(&app, "owner").await;
    let device = owner["device_id"].as_str().unwrap().to_string();
    let resort = last_resort_package(&owner);
    assert_eq!(
        upload_kp(&app, &tok(&owner), resort.clone(), true).await,
        201
    );
    for _ in 0..5 {
        assert_eq!(
            upload_kp(&app, &tok(&owner), key_package(&owner), false).await,
            201
        );
    }
    let stranger = user(&app, "stranger").await;
    assert_eq!(fetch_kp(&app, &tok(&stranger), &device).await.0, 200);
    assert_eq!(fetch_kp(&app, &tok(&stranger), &device).await.0, 200);
    assert_eq!(fetch_kp(&app, &tok(&stranger), &device).await.0, 429);

    let friend = user(&app, "friend").await;
    let (status, body) = fetch_kp(&app, &tok(&friend), &device).await;
    assert_eq!(status, 200);
    assert_ne!(
        body, resort,
        "the third fetch of the minute still gets stock"
    );
    let (status, body) = fetch_kp(&app, &tok(&friend), &device).await;
    assert_eq!(status, 200, "past the device ceiling, not refused");
    assert_eq!(body, resort, "past the device ceiling, the last resort");
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM key_packages WHERE last_resort = 0"
        )
        .await,
        2
    );
}
