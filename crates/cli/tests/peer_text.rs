//! What a peer chooses (message text, reply ids, attachment names and blob
//! ids) cannot reach the terminal raw, crash the CLI, or steer its
//! requests to another route.

mod support;

use std::path::{Path, PathBuf};

use support::Server;
use thrum_cli::api::Client;
use thrum_cli::chat::Chat;
use thrum_cli::session::{self, Session};
use thrum_cli::ws::{RecvOptions, recv};
use thrum_core::envelope::{AppMessage, Envelope, encode_envelope};
use thrum_core::mls::DeviceCrypto;

const PASSWORD: &str = "correct horse battery";

async fn user(client: &Client, dir: &Path, username: &str) -> (Chat, Session, PathBuf) {
    client.register(username, PASSWORD).await.expect("register");
    let session = client
        .login(username, PASSWORD, "laptop")
        .await
        .expect("login");
    let path = dir.join(format!("{username}.json"));
    session::save(&path, &session).expect("save session");
    let chat = Chat::open(client.clone(), session.clone(), &path).expect("open");
    chat.top_up_key_packages().await.expect("key packages");
    (chat, session, path)
}

async fn drain(chat: &Chat) -> Vec<String> {
    let session = chat.session.clone();
    let mut lines = Vec::new();
    recv(
        &session,
        &RecvOptions {
            once: false,
            timeout_secs: 1,
        },
        async |conversation: &str, message_id: &str, sender: &str, payload: &[u8]| {
            chat.interpret(conversation, message_id, sender, payload)
                .await
        },
        |line| lines.push(line),
    )
    .await
    .expect("recv");
    lines
}

fn mls_dir(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".mls");
    PathBuf::from(name)
}

/// Escape sequences in text and attachment names arrive escaped, and a
/// reply id with a multi-byte character where it is shortened is shown
/// instead of crashing the receiver.
#[tokio::test]
async fn peer_text_reaches_the_terminal_escaped() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice, _, alice_path) = user(&client, dir.path(), "alice").await;
    let (bob, _, _) = user(&client, dir.path(), "bob").await;
    let conv = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create");
    drain(&bob).await;

    let evil = "ok\u{1b}[2K\r\u{1b}[1Aalice: wire the money\u{1b}]52;c;cHduZWQ=\u{7}";
    alice.send_text(&conv, evil).await.expect("send");
    alice
        .send_reply(&conv, "abcdefg\u{e9}xyz", "replying")
        .await
        .expect("reply");
    drain(&alice).await;
    let alice_keys = DeviceCrypto::load(&mls_dir(&alice_path)).expect("alice keys");
    let attachment = AppMessage::Attachment {
        blob_id: "00".repeat(16),
        key_b64: "AA==".into(),
        nonce_b64: "AA==".into(),
        name: "a\u{1b}]0;owned\u{7}\u{202e}txt.exe".into(),
        mime: "text/plain".into(),
        size: 1,
        sha256: "00".into(),
    };
    let wire = alice_keys
        .encrypt(&conv, &serde_json::to_vec(&attachment).unwrap())
        .expect("encrypt");
    let shown = bob
        .interpret(
            &conv,
            "m-attach",
            "alice",
            &encode_envelope(&Envelope::mls(&wire)),
        )
        .await
        .expect("attachment line");

    let mut lines = drain(&bob).await;
    lines.push(shown);
    assert_eq!(lines.len(), 3, "{lines:?}");
    for line in &lines {
        assert!(
            !line
                .chars()
                .any(|c| c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c)),
            "{line:?}"
        );
    }
    assert!(
        lines[0].starts_with("alice: ok\\u{1b}[2K\\r"),
        "{:?}",
        lines[0]
    );
    assert_eq!(lines[1], "alice: [re:abcdefgé] replying");
    assert!(
        lines[2].contains("a\\u{1b}]0;owned\\u{7}\\u{202e}txt.exe"),
        "{:?}",
        lines[2]
    );
}

/// An attachment whose blob id is not 32 hex characters is ignored, so a
/// member cannot point another member's `fetch` at a route of its choosing.
#[tokio::test]
async fn an_attachment_with_a_path_for_an_id_is_ignored() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice, _, alice_path) = user(&client, dir.path(), "alice").await;
    let (bob, _, _) = user(&client, dir.path(), "bob").await;
    let conv = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    let alice_keys = DeviceCrypto::load(&mls_dir(&alice_path)).expect("alice keys");
    let attachment = AppMessage::Attachment {
        blob_id: "../devices/0123/key-package".into(),
        key_b64: "AA==".into(),
        nonce_b64: "AA==".into(),
        name: "photo.jpg".into(),
        mime: "image/jpeg".into(),
        size: 1,
        sha256: "00".into(),
    };
    let wire = alice_keys
        .encrypt(&conv, &serde_json::to_vec(&attachment).unwrap())
        .expect("encrypt");
    let line = bob
        .interpret(
            &conv,
            "m-path",
            "alice",
            &encode_envelope(&Envelope::mls(&wire)),
        )
        .await
        .expect("a line");
    assert_eq!(line, "alice sent an attachment with an invalid id; ignored");
    let error = bob
        .fetch_attachment("m-path", None)
        .await
        .expect_err("nothing to fetch");
    assert!(error.contains("not in this device's store"), "{error}");
}

/// Ids in request paths are one encoded segment each, and `.` or `..` is
/// refused, whatever the caller passes.
#[tokio::test]
async fn request_paths_keep_ids_in_their_segment() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let _ = socket
            .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
            .await;
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });
    let client = Client::new(base);
    let _ = client
        .fetch_blob("victim-token", "../devices/0123/key-package")
        .await;
    let request = server.await.unwrap();
    let first = request.lines().next().unwrap().to_string();
    assert_eq!(
        first,
        "GET /v1/blobs/..%2Fdevices%2F0123%2Fkey-package HTTP/1.1"
    );
    let refused = client.fetch_blob("victim-token", "..").await;
    assert_eq!(refused, Err("invalid id in request path".to_string()));
}
