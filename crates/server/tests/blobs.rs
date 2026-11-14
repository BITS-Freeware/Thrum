//! Attachment blob route tests: upload/fetch roundtrip, id validation,
//! size caps, daily quota, duplicate ids, and expiry sweeping.

mod common;

use common::{TestApp, login, register};
use serde_json::Value;

/// A deterministic, well-formed 128-bit hex blob id; the seed sits in the
/// low nibble so different seeds give different ids.
fn blob_id(seed: u8) -> String {
    format!("{:032x}", seed as u128)
}

/// Upload a blob as an authenticated user; returns the response.
async fn upload(app: &TestApp, token: &str, id: &str, bytes: &[u8]) -> reqwest::Response {
    app.client
        .post(app.url("/v1/blobs"))
        .bearer_auth(token)
        .header("x-thrum-blob-id", id)
        .header("content-type", "application/octet-stream")
        .body(bytes.to_vec())
        .send()
        .await
        .expect("upload request")
}

/// Fetch a blob; returns the response.
async fn fetch(app: &TestApp, token: Option<&str>, id: &str) -> reqwest::Response {
    let mut request = app.client.get(app.url(&format!("/v1/blobs/{id}")));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.expect("fetch request")
}

#[tokio::test]
async fn upload_and_fetch_roundtrip() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let token = login(&app, "alice", "correct horse battery", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();

    let payload = vec![7u8; 1000];
    let id = blob_id(1);
    let response = upload(&app, &token, &id, &payload).await;
    assert_eq!(response.status(), 201, "upload should be created");
    let body: Value = response.json().await.expect("upload body");
    assert_eq!(body["id"], id);

    // The uploader can read it back byte for byte.
    let response = fetch(&app, Some(&token), &id).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.bytes().await.expect("fetch body").as_ref(),
        &payload[..]
    );

    // Ids are bearer capabilities: any other authenticated user may fetch
    // the (useless, encrypted) bytes; the server must not require a grant.
    register(&app, "bob", "battery staple").await;
    let bob_token = login(&app, "bob", "battery staple", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();
    let response = fetch(&app, Some(&bob_token), &id).await;
    assert_eq!(response.status(), 200, "any authenticated user can fetch");
}

#[tokio::test]
async fn rejects_unauthenticated() {
    let app = TestApp::spawn().await;
    let response = app.post_bytes("/v1/blobs", None, b"some bytes").await;
    assert_eq!(response.status(), 401);
    let response = app.get("/v1/blobs/some-id", None).await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn rejects_bad_ids() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let token = login(&app, "alice", "correct horse battery", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();

    for bad in [
        "",
        "short",
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        "0123456789abcdef0123456789abcdef0",
        "../../etc/passwd",
    ] {
        let response = upload(&app, &token, bad, b"data").await;
        assert_eq!(
            response.status(),
            400,
            "upload with malformed id {bad:?} must be refused"
        );
    }
    // A malformed path id discloses nothing, so it answers 404 like an
    // unknown blob would.
    let response = fetch(&app, Some(&token), "../escape").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn rejects_oversized_and_empty_blobs() {
    let cfg = thrum_server::config::Config {
        attachment_max_bytes: 1000,
        ..thrum_server::config::Config::default()
    };
    let app = TestApp::spawn_with(cfg).await;
    register(&app, "alice", "correct horse battery").await;
    let token = login(&app, "alice", "correct horse battery", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();

    // An empty body is meaningless as ciphertext.
    let response = upload(&app, &token, &blob_id(3), &[]).await;
    assert_eq!(response.status(), 400);

    // One byte over the cap is refused with the stable error code.
    let response = upload(&app, &token, &blob_id(4), &[0u8; 1001]).await;
    assert_eq!(response.status(), 413);
    let body: Value = response.json().await.expect("error body");
    assert_eq!(body["error"], "too_large");

    // Exactly at the cap is fine.
    let response = upload(&app, &token, &blob_id(5), &[0u8; 1000]).await;
    assert_eq!(response.status(), 201);
}

#[tokio::test]
async fn enforces_daily_quota() {
    let cfg = thrum_server::config::Config {
        daily_upload_bytes: 100,
        ..thrum_server::config::Config::default()
    };
    let app = TestApp::spawn_with(cfg).await;
    register(&app, "alice", "correct horse battery").await;
    let token = login(&app, "alice", "correct horse battery", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();

    let response = upload(&app, &token, &blob_id(6), &[0u8; 60]).await;
    assert_eq!(response.status(), 201);

    // Another 60 bytes would exceed the 100-byte quota.
    let response = upload(&app, &token, &blob_id(7), &[0u8; 60]).await;
    assert_eq!(response.status(), 429);
    let body: Value = response.json().await.expect("error body");
    assert_eq!(body["error"], "quota_exceeded");

    // A different user has an independent quota.
    register(&app, "bob", "battery staple").await;
    let bob_token = login(&app, "bob", "battery staple", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();
    let response = upload(&app, &bob_token, &blob_id(8), &[0u8; 60]).await;
    assert_eq!(response.status(), 201);
}

#[tokio::test]
async fn upload_reuses_are_conflicts() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    register(&app, "bob", "battery staple").await;
    let alice = login(&app, "alice", "correct horse battery", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();
    let bob = login(&app, "bob", "battery staple", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();
    let id = blob_id(9);

    let response = upload(&app, &alice, &id, b"first").await;
    assert_eq!(response.status(), 201);

    // Bob must not be able to replace alice's ciphertext under the same id.
    let response = upload(&app, &bob, &id, b"second").await;
    assert_eq!(response.status(), 409);

    // And alice's bytes are unchanged.
    let response = fetch(&app, Some(&alice), &id).await;
    assert_eq!(response.bytes().await.expect("body").as_ref(), b"first");
}

#[tokio::test]
async fn fetch_unknown_blob_is_not_found() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let token = login(&app, "alice", "correct horse battery", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();
    let response = fetch(&app, Some(&token), &blob_id(10)).await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn sweeper_removes_expired_blobs() {
    let app = TestApp::spawn().await;
    register(&app, "alice", "correct horse battery").await;
    let token = login(&app, "alice", "correct horse battery", "laptop").await["token"]
        .as_str()
        .expect("token")
        .to_string();
    let id = blob_id(11);
    let response = upload(&app, &token, &id, b"ephemeral").await;
    assert_eq!(response.status(), 201);

    // The ciphertext is on disk under the blob dir before expiry.
    let stored = app.blob_dir.join(&id);
    assert!(stored.exists(), "disk store must hold the ciphertext");

    // Age the row past retention, then sweep.
    sqlx::query("UPDATE blobs SET expires_at = $2 WHERE id = $1")
        .bind(&id)
        .bind(thrum_server::accounts::now() - 1)
        .execute(&app.state.db)
        .await
        .expect("age blob");

    let removed =
        thrum_server::blobs::sweep_expired(&app.state.db, app.state.blob_store.as_ref()).await;
    assert_eq!(removed, 1, "the expired blob must be removed");

    // Both the object and the row are gone.
    assert!(!stored.exists(), "expired object must be deleted");
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs WHERE id = $1")
        .bind(&id)
        .fetch_one(&app.state.db)
        .await
        .expect("count");
    assert_eq!(remaining, 0, "expired row must be deleted");

    // And the blob answers 404 like any unknown blob.
    let response = fetch(&app, Some(&token), &id).await;
    assert_eq!(response.status(), 404);
}
