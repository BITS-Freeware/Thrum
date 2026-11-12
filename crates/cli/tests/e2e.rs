//! End-to-end tests over MLS: real server, real CLI processes, ciphertext
//! on the wire and in the database.

mod support;

use support::{Server, expect_fail, expect_ok, recv_once, recv_until, run_cli};

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

const PASSWORD_D: &str = "correct horse delta";

/// `create --name <name>`; returns the conversation id.
fn create_named(base: &str, state_file: &str, members: &str, name: &str) -> String {
    let out = expect_ok(
        &run_cli(&[
            "--server",
            base,
            "--state-file",
            state_file,
            "create",
            "--members",
            members,
            "--name",
            name,
        ]),
        "create",
    );
    out.trim()
        .strip_prefix("conversation ")
        .expect("conversation id")
        .to_string()
}

/// Run one CLI command with a state file; returns stdout.
fn run_as(base: &str, state_file: &str, args: &[&str]) -> String {
    let mut full = vec!["--server", base, "--state-file", state_file];
    full.extend_from_slice(args);
    expect_ok(&run_cli(&full), args.first().unwrap_or(&""))
}

/// Run one CLI command with a state file, expecting failure; returns stderr.
fn fail_as(base: &str, state_file: &str, args: &[&str]) -> String {
    let mut full = vec!["--server", base, "--state-file", state_file];
    full.extend_from_slice(args);
    expect_fail(&full, args.first().unwrap_or(&""))
}

#[tokio::test(flavor = "multi_thread")]
async fn group_names_and_invite_links_flow() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = state_dir();
    let alice = register_and_login(&base, &dir, "alice", PASSWORD_A);
    let bob = register_and_login(&base, &dir, "bob", PASSWORD_B);
    let carol = register_and_login(&base, &dir, "carol", PASSWORD_C);
    let dave = register_and_login(&base, &dir, "dave", PASSWORD_D);

    let conversation = create_named(&base, &alice, "bob", "crew");
    let joined = recv_until(&base, &bob, "joined", 4);
    assert!(joined.contains(&conversation), "joined line: {joined:?}");

    // The admin renames; a plain member cannot.
    let out = run_as(
        &base,
        &alice,
        &[
            "rename",
            "--conversation",
            &conversation,
            "--name",
            "new crew",
        ],
    );
    assert!(out.contains("renamed"), "rename out: {out:?}");
    let err = fail_as(
        &base,
        &bob,
        &[
            "rename",
            "--conversation",
            &conversation,
            "--name",
            "bob's crew",
        ],
    );
    assert!(
        err.contains("403") && err.contains("not_admin"),
        "rename err: {err:?}"
    );

    // The admin mints an invite link; carol joins with it.
    let out = run_as(&base, &alice, &["invite", "--conversation", &conversation]);
    let token = out
        .trim()
        .strip_prefix("invite ")
        .expect("invite token")
        .to_string();
    let out = run_as(&base, &carol, &["join", "--token", &token]);
    assert!(out.contains(&conversation), "join out: {out:?}");

    // A member's sync pulls carol into the MLS group.
    recv_once(&base, &bob);
    let joined = recv_until(&base, &carol, "joined", 6);
    assert!(joined.contains(&conversation), "carol joined: {joined:?}");

    // The invited member can now read live traffic.
    send(&base, &alice, &conversation, "welcome carol");
    let got = recv_until(&base, &carol, "welcome carol", 4);
    assert!(got.contains("alice: welcome carol"), "got: {got:?}");

    // A revoked invite is dead: redeeming it fails with 404.
    let out = run_as(&base, &alice, &["invite", "--conversation", &conversation]);
    let token = out
        .trim()
        .strip_prefix("invite ")
        .expect("invite token")
        .to_string();
    let out = run_as(
        &base,
        &alice,
        &["revoke", "--conversation", &conversation, "--token", &token],
    );
    assert!(out.contains("revoked"), "revoke out: {out:?}");
    let err = fail_as(&base, &dave, &["join", "--token", &token]);
    assert!(err.contains("404"), "revoked join err: {err:?}");

    // The server database never saw plaintext.
    let db = std::fs::read(&server.db_path).expect("read db");
    let db = String::from_utf8_lossy(&db).into_owned();
    assert!(!db.contains("welcome carol"), "plaintext in server db");
}

