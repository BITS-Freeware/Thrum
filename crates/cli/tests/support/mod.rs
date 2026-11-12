#![allow(dead_code)] // shared helpers; each test binary uses a subset

use std::time::Duration;

/// A running Geist3 server on a random loopback port.
pub struct Server {
    pub base: String,
    pub db_path: std::path::PathBuf,
    /// Directory holding the server's blob store (disk backend).
    pub blob_dir: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl Server {
    pub async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("e2e.sqlite");
        // Keep blobs inside the temp dir so tests never write into the
        // source tree.
        let cfg = geist3_server::config::Config {
            blob_dir: dir.path().join("blobs").to_string_lossy().into_owned(),
            ..geist3_server::config::Config::default()
        };
        let state = geist3_server::state::AppState::connect_sqlite(&db_path.to_string_lossy(), cfg)
            .await
            .expect("test database");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            geist3_server::serve(state, listener)
                .await
                .expect("server stopped unexpectedly");
        });
        Self {
            base: format!("http://{addr}"),
            db_path,
            blob_dir: dir.path().join("blobs"),
            _dir: dir,
        }
    }
}

/// Run the geist3 binary.
pub fn run_cli(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_geist3"))
        .args(args)
        .output()
        .expect("run geist3")
}

/// Run the geist3 binary and fail the test on a non-zero exit.
pub fn expect_ok(output: &std::process::Output, what: &str) -> String {
    assert!(
        output.status.success(),
        "{what} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// One `recv --once` run; returns everything it printed. An empty result
/// means nothing was delivered (or only silent control messages).
pub fn recv_once(base: &str, state_file: &str) -> String {
    let output = run_cli(&[
        "--server",
        base,
        "--state-file",
        state_file,
        "recv",
        "--once",
        "--timeout",
        "8",
    ]);
    expect_ok(&output, "recv")
}

/// Run `recv --once` repeatedly until the output contains `needle` or the
/// tries run out. MLS handshake messages produce no output, so one exchange
/// often takes a couple of rounds.
pub fn recv_until(base: &str, state_file: &str, needle: &str, tries: usize) -> String {
    let mut collected = String::new();
    for _ in 0..tries {
        let round = recv_once(base, state_file);
        if round.contains(needle) {
            return collected + &round;
        }
        collected.push_str(&round);
    }
    panic!("did not receive {needle:?} in {tries} recv rounds; got: {collected:?}");
}

/// Sleep that also yields the runtime.
pub async fn pause(millis: u64) {
    tokio::time::sleep(Duration::from_millis(millis)).await;
}

/// Run the geist3 binary and fail the test on a zero exit; returns stderr.
pub fn expect_fail(args: &[&str], what: &str) -> String {
    let output = run_cli(args);
    assert!(
        !output.status.success(),
        "{what} unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}
