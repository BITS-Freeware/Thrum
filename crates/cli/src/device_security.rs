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
    /// The server's state, so a test can play a malicious operator.
    state: thrum_server::state::AppState,
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
        let serving = state.clone();
        tokio::spawn(async move {
            thrum_server::serve(serving, listener)
                .await
                .expect("server");
        });
        Self {
            base: format!("http://{addr}"),
            state,
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
        async |conversation: &str, message_id: &str, sender: &str, payload: &[u8]| {
            chat.interpret(conversation, message_id, sender, payload)
                .await
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
    if let Ok(Incoming::Application { payload, .. }) = stolen_keys.process_incoming(
        &conversation,
        &mls,
        &thrum_core::membership::ServerRoster::new(),
    ) {
        let text = String::from_utf8_lossy(&payload);
        panic!("revoked device read a later message: {text}");
    }
}

/// The operator hands out a known device under a key of its own: in a new
/// conversation, and after the device was revoked and comes back. Either
/// way the device is not added with the new key, because this client
/// pinned the key it first saw.
#[tokio::test]
async fn a_known_device_with_another_key_is_not_added() {
    let server = Running::spawn().await;
    let client = Client::new(server.base.clone());
    client.register("alice", PASSWORD).await.expect("register");
    client.register("bob", PASSWORD).await.expect("register");
    let dir = server._dir.path();
    let (alice_session, alice_path) = login(&client, dir, "alice", "laptop").await;
    let (bob_session, bob_path) = login(&client, dir, "bob", "laptop").await;
    let (phone_session, _) = login(&client, dir, "bob", "phone").await;
    let phone = format!("bob#{}", phone_session.device_id);
    let alice = Chat::open(client.clone(), alice_session, &alice_path).expect("alice");
    let first = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create");
    assert!(alice.group_members(&first).unwrap().contains(&phone));

    // The operator swaps the phone's key and its key packages for its own.
    let impostor = DeviceCrypto::generate(phone.clone());
    let db = &server.state.db;
    sqlx::query("UPDATE devices SET signature_key = $1 WHERE id = $2")
        .bind(impostor.signature_key())
        .bind(&phone_session.device_id)
        .execute(db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM key_packages WHERE device_id = $1")
        .bind(&phone_session.device_id)
        .execute(db)
        .await
        .unwrap();
    for n in 0..4 {
        sqlx::query(
            "INSERT INTO key_packages (id, device_id, content, created_at, last_resort) \
             VALUES ($1, $2, $3, 0, 0)",
        )
        .bind(format!("impostor-{n}"))
        .bind(&phone_session.device_id)
        .bind(impostor.new_key_package().unwrap())
        .execute(db)
        .await
        .unwrap();
    }

    // A new conversation with a known account is not a first sight.
    let second = alice
        .create_conversation(&["bob".into()], "")
        .await
        .expect("create second");
    let members = alice.group_members(&second).unwrap();
    assert!(
        members
            .iter()
            .any(|m| m == &format!("bob#{}", bob_session.device_id))
    );
    assert!(
        !members.contains(&phone),
        "the impostor key was added: {members:?}"
    );
    assert!(
        alice
            .pending_devices()
            .iter()
            .any(|(_, identity)| identity == &phone),
        "the changed device should wait for the user"
    );

    // Revoked, then listed again: the device waits for the user.
    sqlx::query("UPDATE devices SET revoked_at = 1 WHERE id = $1")
        .bind(&phone_session.device_id)
        .execute(db)
        .await
        .unwrap();
    alice.sync().await.expect("sync after revoke");
    assert!(!alice.group_members(&first).unwrap().contains(&phone));
    sqlx::query("UPDATE devices SET revoked_at = NULL WHERE id = $1")
        .bind(&phone_session.device_id)
        .execute(db)
        .await
        .unwrap();
    let report = alice.sync().await.expect("sync after return");
    assert!(!alice.group_members(&first).unwrap().contains(&phone));
    assert!(
        report.pending.iter().any(|line| line.contains(&phone)),
        "pending was {report:?}"
    );
    drop(bob_path);
}