#[tokio::test(flavor = "multi_thread")]
async fn edits_deletes_and_read_receipts_flow() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = state_dir();
    let alice = register_and_login(&base, &dir, "alice", PASSWORD_A);
    let bob = register_and_login(&base, &dir, "bob", PASSWORD_B);

    let conversation = create(&base, &alice, "bob");
    recv_until(&base, &bob, "joined", 4);

    let out = run_as(
        &base,
        &alice,
        &[
            "send",
            "--conversation",
            &conversation,
            "--text",
            "first message",
        ],
    );
    let first = out
        .trim()
        .strip_prefix("sent ")
        .expect("message id")
        .to_string();
    let out = run_as(
        &base,
        &alice,
        &[
            "send",
            "--conversation",
            &conversation,
            "--text",
            "second message",
        ],
    );
    let second = out
        .trim()
        .strip_prefix("sent ")
        .expect("message id")
        .to_string();
    let got = recv_until(&base, &bob, "second message", 4);
    assert!(got.contains("alice: first message"), "got: {got:?}");

    // Bob replies, quoting the first message by id.
    let _reply = run_as(
        &base,
        &bob,
        &[
            "send",
            "--conversation",
            &conversation,
            "--reply-to",
            &first,
            "--text",
            "a reply",
        ],
    );
    let got = recv_until(&base, &alice, "a reply", 4);
    assert!(got.contains("bob: [re:"), "got: {got:?}");

    // Alice edits her own message; bob sees the edit notice.
    let out = run_as(
        &base,
        &alice,
        &["edit", "--message", &first, "--text", "first, edited"],
    );
    assert!(out.contains("edited"), "edit out: {out:?}");
    let got = recv_until(&base, &bob, "edited", 4);
    assert!(got.contains("alice edited"), "got: {got:?}");

    // Alice deletes it; bob sees the delete notice.
    let out = run_as(&base, &alice, &["delete", "--message", &first]);
    assert!(out.contains("deleted"), "delete out: {out:?}");
    let got = recv_until(&base, &bob, "deleted", 4);
    assert!(got.contains("alice deleted"), "got: {got:?}");

    // Bob cannot edit alice's message.
    let err = fail_as(
        &base,
        &bob,
        &["edit", "--message", &second, "--text", "hacked"],
    );
    assert!(
        err.contains("not yours to edit"),
        "foreign edit err: {err:?}"
    );

    // Bob marks what he read: the deleted first and his own reply are not
    // unread, the second message is.
    let out = run_as(&base, &bob, &["read", "--conversation", &conversation]);
    assert!(
        out.contains("sent read receipts for 1 message(s)"),
        "read out: {out:?}"
    );
    let got = recv_until(&base, &alice, "read", 4);
    assert!(got.contains("bob read 1 message(s)"), "got: {got:?}");

    // The server database never saw plaintext.
    let db = std::fs::read(&server.db_path).expect("read db");
    let db = String::from_utf8_lossy(&db).into_owned();
    assert!(!db.contains("first, edited"), "plaintext in server db");
    assert!(!db.contains("a reply"), "plaintext in server db");
}

