//! Account deletion is all or nothing while the account's other devices
//! keep writing, it removes the account's blob objects, and overlapping
//! uploads of one blob id never destroy the stored object.

mod common;

use common::{TestApp, login, register};
use serde_json::json;

fn busy_config() -> thrum_server::config::Config {
    thrum_server::config::Config {
        messages_per_device_per_minute: 1_000_000,
        register_per_minute: 1_000,
        login_per_minute: 1_000,
        login_per_account_per_minute: 1_000,
        ..thrum_server::config::Config::default()
    }
}

async fn count(app: &TestApp, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .bind(bind)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

fn token(session: &serde_json::Value) -> String {
    session["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn deletion_while_another_device_sends_is_all_or_nothing() {
    let app = TestApp::spawn_with(busy_config()).await;
    let password = "correct horse battery";
    for round in 0..12 {
        let name = format!("alice{round}");
        let friend = format!("bob{round}");
        register(&app, &name, password).await;
        register(&app, &friend, password).await;
        let first = token(&login(&app, &name, password, "laptop").await);
        let second = token(&login(&app, &name, password, "phone").await);
        login(&app, &friend, password, "laptop").await;
        let conv: serde_json::Value = app
            .client
            .post(app.url("/v1/conversations"))
            .bearer_auth(&first)
            .json(&json!({ "members": [friend], "name": "c" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let conv = conv["id"].as_str().unwrap().to_string();
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
            .bind(&name)
            .fetch_one(&app.state.db)
            .await
            .unwrap();

        let sender = {
            let client = app.client.clone();
            let url = app.url(&format!("/v1/conversations/{conv}/messages"));
            let second = second.clone();
            tokio::spawn(async move {
                for _ in 0..400 {
                    let response = client
                        .post(&url)
                        .bearer_auth(&second)
                        .header("content-type", "application/octet-stream")
                        .body(vec![7u8; 64])
                        .send()
                        .await
                        .unwrap();
                    if response.status() == 401 {
                        break;
                    }
                }
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let deleted = app
            .client
            .delete(app.url("/v1/account"))
            .bearer_auth(&first)
            .json(&json!({ "password": password }))
            .send()
            .await
            .unwrap();
        sender.await.unwrap();
        assert_eq!(deleted.status(), 204, "round {round}");
        assert_eq!(
            count(
                &app,
                "SELECT COUNT(*) FROM users WHERE username = $1",
                &name
            )
            .await,
            0,
            "round {round}"
        );
        let devices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
        assert_eq!(devices, 0, "round {round}");
    }
}

#[tokio::test]
async fn deleting_an_account_removes_its_blob_objects() {
    let app = TestApp::spawn().await;
    let password = "correct horse battery";
    register(&app, "alice", password).await;
    let alice = token(&login(&app, "alice", password, "laptop").await);
    let id = "0123456789abcdef0123456789abcdef";
    let uploaded = app
        .client
        .post(app.url("/v1/blobs"))
        .bearer_auth(&alice)
        .header("x-thrum-blob-id", id)
        .body(vec![1u8; 100])
        .send()
        .await
        .unwrap();
    assert_eq!(uploaded.status(), 201);
    assert!(app.blob_dir.join(id).exists());
    let deleted = app
        .client
        .delete(app.url("/v1/account"))
        .bearer_auth(&alice)
        .json(&json!({ "password": password }))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);
    assert!(!app.blob_dir.join(id).exists());
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM blob_tombstones WHERE id = $1",
            id
        )
        .await,
        0
    );
}

#[tokio::test]
async fn overlapping_uploads_of_one_id_keep_the_object() {
    let app = TestApp::spawn().await;
    let password = "correct horse battery";
    register(&app, "alice", password).await;
    let alice = token(&login(&app, "alice", password, "laptop").await);
    for round in 0..10u8 {
        let id = format!("{round:02x}{}", "ab".repeat(15));
        let upload = |byte: u8| {
            let client = app.client.clone();
            let url = app.url("/v1/blobs");
            let alice = alice.clone();
            let id = id.clone();
            async move {
                client
                    .post(url)
                    .bearer_auth(alice)
                    .header("x-thrum-blob-id", id)
                    .body(vec![byte; 2 * 1024 * 1024])
                    .send()
                    .await
                    .unwrap()
                    .status()
            }
        };
        let (a, b) = tokio::join!(upload(1), upload(2));
        assert!(a == 201 || b == 201, "round {round}: {a} {b}");
        let fetched = app
            .client
            .get(app.url(&format!("/v1/blobs/{id}")))
            .bearer_auth(&alice)
            .send()
            .await
            .unwrap();
        assert_eq!(fetched.status(), 200, "round {round}: {a} {b}");
        let body = fetched.bytes().await.unwrap();
        assert_eq!(body.len(), 2 * 1024 * 1024);
    }
}
