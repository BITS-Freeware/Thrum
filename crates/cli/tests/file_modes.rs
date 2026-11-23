//! What the CLI writes is readable by its owner only, and deleting the
//! account removes what the device kept.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use support::{Server, expect_ok, recv_until, run_cli};

const PASSWORD: &str = "correct horse battery";

/// The common default umask, so the test sees what a typical system would.
fn typical_umask() {
    unsafe extern "C" {
        fn umask(mask: u32) -> u32;
    }
    // SAFETY: umask only sets the process's file mode creation mask.
    unsafe { umask(0o022) };
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

fn cli(base: &str, state_file: &str, args: &[&str]) -> String {
    let mut all = vec!["--server", base, "--state-file", state_file];
    all.extend_from_slice(args);
    expect_ok(&run_cli(&all), args[0])
}

fn user(base: &str, dir: &Path, username: &str) -> String {
    expect_ok(
        &run_cli(&[
            "--server",
            base,
            "register",
            "--username",
            username,
            "--password",
            PASSWORD,
        ]),
        "register",
    );
    let state_file = dir
        .join(format!("{username}.json"))
        .to_string_lossy()
        .into_owned();
    cli(
        base,
        &state_file,
        &["login", "--username", username, "--password", PASSWORD],
    );
    state_file
}

#[tokio::test(flavor = "multi_thread")]
async fn client_files_are_private_and_deletion_removes_them() {
    typical_umask();
    let server = Server::spawn().await;
    let base = server.base.clone();
    let dir = tempfile::tempdir().expect("dir");
    let alice = user(&base, dir.path(), "alice");
    let bob = user(&base, dir.path(), "bob");
    let out = cli(&base, &alice, &["create", "--members", "bob"]);
    let conversation = out
        .trim()
        .strip_prefix("conversation ")
        .unwrap()
        .to_string();
    recv_until(&base, &bob, "joined", 4);
    cli(
        &base,
        &alice,
        &["send", "--conversation", &conversation, "--text", "secret"],
    );
    let file = dir.path().join("note.txt");
    std::fs::write(&file, b"attached secret").unwrap();
    let out = cli(
        &base,
        &alice,
        &[
            "attach",
            "--conversation",
            &conversation,
            "--file",
            file.to_str().unwrap(),
        ],
    );
    let message = out.trim().strip_prefix("sent ").unwrap().to_string();
    recv_until(&base, &bob, "note.txt", 4);

    let saved = cli(&base, &bob, &["fetch", "--message", &message]);
    let saved = saved.trim().strip_prefix("saved ").unwrap().to_string();
    let out_file = dir.path().join("out").join("copy.txt");
    cli(
        &base,
        &bob,
        &[
            "fetch",
            "--message",
            &message,
            "--out",
            out_file.to_str().unwrap(),
        ],
    );
    let export = dir.path().join("export.json");
    cli(&base, &bob, &["export", "--out", export.to_str().unwrap()]);

    let state = Path::new(&bob);
    let store = format!("{bob}.messages.json");
    let mls = format!("{bob}.mls");
    let attachments = format!("{bob}.attachments");
    assert!(std::fs::read_to_string(&store).unwrap().contains("secret"));
    let modes = [
        ("session", mode(state)),
        ("store", mode(Path::new(&store))),
        ("keys", mode(Path::new(&mls))),
        ("attachments", mode(Path::new(&attachments))),
        ("saved attachment", mode(Path::new(&saved))),
        ("fetch --out", mode(&out_file)),
        ("export", mode(&export)),
    ];
    assert_eq!(
        modes,
        [
            ("session", 0o600),
            ("store", 0o600),
            ("keys", 0o700),
            ("attachments", 0o700),
            ("saved attachment", 0o600),
            ("fetch --out", 0o600),
            ("export", 0o600),
        ]
    );

    let out = cli(
        &base,
        &bob,
        &["delete-account", "--password", PASSWORD, "--yes"],
    );
    assert!(out.starts_with("account bob deleted"), "{out}");
    for path in [bob.clone(), store, mls, attachments] {
        assert!(!Path::new(&path).exists(), "{path} is still there");
    }
    // What the user chose to save elsewhere stays theirs.
    assert!(out_file.exists());
    assert!(export.exists());
}
