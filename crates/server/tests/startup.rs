//! Starting the real binary: API paths stay JSON when the web client is
//! served beside them, and the image's own bind address gets a short note
//! rather than the full warning.

#![cfg(unix)]

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Start the server with `env` on a fresh SQLite database and wait for it.
async fn start(dir: &std::path::Path, env: &[(&str, &str)]) -> (Child, String) {
    let port = free_port();
    let mut command = Command::new(env!("CARGO_BIN_EXE_thrum-server"));
    command
        .env("THRUM_DB", dir.join("thrum.sqlite"))
        .env("THRUM_BLOB_DIR", dir.join("blobs"))
        .env("THRUM_PORT", port.to_string())
        .env_remove("THRUM_WEB_DIR")
        .env_remove("THRUM_BIND")
        .env_remove("THRUM_IMAGE_BIND")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("start server");
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client.get(format!("{base}/healthz")).send().await.is_ok() {
            return (child, base);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("server did not come up");
}

/// Stop the server and return what it wrote to stderr.
fn stop(mut child: Child) -> String {
    let _ = child.kill();
    let _ = child.wait();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    stderr
}

#[tokio::test]
async fn api_paths_are_json_beside_the_web_client() {
    if std::env::var("THRUM_TEST_DB_URL").is_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>web</title>").unwrap();
    let (child, base) = start(dir.path(), &[("THRUM_WEB_DIR", web.to_str().unwrap())]).await;

    let response = reqwest::get(format!("{base}/v1/nope")).await.unwrap();
    assert_eq!(response.status(), 404);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"], "not_found");

    // Client-side routes still get the web client.
    let response = reqwest::get(format!("{base}/conversations/x"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("<title>web</title>")
    );
    stop(child);
}

#[tokio::test]
async fn the_image_default_bind_gets_a_note_and_a_chosen_one_a_warning() {
    if std::env::var("THRUM_TEST_DB_URL").is_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (child, _) = start(
        dir.path(),
        &[("THRUM_BIND", "0.0.0.0"), ("THRUM_IMAGE_BIND", "0.0.0.0")],
    )
    .await;
    let stderr = stop(child);
    assert!(stderr.contains("note: listening on 0.0.0.0"), "{stderr}");
    assert!(!stderr.contains("warning:"), "{stderr}");

    let dir = tempfile::tempdir().unwrap();
    let (child, _) = start(dir.path(), &[("THRUM_BIND", "0.0.0.0")]).await;
    let stderr = stop(child);
    assert!(
        stderr.contains("warning: thrum-server is bound to 0.0.0.0"),
        "{stderr}"
    );
}

#[test]
fn a_database_that_never_answers_ends_startup_without_its_password() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_thrum-server"))
        .env(
            "THRUM_DB",
            format!("postgres://thrum:hunter2-secret@127.0.0.1:{port}/thrum"),
        )
        .env("THRUM_DB_CONNECT_WAIT_SECONDS", "1")
        .env("THRUM_BLOB_DIR", dir.path().join("blobs"))
        .env("THRUM_PORT", free_port().to_string())
        .env_remove("THRUM_WEB_DIR")
        .output()
        .expect("run server");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1), "{all}");
    assert!(!all.contains("hunter2-secret"), "{all}");
    assert!(all.contains("thrum:***@127.0.0.1"), "{all}");
    assert!(all.contains("waiting for the database"), "{all}");
    assert!(started.elapsed() < Duration::from_secs(10));
}
