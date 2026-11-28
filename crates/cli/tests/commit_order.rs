//! One commit per epoch, end to end: a commit that lost its epoch to
//! another device's is dropped, and a commit whose reply was lost is
//! merged when the server hands it back, with its welcome sent after.

mod support;

use std::path::{Path, PathBuf};

use support::Server;
use thrum_cli::api::Client;
use thrum_cli::chat::Chat;
use thrum_cli::session::{self, Session};
use thrum_cli::ws::{RecvOptions, recv};
use thrum_core::envelope::{Envelope, encode_envelope};
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

async fn drain(chat: &Chat) {
    let session = chat.session.clone();
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
        |_| {},
    )
    .await
    .expect("recv");
}

fn mls_dir(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".mls");
    PathBuf::from(name)
}

/// Bob builds a commit in the same epoch as the admin's removal, which
/// the server took first. The server refuses bob's, and bob follows the
/// admin's change instead of splitting off on his own.
#[tokio::test]
async fn a_commit_that_lost_its_epoch_is_dropped() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice, _, _) = user(&client, dir.path(), "alice").await;
    let (bob, bob_session, bob_path) = user(&client, dir.path(), "bob").await;
    let (carol, carol_session, _) = user(&client, dir.path(), "carol").await;
    let conv = alice
        .create_conversation(&["bob".into(), "carol".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    drain(&carol).await;
    drop(bob);

    // Bob, offline, stages a commit of his own in the current epoch.
    let carol_leaf = format!("carol#{}", carol_session.device_id);
    let offline = DeviceCrypto::load(&mls_dir(&bob_path)).expect("bob keys");
    let stale = offline
        .stage_remove_member(&conv, &carol_leaf)
        .expect("stage");
    offline.save(&mls_dir(&bob_path)).expect("save");

    // Meanwhile the admin's removal takes that epoch.
    alice.remove_member(&conv, "carol").await.expect("remove");
    let refused = client
        .send_commit(
            &bob_session.token,
            &conv,
            &encode_envelope(&Envelope::mls(&stale.commit)),
            stale.epoch,
        )
        .await
        .expect("send");
    assert_eq!(refused, None, "the epoch is taken");

    // Bob comes back: he applies the admin's commit, which drops his own.
    let bob = Chat::open(client.clone(), bob_session.clone(), &bob_path).expect("reopen");
    bob.sync().await.expect("sync");
    assert_eq!(bob.group_members(&conv).expect("group").len(), 2);
    let after = alice.send_text(&conv, "just us").await.expect("send");
    drain(&bob).await;
    assert!(bob.find_message(&after).is_some(), "bob follows the group");
    let from_bob = bob.send_text(&conv, "agreed").await.expect("send");
    drain(&alice).await;
    assert!(alice.find_message(&from_bob).is_some(), "alice reads bob");
}

/// The admin's add commit reaches the server but its reply is lost. The
/// server hands the commit back, the admin merges it, and the next sync
/// sends the welcome, so the new member joins the group everyone is in.
#[tokio::test]
async fn a_commit_whose_reply_was_lost_is_merged_when_it_comes_back() {
    let server = Server::spawn().await;
    let client = Client::new(server.base.clone());
    let dir = tempfile::tempdir().expect("dir");
    let (alice, alice_session, alice_path) = user(&client, dir.path(), "alice").await;
    let (bob, _, _) = user(&client, dir.path(), "bob").await;
    let (dave, dave_session, _) = user(&client, dir.path(), "dave").await;
    let conv = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    // Dave joins on the server; the MLS add is what goes astray.
    client
        .add_members(&alice_session.token, &conv, &["dave".into()])
        .await
        .expect("server add");
    drop(alice);

    let offline = DeviceCrypto::load(&mls_dir(&alice_path)).expect("alice keys");
    let package = client
        .fetch_key_package(&alice_session.token, &dave_session.device_id)
        .await
        .expect("fetch")
        .expect("a package");
    let pending = offline.stage_add_members(&conv, &[package]).expect("stage");
    offline.save(&mls_dir(&alice_path)).expect("save");
    let taken = client
        .send_commit(
            &alice_session.token,
            &conv,
            &encode_envelope(&Envelope::mls(&pending.commit)),
            pending.epoch,
        )
        .await
        .expect("send");
    assert!(taken.is_some(), "the server took it; alice never heard");

    let alice = Chat::open(client.clone(), alice_session.clone(), &alice_path).expect("reopen");
    alice.sync().await.expect("sync");
    assert_eq!(alice.group_members(&conv).expect("group").len(), 3);
    drain(&bob).await;
    drain(&dave).await;
    assert_eq!(dave.group_members(&conv).expect("dave joined").len(), 3);
    let hello = alice.send_text(&conv, "welcome dave").await.expect("send");
    drain(&bob).await;
    drain(&dave).await;
    assert!(bob.find_message(&hello).is_some(), "bob reads alice");
    assert!(dave.find_message(&hello).is_some(), "dave reads alice");
}

/// The server removal comes first, so a removed member's device never
/// gets the commit that removes it and keeps its old group. Added again,
/// it is behind the epoch the server knows, so it drops that group and
/// joins the current one instead of refusing the welcome.
#[tokio::test]
async fn a_member_removed_and_added_again_joins_the_current_group() {
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
    drain(&carol).await;
    assert!(
        carol.group_members(&conv).is_ok(),
        "carol keeps her old group"
    );
    carol.top_up_key_packages().await.expect("key packages");

    alice
        .add_members(&conv, &["carol".into()])
        .await
        .expect("add again");
    drain(&bob).await;
    drain(&carol).await;
    assert_eq!(carol.group_members(&conv).expect("rejoined").len(), 3);
    let hello = alice.send_text(&conv, "welcome back").await.expect("send");
    drain(&bob).await;
    drain(&carol).await;
    assert!(carol.find_message(&hello).is_some(), "carol reads alice");
    assert!(bob.find_message(&hello).is_some(), "bob reads alice");
}
