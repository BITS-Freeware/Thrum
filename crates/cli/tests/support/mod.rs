#![allow(dead_code)] // shared helpers; each test binary uses a subset

/// A running Thrum server on a random loopback port.
pub struct Server {
    pub base: String,
    _dir: tempfile::TempDir,
}

impl Server {
    pub async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("e2e.sqlite");
        let state = thrum_server::state::AppState::connect_sqlite(
            &db_path.to_string_lossy(),
            thrum_server::config::Config::default(),
        )
        .await
        .expect("test database");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            thrum_server::serve(state, listener)
                .await
                .expect("server stopped unexpectedly");
        });
        Self {
            base: format!("http://{addr}"),
            _dir: dir,
        }
    }
}

/// Run the thrum binary.
pub fn run_cli(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_thrum"))
        .args(args)
        .output()
        .expect("run thrum")
}

/// Run the thrum binary and fail the test on a non-zero exit.
pub fn expect_ok(output: &std::process::Output, what: &str) -> String {
    assert!(
        output.status.success(),
        "{what} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}
