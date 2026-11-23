//! A device restored from a backup taken before its last key-package
//! top-up still gets invited: the packages the server holds for it that
//! it can no longer open are replaced, not handed to the next inviter.

mod support;

use std::path::Path;

use support::Server;
use thrum_cli::api::Client;
use thrum_cli::chat::Chat;
use thrum_cli::session;
use thrum_cli::ws::{RecvOptions, recv};

const PASSWORD: &str = "correct horse battery";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy");
        }
    }
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

#[tokio::test]
async fn a_device_restored_from_an_old_backup_can_still_be_invited() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");

    client.register("alice", PASSWORD).await.expect("register");
    let session = client
        .login("alice", PASSWORD, "laptop")
        .await
        .expect("login");
    let alice_path = dir.path().join("alice.json");
    session::save(&alice_path, &session).expect("save");
    let alice = Chat::open(client.clone(), session, &alice_path).expect("open");
    alice.top_up_key_packages().await.expect("key packages");

    client.register("bob", PASSWORD).await.expect("register");
    let session = client
        .login("bob", PASSWORD, "laptop")
        .await
        .expect("login");
    let bob_path = dir.path().join("bob.json");
    session::save(&bob_path, &session).expect("save");
    let bob = Chat::open(client.clone(), session.clone(), &bob_path).expect("open");
    let mls = dir.path().join("bob.json.mls");
    let backup = dir.path().join("backup.mls");
    // The backup is taken before this top-up.
    copy_dir(&mls, &backup);
    bob.top_up_key_packages().await.expect("key packages");
    drop(bob);

    std::fs::remove_dir_all(&mls).expect("remove");
    copy_dir(&backup, &mls);
    let bob = Chat::open(client.clone(), session, &bob_path).expect("reopen");
    bob.top_up_key_packages().await.expect("key packages");

    let conv = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create");
    alice.send_text(&conv, "hello bob").await.expect("send");
    let seen = drain(&bob).await.join("\n");
    assert!(seen.contains("hello bob"), "bob saw: {seen}");
}