#[tokio::test(flavor = "multi_thread")]
async fn blocking_stops_new_shared_conversations() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = state_dir();
    let alice = register_and_login(&base, &dir, "alice", PASSWORD_A);
    let bob = register_and_login(&base, &dir, "bob", PASSWORD_B);

    let out = run_as(&base, &alice, &["block", "--user", "bob"]);
    assert!(out.contains("blocked bob"), "block out: {out:?}");

    // Neither side can create a conversation containing both.
    let err = fail_as(&base, &alice, &["create", "--members", "bob"]);
    assert!(
        err.contains("403") && err.contains("blocked"),
        "err: {err:?}"
    );
    let err = fail_as(&base, &bob, &["create", "--members", "alice"]);
    assert!(
        err.contains("403") && err.contains("blocked"),
        "err: {err:?}"
    );

    // Lifting the block restores the path.
    let out = run_as(&base, &alice, &["unblock", "--user", "bob"]);
    assert!(out.contains("unblocked bob"), "unblock out: {out:?}");
    let conversation = create(&base, &alice, "bob");
    assert!(!conversation.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_flow_end_to_end() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = state_dir();
    let alice = register_and_login(&base, &dir, "alice", PASSWORD_A);
    let bob = register_and_login(&base, &dir, "bob", PASSWORD_B);

    let conversation = create(&base, &alice, "bob");
    recv_until(&base, &bob, "joined", 4);

    // Alice sends a small PNG image as an attachment.
    let payload_dir = format!("{dir}/payload");
    std::fs::create_dir_all(&payload_dir).expect("payload dir");
    let original = format!("{payload_dir}/photo.png");
    let image = image::RgbImage::from_fn(32, 24, |x, y| {
        image::Rgb([(x * 8) as u8, (y * 10) as u8, 128])
    });
    image.save(&original).expect("save png");
    let plaintext = std::fs::read(&original).expect("read png");
    assert!(!plaintext.is_empty());

    let out = run_as(
        &base,
        &alice,
        &[
            "attach",
            "--conversation",
            &conversation,
            "--file",
            &original,
        ],
    );
    assert!(out.starts_with("sent "), "attach out: {out:?}");
    let message_id = out.trim().strip_prefix("sent ").expect("id").to_string();

    // Bob receives the notice with the file name.
    let got = recv_until(&base, &bob, "photo.png", 4);
    assert!(got.contains("alice sent photo.png"), "got: {got:?}");

    // ...and fetches the attachment by message id; the bytes survive and
    // still decode as the same image.
    let saved = format!("{payload_dir}/from-bob/photo.png");
    let out = run_as(
        &base,
        &bob,
        &["fetch", "--message", &message_id, "--out", &saved],
    );
    assert!(out.contains("saved"), "fetch out: {out:?}");
    assert_eq!(
        std::fs::read(&saved).expect("fetched file"),
        plaintext,
        "fetched bytes must match what was sent"
    );
    let decoded = image::open(&saved).expect("fetched file is an image");
    assert_eq!((decoded.width(), decoded.height()), (32, 24));

    // The server stored only ciphertext: neither the image bytes nor the
    // file name may appear in the database.
    let db_bytes = std::fs::read(&server.db_path).expect("read db");
    assert!(
        !db_bytes
            .windows(plaintext.len())
            .any(|w| w == plaintext.as_slice()),
        "image bytes in server db"
    );
    let db = String::from_utf8_lossy(&db_bytes);
    assert!(!db.contains("photo.png"), "file name in server db");

    // Nor in any stored blob.
    let entries = std::fs::read_dir(&server.blob_dir).expect("blob dir");
    for entry in entries {
        let bytes = std::fs::read(entry.expect("entry").path()).expect("blob bytes");
        assert!(
            !bytes.windows(plaintext.len()).any(|w| w == plaintext),
            "plaintext stored in a blob"
        );
    }

    // A plain text message has no attachment to fetch.
    let out = run_as(
        &base,
        &alice,
        &[
            "send",
            "--conversation",
            &conversation,
            "--text",
            "plain text only",
        ],
    );
    let text_id = out.trim().strip_prefix("sent ").expect("id").to_string();
    recv_until(&base, &bob, "plain text only", 4);
    let err = fail_as(&base, &bob, &["fetch", "--message", &text_id]);
    assert!(
        err.contains("carries no attachment"),
        "fetch text err: {err:?}"
    );

    // An unknown message id fails too.
    let err = fail_as(&base, &bob, &["fetch", "--message", "no-such-message"]);
    assert!(
        err.contains("not in this device's store"),
        "fetch unknown err: {err:?}"
    );
}
