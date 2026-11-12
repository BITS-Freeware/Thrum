#![allow(dead_code)] // shared helpers; each test binary uses a subset

/// A running Geist3 server on a random loopback port.
pub struct Server {
    pub base: String,
    _dir: tempfile::TempDir,
}

impl Server {
    pub async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("e2e.sqlite");
        let state = geist3_server::state::AppState::connect_sqlite(
            &db_path.to_string_lossy(),
            geist3_server::config::Config::default(),
        )
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
