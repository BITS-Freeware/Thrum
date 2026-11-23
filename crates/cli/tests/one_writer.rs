//! End to end: one device runs in one process at a time, so a second
//! command on the same device waits instead of splitting the device's state
//! and its live connection.

mod support;

use std::process::{Command, Stdio};
use std::time::Duration;

use support::{Server, expect_ok, recv_until, run_cli};

const PASSWORD: &str = "correct horse battery";

/// While `thrum recv` listens, a `thrum send` from the same device waits
/// for it; `recv` keeps receiving live, and the send goes out afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn a_send_waits_for_a_running_recv_on_the_same_device() {
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = tempfile::tempdir().unwrap();
    let login = |name: &str| {
        expect_ok(
            &run_cli(&[
                "--server",
                &base,
                "register",
                "--username",
                name,
                "--password",
                PASSWORD,
            ]),
            "register",
        );
        let state = dir
            .path()
            .join(format!("{name}.json"))
            .to_string_lossy()
            .into_owned();
        expect_ok(
            &run_cli(&[
                "--server",
                &base,
                "--state-file",
                &state,
                "login",
                "--username",
                name,
                "--password",
                PASSWORD,
            ]),
            "login",
        );
        state
    };
    let alice = login("alice");
    let bob = login("bob");
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
        "create",
    );
    let conversation = out
        .trim()
        .strip_prefix("conversation ")
        .unwrap()
        .to_string();
    recv_until(&base, &bob, "joined", 4);

    let listener = Command::new(env!("CARGO_BIN_EXE_thrum"))
        .args([
            "--server",
            &base,
            "--state-file",
            &bob,
            "recv",
            "--timeout",
            "6",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(2));
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
            "before",
        ]),
        "send before",
    );
    let bob_send = Command::new(env!("CARGO_BIN_EXE_thrum"))
        .args([
            "--server",
            &base,
            "--state-file",
            &bob,
            "send",
            "--conversation",
            &conversation,
            "--text",
            "from bob",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(1));
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
            "after",
        ]),
        "send after",
    );

    let printed =
        String::from_utf8_lossy(&listener.wait_with_output().unwrap().stdout).into_owned();
    assert!(printed.contains("alice: before"), "{printed:?}");
    assert!(
        printed.contains("alice: after"),
        "recv went deaf: {printed:?}"
    );
    let sent = bob_send.wait_with_output().unwrap();
    assert!(sent.status.success(), "bob's send failed: {sent:?}");
    assert!(
        String::from_utf8_lossy(&sent.stderr).contains("waiting for another thrum command"),
        "bob's send did not wait: {sent:?}"
    );
    let alice_sees = recv_until(&base, &alice, "from bob", 3);
    assert!(alice_sees.contains("bob: from bob"), "{alice_sees:?}");
}
