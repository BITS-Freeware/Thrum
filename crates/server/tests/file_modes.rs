//! The server's SQLite files and stored blobs are readable by the server's
//! own user only.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::{TestApp, login, register};

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

#[tokio::test]
async fn database_and_blob_files_are_private() {
    if std::env::var("THRUM_TEST_DB_URL").is_ok() {
        return; // PostgreSQL keeps its own files.
    }
    unsafe extern "C" {
        fn umask(mask: u32) -> u32;
    }
    // SAFETY: umask only sets the process's file mode creation mask.
    unsafe { umask(0o022) };
    let app = TestApp::spawn().await;
    register(&app, "alice", "long enough password").await;
    let alice = login(&app, "alice", "long enough password", "laptop").await;
    let id = "ab".repeat(16);
    let response = app
        .client
        .post(app.url("/v1/blobs"))
        .bearer_auth(alice["token"].as_str().unwrap())
        .header("x-thrum-blob-id", &id)
        .body(vec![1u8; 64])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);

    assert_eq!(mode(&app.db_path), 0o600, "database");
    let mut wal = app.db_path.as_os_str().to_os_string();
    wal.push("-wal");
    assert_eq!(mode(std::path::Path::new(&wal)), 0o600, "write-ahead log");
    assert_eq!(mode(&app.blob_dir), 0o700, "blob directory");
    assert_eq!(mode(&app.blob_dir.join(&id)), 0o600, "blob");
}
