//! A device that missed roster changes while offline catches up before it
//! plans any of its own, so it stays in the same group as everyone else.

mod support;

use std::path::Path;

use support::Server;
use thrum_cli::api::Client;
use thrum_cli::chat::Chat;
use thrum_cli::session;

const PASSWORD: &str = "correct horse battery";

async fn user(client: &Client, dir: &Path, username: &str) -> Chat {
    client.register(username, PASSWORD).await.expect("register");
    let session = client
        .login(username, PASSWORD, "laptop")
        .await
        .expect("login");
    let path = dir.join(format!("{username}.json"));
    session::save(&path, &session).expect("save session");
    let chat = Chat::open(client.clone(), session, &path).expect("open");
    chat.top_up_key_packages().await.expect("key packages");
    chat
}

async fn drain(chat: &Chat) -> Vec<String> {
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
    .await
    .expect("drain");
    lines
}

/// Bob is offline while alice adds dave. Coming back, bob syncs (as
/// `thrum recv` does before it listens). He must apply alice's commit
/// first, not commit his own add of dave in the old epoch, so carol and
/// bob can still read each other.
#[tokio::test]
async fn sync_after_a_missed_add_does_not_fork_the_group() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let alice = user(&client, dir.path(), "alice").await;
    let bob = user(&client, dir.path(), "bob").await;
    let carol = user(&client, dir.path(), "carol").await;
    let dave = user(&client, dir.path(), "dave").await;
    let conv = alice
        .create_conversation(&["bob".into(), "carol".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    drain(&carol).await;

    // Bob is away for this.
    alice
        .add_members(&conv, &["dave".into()])
        .await
        .expect("add dave");
    drain(&carol).await;
    drain(&dave).await;

    bob.sync().await.expect("bob syncs on return");
    drain(&bob).await;
    drain(&carol).await;
    drain(&dave).await;
    drain(&alice).await;

    carol
        .send_text(&conv, "from carol")
        .await
        .expect("carol sends");
    let seen = drain(&bob).await.join("\n");
    assert!(seen.contains("from carol"), "bob saw: {seen}");
    bob.send_text(&conv, "from bob").await.expect("bob sends");
    let seen = drain(&carol).await.join("\n");
    assert!(seen.contains("from bob"), "carol saw: {seen}");
    let seen = drain(&dave).await.join("\n");
    assert!(seen.contains("from bob"), "dave saw: {seen}");
}
