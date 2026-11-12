//! End-to-end tests over MLS: real server, real CLI processes, ciphertext
//! on the wire and in the database.

mod support;

use support::{Server, expect_ok, recv_once, recv_until, run_cli};

const PASSWORD_A: &str = "correct horse alpha";
const PASSWORD_B: &str = "correct horse bravo";
const PASSWORD_C: &str = "correct horse charlie";

/// A fresh temp directory for all CLI state of one test.
fn state_dir() -> String {
    let dir = std::env::temp_dir().join(format!(
        "thrum-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir.to_string_lossy().into_owned()
}

fn register_and_login(base: &str, dir: &str, username: &str, password: &str) -> String {
    expect_ok(
        &run_cli(&[
            "--server",
            base,
            "register",
            "--username",
            username,
            "--password",
            password,
        ]),
        "register",
    );
    let state_file = format!("{dir}/{username}.json");
    expect_ok(
        &run_cli(&[
            "--server",
            base,
            "--state-file",
            &state_file,
            "login",
            "--username",
            username,
            "--password",
            password,
            "--label",
            "laptop",
        ]),
        "login",
    );
    state_file
}

/// The conversation id printed by `create`.
fn create(base: &str, state_file: &str, members: &str) -> String {
    let out = expect_ok(
        &run_cli(&[
            "--server",
            base,
            "--state-file",
            state_file,
            "create",
            "--members",
            members,
        ]),
        "create",
    );
    out.trim()
        .strip_prefix("conversation ")
        .expect("conversation id")
        .to_string()
}

fn send(base: &str, state_file: &str, conversation: &str, text: &str) {
    expect_ok(
        &run_cli(&[
            "--server",
            base,
            "--state-file",
            state_file,
            "send",
            "--conversation",
            conversation,
            "--text",
            text,
        ]),
        "send",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_cli_clients_exchange_encrypted_messages() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = state_dir();
    let alice = register_and_login(&base, &dir, "alice", PASSWORD_A);
    let bob = register_and_login(&base, &dir, "bob", PASSWORD_B);

    let conversation = create(&base, &alice, "bob");

    // Bob catches up: the welcome that joins him to the MLS group.
    let joined = recv_until(&base, &bob, "joined", 4);
    assert!(joined.contains(&conversation), "joined line: {joined:?}");

    send(&base, &alice, &conversation, "hello bob");
    let got = recv_until(&base, &bob, "hello bob", 4);
    assert!(got.contains("alice: hello bob"), "got: {got:?}");

    // Bob replies while alice is offline; the queue holds it.
    send(&base, &bob, &conversation, "hello alice");
    let got = recv_until(&base, &alice, "hello alice", 4);
    assert!(got.contains("bob: hello alice"), "got: {got:?}");

    // The server database never saw plaintext.
    let db = std::fs::read(&server.db_path).expect("read db");
    let db = String::from_utf8_lossy(&db).into_owned();
    assert!(!db.contains("hello bob"), "plaintext in server db");
    assert!(!db.contains("hello alice"), "plaintext in server db");
}

#[tokio::test(flavor = "multi_thread")]
async fn added_member_cannot_read_earlier_messages() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = state_dir();
    let alice = register_and_login(&base, &dir, "alice", PASSWORD_A);
    let bob = register_and_login(&base, &dir, "bob", PASSWORD_B);
    let carol = register_and_login(&base, &dir, "carol", PASSWORD_C);

    let conversation = create(&base, &alice, "bob");
    recv_until(&base, &bob, "joined", 4);
    send(&base, &alice, &conversation, "the first secret");
    recv_until(&base, &bob, "the first secret", 4);

    // Carol joins now; she must not receive the earlier message.
    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &alice,
            "add",
            "--conversation",
            &conversation,
            "--members",
            "carol",
        ]),
        "add carol",
    );
    let joined = recv_until(&base, &carol, "joined", 4);
    assert!(joined.contains(&conversation));
    assert!(!joined.contains("the first secret"));

    send(&base, &alice, &conversation, "carol can read this");
    let got = recv_until(&base, &carol, "carol can read this", 4);
    assert!(got.contains("carol can read this"));

    let db = std::fs::read(&server.db_path).expect("read db");
    let db = String::from_utf8_lossy(&db).into_owned();
    assert!(!db.contains("the first secret"), "plaintext in server db");
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_member_stops_receiving_messages() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = state_dir();
    let alice = register_and_login(&base, &dir, "alice", PASSWORD_A);
    let bob = register_and_login(&base, &dir, "bob", PASSWORD_B);

    let conversation = create(&base, &alice, "bob");
    recv_until(&base, &bob, "joined", 4);
    send(&base, &alice, &conversation, "before removal");
    recv_until(&base, &bob, "before removal", 4);

    // The removal commit reaches bob while he is still a member; his next
    // recv processes it (no output), then the server stops delivering.
    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &alice,
            "remove",
            "--conversation",
            &conversation,
            "--member",
            "bob",
        ]),
        "remove bob",
    );
    let after = recv_once(&base, &bob);
    assert!(!after.contains("before removal"), "no replay after removal");

    send(&base, &alice, &conversation, "after removal");
    let quiet = recv_once(&base, &bob);
    assert!(
        !quiet.contains("after removal"),
        "removed member got: {quiet:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_device_joins_existing_conversations() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = state_dir();
    let alice = register_and_login(&base, &dir, "alice", PASSWORD_A);
    let bob1 = register_and_login(&base, &dir, "bob", PASSWORD_B);

    let conversation = create(&base, &alice, "bob");
    recv_until(&base, &bob1, "joined", 4);

    // Bob logs in on a second device; it has no groups yet.
    let bob2 = format!("{dir}/bob2.json");
    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &bob2,
            "login",
            "--username",
            "bob",
            "--password",
            PASSWORD_B,
            "--label",
            "phone",
        ]),
        "login bob2",
    );

    // Bob's first device runs sync, which adds the new device to the group.
    expect_ok(
        &run_cli(&["--server", &base, "--state-file", &bob1, "sync"]),
        "sync",
    );
    let joined = recv_until(&base, &bob2, "joined", 4);
    assert!(joined.contains(&conversation));

    // Now alice's message reaches the second device too.
    send(&base, &alice, &conversation, "for both devices");
    let got = recv_until(&base, &bob2, "for both devices", 4);
    assert!(got.contains("alice: for both devices"));
}
