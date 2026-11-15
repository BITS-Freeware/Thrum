//! Client behavior a malicious or stolen device must not get for free.

use std::path::PathBuf;

use base64::Engine as _;
use thrum_core::envelope::decode_envelope;
use thrum_core::mls::{DeviceCrypto, Incoming};

use crate::api::Client;
use crate::chat::Chat;
use crate::session::{self, Session};
use crate::ws::{RecvOptions, recv};

const PASSWORD: &str = "correct horse battery";

struct Running {
    base: String,
    _dir: tempfile::TempDir,
}

impl Running {
    async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("security.sqlite");
        let cfg = thrum_server::config::Config {
            blob_dir: dir.path().join("blobs").to_string_lossy().into_owned(),
            ..thrum_server::config::Config::default()
        };
        let state =
            thrum_server::state::AppState::connect_database(&db_path.to_string_lossy(), cfg)
                .await
                .expect("database");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            thrum_server::serve(state, listener).await.expect("server");
        });
        Self {
            base: format!("http://{addr}"),
            _dir: dir,
        }
    }
}

async fn login(
    client: &Client,
    dir: &std::path::Path,
    username: &str,
    label: &str,
) -> (Session, PathBuf) {
    let session = client
        .login(username, PASSWORD, label)
        .await
        .expect("login");
    let path = dir.join(format!("{username}-{label}.json"));
    session::save(&path, &session).expect("save session");
    let chat = Chat::open(client.clone(), session.clone(), &path).expect("open");
    chat.top_up_key_packages().await.expect("key packages");
    (session, path)
}

async fn drain(chat: &Chat) {
    let session = chat.session.clone();
    recv(
        &session,
        &RecvOptions {
            once: false,
            timeout_secs: 1,
        },
        |conversation, message_id, sender, payload| {
            chat.interpret(conversation, message_id, sender, payload)
        },
        |_| {},
    )
    .await
    .expect("recv");
}

#[tokio::test]
async fn a_new_device_is_not_added_until_it_is_trusted() {
    let server = Running::spawn().await;
    let client = Client::new(server.base.clone());
    client.register("alice", PASSWORD).await.expect("register");
    client.register("bob", PASSWORD).await.expect("register");
    let dir = server._dir.path();
    let (alice_session, alice_path) = login(&client, dir, "alice", "laptop").await;
    let (bob_session, bob_path) = login(&client, dir, "bob", "phone").await;
    let alice = Chat::open(client.clone(), alice_session, &alice_path).expect("alice");
    let bob = Chat::open(client.clone(), bob_session, &bob_path).expect("bob");
    let conversation = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    assert!(
        bob.group_members(&conversation)
            .expect("members")
            .iter()
            .any(|identity| identity.starts_with("alice#"))
    );

    let (bob_tablet, tablet_path) = login(&client, dir, "bob", "tablet").await;
    let unexpected = format!("bob#{}", bob_tablet.device_id);
    let report = alice.sync().await.expect("sync");
    assert!(
        report.pending.iter().any(|line| line.contains(&unexpected)),
        "pending was {report:?}"
    );
    assert!(
        !alice
            .group_members(&conversation)
            .expect("members")
            .contains(&unexpected)
    );

    alice.trust_device(&unexpected).expect("trust");
    let report = alice.sync().await.expect("sync after trust");
    assert!(
        report.added.iter().any(|line| line.contains(&unexpected)),
        "added was {report:?}"
    );
    let tablet = Chat::open(client, bob_tablet, &tablet_path).expect("tablet");
    drain(&tablet).await;
    assert!(
        tablet
            .group_members(&conversation)
            .expect("tablet joined")
            .contains(&unexpected)
    );
}

#[tokio::test]
async fn a_revoked_device_cannot_read_messages_sent_afterwards() {
    let server = Running::spawn().await;
    let client = Client::new(server.base.clone());
    client.register("alice", PASSWORD).await.expect("register");
    client.register("bob", PASSWORD).await.expect("register");
    let dir = server._dir.path();
    let (alice_session, alice_path) = login(&client, dir, "alice", "laptop").await;
    let (bob_session, bob_path) = login(&client, dir, "bob", "phone").await;
    let alice = Chat::open(client.clone(), alice_session.clone(), &alice_path).expect("alice");
    let bob = Chat::open(client.clone(), bob_session, &bob_path).expect("bob");
    let conversation = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create");
    drain(&bob).await;
    alice
        .send_text(&conversation, "before revocation")
        .await
        .expect("send");
    drain(&bob).await;

    let (alice_other, _) = login(&client, dir, "alice", "other").await;
    client
        .revoke_device(&alice_other.token, &alice_session.device_id)
        .await
        .expect("revoke");
    let report = bob.sync().await.expect("bob sync");
    let stolen = format!("alice#{}", alice_session.device_id);
    assert!(
        report.removed.iter().any(|line| line.contains(&stolen)),
        "removed was {report:?}"
    );

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    bob.send_text(&conversation, "after revocation")
        .await
        .expect("later message");
    let export = client
        .export_account(&bob.session.token)
        .await
        .expect("export");
    let messages = export["messages"].as_array().expect("messages");
    let later = messages
        .iter()
        .rev()
        .find(|message| message["sender"] == "bob")
        .expect("bob's message");
    let payload = base64::engine::general_purpose::STANDARD
        .decode(later["payload"].as_str().expect("payload"))
        .expect("decode");
    let mls = decode_envelope(&payload)
        .and_then(|envelope| envelope.data())
        .expect("envelope");
    let mut mls_dir = alice_path.as_os_str().to_os_string();
    mls_dir.push(".mls");
    let stolen_keys = DeviceCrypto::load(std::path::Path::new(&mls_dir)).expect("stolen keys");
    if let Ok(Incoming::Application { payload, .. }) =
        stolen_keys.process_incoming(&conversation, &mls)
    {
        let text = String::from_utf8_lossy(&payload);
        panic!("revoked device read a later message: {text}");
    }
}
