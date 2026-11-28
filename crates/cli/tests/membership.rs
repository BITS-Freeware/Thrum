//! End to end against a real server: welcomes and sender labels that the
//! CLI must refuse, and the honest paths that must still work.

mod support;

use std::path::{Path, PathBuf};

use base64::Engine as _;
use support::Server;
use thrum_cli::api::Client;
use thrum_cli::chat::Chat;
use thrum_cli::session::{self, Session};
use thrum_cli::ws::{RecvOptions, recv};
use thrum_core::envelope::{AppMessage, Envelope, decode_envelope, encode_envelope};
use thrum_core::membership::ServerRoster;
use thrum_core::mls::{DeviceCrypto, Incoming};

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

/// The newest payload this account sent, from its own export.
async fn last_sent_payload(client: &Client, token: &str, username: &str) -> Vec<u8> {
    let export = client.export_account(token).await.expect("export");
    let message = export["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .rev()
        .find(|message| message["sender"] == username)
        .expect("a sent message")
        .clone();
    base64::engine::general_purpose::STANDARD
        .decode(message["payload"].as_str().expect("payload"))
        .expect("base64")
}

/// A relay frame that labels bob's ciphertext as alice's is dropped: the
/// sender comes from MLS, and a label that disagrees is tampering.
#[tokio::test]
async fn a_relabeled_message_is_dropped() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice, _, _) = user(&client, dir.path(), "alice").await;
    let (bob, _, bob_path) = user(&client, dir.path(), "bob").await;
    let (carol, _, _) = user(&client, dir.path(), "carol").await;
    let conv = alice
        .create_conversation(&["bob".into(), "carol".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    drain(&carol).await;
    let alice_msg = alice.send_text(&conv, "lunch at noon").await.expect("send");
    drain(&carol).await;
    drain(&bob).await;

    let bob_keys = DeviceCrypto::load(&mls_dir(&bob_path)).expect("bob keys");
    let edit = serde_json::to_vec(&AppMessage::Edit {
        message_id: alice_msg.clone(),
        text: "lunch cancelled, send money to bob".into(),
    })
    .unwrap();
    let wire = bob_keys.encrypt(&conv, &edit).expect("encrypt");
    let payload = encode_envelope(&Envelope::mls(&wire));
    assert_eq!(
        carol
            .interpret(&conv, "relabeled-1", "alice", &payload)
            .await,
        Ok(None)
    );
    let entry = carol.find_message(&alice_msg).unwrap().1;
    assert_eq!(entry.text, "lunch at noon");
    assert_eq!(entry.sender, "alice");
}

/// A welcome for conversation C sent inside another conversation is
/// refused, so the real welcome still joins bob to C and only C's real
/// members can read what he writes there.
#[tokio::test]
async fn a_welcome_from_another_conversation_cannot_take_over_a_group() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice, alice_session, _) = user(&client, dir.path(), "alice").await;
    let (bob, bob_session, _) = user(&client, dir.path(), "bob").await;
    let (mallory, mallory_session, _) = user(&client, dir.path(), "mallory").await;

    let conv_c = alice
        .create_conversation(&[], "plans")
        .await
        .expect("create");
    let invite = alice.create_invite(&conv_c).await.expect("invite");
    assert_eq!(bob.join(&invite).await.expect("join"), conv_c);

    let conv_m = mallory
        .create_conversation(&["bob".into()], "")
        .await
        .expect("mallory conv");
    let fake_alice = DeviceCrypto::generate(format!("alice#{}", alice_session.device_id));
    fake_alice.create_group(&conv_c).unwrap();
    let bob_kp = client
        .fetch_key_package(&mallory_session.token, &bob_session.device_id)
        .await
        .expect("fetch")
        .expect("a package");
    let (_commit, welcome) = fake_alice.add_members(&conv_c, &[bob_kp]).unwrap();
    let payload = encode_envelope(&Envelope::welcome(&welcome.unwrap()));
    client
        .send_message(&mallory_session.token, &conv_m, &payload)
        .await
        .expect("send fake welcome");
    drain(&bob).await;
    assert!(
        bob.group_members(&conv_c).is_err(),
        "bob joined a fake group"
    );

    alice.sync().await.expect("alice sync");
    drain(&bob).await;
    assert!(
        bob.group_members(&conv_c).is_ok(),
        "bob did not join the real group"
    );

    bob.send_text(&conv_c, "the safe code is 4711")
        .await
        .expect("bob send");
    let lines = drain(&alice).await;
    assert!(
        lines
            .iter()
            .any(|line| line == "bob: the safe code is 4711"),
        "alice cannot read bob: {lines:?}"
    );
    let sent = last_sent_payload(&client, &bob_session.token, "bob").await;
    let mls = decode_envelope(&sent).and_then(|e| e.data()).unwrap();
    let mut roster = ServerRoster::new();
    roster.add_device(fake_alice.identity(), None);
    assert!(!matches!(
        fake_alice.process_incoming(&conv_c, &mls, &roster),
        Ok(Incoming::Application { .. })
    ));
}

/// A removal the server refuses sends no commit. Before the server
/// removal went first, a member who was not the admin could still send the
/// removal commit and merge it; everyone else refused it, because the
/// server still listed the device, and the member was left alone in a newer
/// epoch, unable to read or be read.
#[tokio::test]
async fn a_removal_the_server_refuses_sends_no_commit() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice, _, _) = user(&client, dir.path(), "alice").await;
    let (bob, _, _) = user(&client, dir.path(), "bob").await;
    let (carol, _, _) = user(&client, dir.path(), "carol").await;
    let conv = alice
        .create_conversation(&["bob".into(), "carol".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    drain(&carol).await;

    let refused = bob.remove_member(&conv, "carol").await;
    assert!(refused.is_err(), "only the admin may remove: {refused:?}");
    assert_eq!(bob.group_members(&conv).expect("bob's group").len(), 3);

    let from_bob = bob.send_text(&conv, "still here").await.expect("send");
    drain(&alice).await;
    drain(&carol).await;
    assert!(alice.find_message(&from_bob).is_some(), "alice reads bob");
    assert!(carol.find_message(&from_bob).is_some(), "carol reads bob");
    let from_alice = alice.send_text(&conv, "me too").await.expect("send");
    drain(&bob).await;
    assert!(bob.find_message(&from_alice).is_some(), "bob reads alice");
}

/// The admin's removal reaches the other members, and the removed member
/// stops reading, now that the server removal goes first.
#[tokio::test]
async fn a_removal_reaches_the_other_members() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice, _, _) = user(&client, dir.path(), "alice").await;
    let (bob, _, _) = user(&client, dir.path(), "bob").await;
    let (carol, _, _) = user(&client, dir.path(), "carol").await;
    let conv = alice
        .create_conversation(&["bob".into(), "carol".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    drain(&carol).await;

    alice.remove_member(&conv, "carol").await.expect("remove");
    drain(&bob).await;
    assert_eq!(bob.group_members(&conv).expect("bob's group").len(), 2);
    let after = alice.send_text(&conv, "just us").await.expect("send");
    drain(&bob).await;
    drain(&carol).await;
    assert!(bob.find_message(&after).is_some(), "bob reads alice");
    assert!(
        carol.find_message(&after).is_none(),
        "carol no longer reads"
    );
}
