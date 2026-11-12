//! End-to-end test: a real server, two real CLI clients, messages both
//! ways, including one sent while the receiver is offline.

mod support;

use std::process::Stdio;
use std::time::Duration;

use support::{Server, expect_ok, run_cli};

const PASSWORD_A: &str = "correct horse alpha";
const PASSWORD_B: &str = "correct horse bravo";

#[tokio::test(flavor = "multi_thread")]
async fn two_cli_clients_exchange_messages() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = tempfile::tempdir().expect("temp dir");
    let alice_state = dir.path().join("alice.json");
    let bob_state = dir.path().join("bob.json");
    let alice = alice_state.to_string_lossy().into_owned();
    let bob = bob_state.to_string_lossy().into_owned();

    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "register",
            "--username",
            "alice",
            "--password",
            PASSWORD_A,
        ]),
        "register alice",
    );
    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "register",
            "--username",
            "bob",
            "--password",
            PASSWORD_B,
        ]),
        "register bob",
    );
    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &alice,
            "login",
            "--username",
            "alice",
            "--password",
            PASSWORD_A,
            "--label",
            "laptop",
        ]),
        "login alice",
    );
    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &bob,
            "login",
            "--username",
            "bob",
            "--password",
            PASSWORD_B,
            "--label",
            "phone",
        ]),
        "login bob",
    );

    let out = expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &alice,
            "create",
            "--members",
            "bob",
        ]),
        "create conversation",
    );
    let conversation = out
        .trim()
        .strip_prefix("conversation ")
        .expect("conversation id")
        .to_string();

    // Bob starts listening; alice sends. Whether bob is already connected
    // or still connecting, the server must deliver the message.
    let bob_child = spawn_recv(&base, &bob);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let sent = expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &alice,
            "send",
            "--conversation",
            &conversation,
            "--text",
            "hello bob",
        ]),
        "alice sends",
    );
    assert!(sent.starts_with("sent "), "send output: {sent:?}");
    let bob_output = wait_for(bob_child).await;
    assert!(
        bob_output.contains("hello bob"),
        "bob should receive the message, got: {bob_output:?}"
    );

    // Bob is offline now. This message must wait in the queue.
    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &alice,
            "send",
            "--conversation",
            &conversation,
            "--text",
            "second for bob",
        ]),
        "alice sends while bob is offline",
    );
    let bob_output = recv_once(&base, &bob).await;
    assert!(
        bob_output.contains("second for bob"),
        "queued message must be delivered on reconnect, got: {bob_output:?}"
    );

    // Bob replies while alice has never been connected at all.
    expect_ok(
        &run_cli(&[
            "--server",
            &base,
            "--state-file",
            &bob,
            "send",
            "--conversation",
            &conversation,
            "--text",
            "hello alice",
        ]),
        "bob sends",
    );
    let alice_output = recv_once(&base, &alice).await;
    assert!(
        alice_output.contains("hello alice"),
        "alice should receive the reply, got: {alice_output:?}"
    );
}

/// Spawn `geist3 recv --once` with captured stdout.
fn spawn_recv(base: &str, state_file: &str) -> tokio::process::Child {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_geist3"))
        .args([
            "--server",
            base,
            "--state-file",
            state_file,
            "recv",
            "--once",
            "--timeout",
            "20",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn recv")
}

/// Wait for a spawned recv to finish and return its stdout.
async fn wait_for(child: tokio::process::Child) -> String {
    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .expect("recv finishes in time")
        .expect("wait for recv");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Run `geist3 recv --once` to completion and return its stdout.
async fn recv_once(base: &str, state_file: &str) -> String {
    let child = spawn_recv(base, state_file);
    wait_for(child).await
}
