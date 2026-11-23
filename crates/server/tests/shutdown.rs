//! Stopping the server: SIGTERM ends it promptly and cleanly, and the
//! SQLite file it leaves is complete on its own, so copying that one file
//! is a backup. A deleted account's rows do not linger in the file.

#![cfg(unix)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Running {
    child: Child,
    base: String,
}

async fn start(db: &std::path::Path, blobs: &std::path::Path) -> Running {
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_thrum-server"))
        .env("THRUM_DB", db)
        .env("THRUM_BLOB_DIR", blobs)
        .env("THRUM_PORT", port.to_string())
        .env_remove("THRUM_WEB_DIR")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start server");
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client.get(format!("{base}/healthz")).send().await.is_ok() {
            return Running { child, base };
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("server did not come up");
}

fn terminate(running: &mut Running) -> (std::process::ExitStatus, Duration) {
    let started = Instant::now();
    let status = Command::new("kill")
        .args(["-TERM", &running.child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    loop {
        if let Some(status) = running.child.try_wait().unwrap() {
            return (status, started.elapsed());
        }
        if started.elapsed() > Duration::from_secs(10) {
            let _ = running.child.kill();
            panic!("server ignored SIGTERM for 10 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test]
async fn sigterm_stops_cleanly_and_the_database_file_is_complete() {
    if std::env::var("THRUM_TEST_DB_URL").is_ok() {
        return; // about the SQLite file
    }
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("thrum.sqlite");
    let mut running = start(&db, &dir.path().join("blobs")).await;
    let client = reqwest::Client::new();
    for name in ["alice", "bob", "carol"] {
        let response = client
            .post(format!("{}/v1/accounts", running.base))
            .json(&json!({ "username": name, "password": "long enough password" }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 201);
    }
    let (status, took) = terminate(&mut running);
    assert!(status.success(), "exit {status:?}");
    assert!(took < Duration::from_secs(3), "took {took:?}");

    // Copy only the main file, the way a backup of "the database" would.
    let copy_dir = tempfile::tempdir().unwrap();
    let copy = copy_dir.path().join("copy.sqlite");
    std::fs::copy(&db, &copy).unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect(&format!("sqlite://{}?mode=ro", copy.display()))
        .await
        .unwrap();
    let users: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE username IN ('alice', 'bob', 'carol')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(users, 3);
}

#[tokio::test]
async fn a_deleted_account_does_not_linger_in_the_file() {
    if std::env::var("THRUM_TEST_DB_URL").is_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("thrum.sqlite");
    let mut running = start(&db, &dir.path().join("blobs")).await;
    let client = reqwest::Client::new();
    // The name itself stays on purpose (deleted names are retired), so look
    // for the device label, which only the deleted device row held.
    let name = "zebulon";
    let label = "quixotic-label-for-the-deleted-device";
    let password = "a very distinctive password";
    client
        .post(format!("{}/v1/accounts", running.base))
        .json(&json!({ "username": name, "password": password }))
        .send()
        .await
        .unwrap();
    let session: serde_json::Value = client
        .post(format!("{}/v1/sessions", running.base))
        .json(&json!({ "username": name, "password": password, "device_label": label }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deleted = client
        .delete(format!("{}/v1/account", running.base))
        .bearer_auth(session["token"].as_str().unwrap())
        .json(&json!({ "password": password }))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);
    let (status, _) = terminate(&mut running);
    assert!(status.success());
    let mut bytes = std::fs::read(&db).unwrap();
    let mut wal = db.as_os_str().to_os_string();
    wal.push("-wal");
    if let Ok(log) = std::fs::read(&wal) {
        bytes.extend(log);
    }
    let found = bytes.windows(label.len()).any(|w| w == label.as_bytes());
    assert!(!found, "the deleted device's label is still in the file");
}
