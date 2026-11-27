//! A message the CLI could not apply because the network failed stays
//! queued on the server, so the device applies it on the next try instead
//! of losing it.

mod support;

use std::path::Path;

use support::Server;
use thrum_cli::api::Client;
use thrum_cli::chat::Chat;
use thrum_cli::session::{self, Session};

const PASSWORD: &str = "correct horse battery";

async fn user(client: &Client, dir: &Path, username: &str) -> (Session, std::path::PathBuf) {
    client.register(username, PASSWORD).await.expect("register");
    let session = client
        .login(username, PASSWORD, "laptop")
        .await
        .expect("login");
    let path = dir.join(format!("{username}.json"));
    session::save(&path, &session).expect("save session");
    let chat = Chat::open(client.clone(), session.clone(), &path).expect("open");
    chat.top_up_key_packages().await.expect("key packages");
    (session, path)
}

async fn drain(chat: &Chat) -> Result<Vec<String>, String> {
    let session = chat.session.clone();
    let mut lines = Vec::new();
    thrum_cli::ws::drain(
        &session,
        async |conversation: &str, message_id: &str, sender: &str, payload: &[u8]| {
            chat.interpret(conversation, message_id, sender, payload)
                .await
        },
        |line| lines.push(line),
    )
    .await?;
    Ok(lines)
}

/// An address nothing listens on: HTTP requests to it fail at once.
fn unreachable_base() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    drop(listener);
    format!("http://{address}")
}

/// Bob's WebSocket works but his HTTP requests fail, so he cannot fetch
/// the roster his welcome is checked against. The welcome must stay
/// queued: once his requests work again, he joins and can read alice.
#[tokio::test]
async fn a_welcome_that_hit_a_network_error_stays_queued() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice_session, alice_path) = user(&client, dir.path(), "alice").await;
    let (bob_session, bob_path) = user(&client, dir.path(), "bob").await;
    let alice = Chat::open(client.clone(), alice_session, &alice_path).expect("alice");
    let conv = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create");

    {
        let offline = Client::new(unreachable_base());
        let bob = Chat::open(offline, bob_session.clone(), &bob_path).expect("bob, offline");
        let error = drain(&bob)
            .await
            .expect_err("the welcome cannot be applied");
        assert!(error.contains("stays queued"), "{error}");
    }

    let bob = Chat::open(client.clone(), bob_session, &bob_path).expect("bob, online");
    let lines = drain(&bob).await.expect("drain").join("\n");
    assert!(
        lines.contains(&format!("joined {conv}")),
        "bob saw: {lines}"
    );
    alice.send_text(&conv, "hello bob").await.expect("send");
    let lines = drain(&bob).await.expect("drain").join("\n");
    assert!(lines.contains("hello bob"), "bob saw: {lines}");
}
