#![allow(dead_code)] // shared helpers; each test binary uses a subset

use serde_json::Value;

/// A running server on a random loopback port with its own database.
pub struct TestApp {
    pub base: String,
    pub client: reqwest::Client,
    pub db_path: std::path::PathBuf,
    /// Directory holding the server's blob store (disk backend).
    pub blob_dir: std::path::PathBuf,
    pub state: geist3_server::state::AppState,
    _dir: tempfile::TempDir,
}

impl TestApp {
    /// Spawn a server with default configuration.
    pub async fn spawn() -> Self {
        Self::spawn_with(geist3_server::config::Config::default()).await
    }

    /// Spawn a server with custom configuration. The blob directory is
    /// always redirected inside the test's temp directory so tests never
    /// write into the source tree.
    pub async fn spawn_with(mut cfg: geist3_server::config::Config) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        cfg.blob_dir = dir.path().join("blobs").to_string_lossy().into_owned();
        let db_path = dir.path().join("test.sqlite");
        let state = geist3_server::state::AppState::connect_sqlite(&db_path.to_string_lossy(), cfg)
            .await
            .expect("test database");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn({
            let state = state.clone();
            async move {
                geist3_server::serve(state, listener)
                    .await
                    .expect("server stopped unexpectedly");
            }
        });
        Self {
            base: format!("http://{addr}"),
            client: reqwest::Client::new(),
            blob_dir: dir.path().join("blobs"),
            db_path,
            state,
            _dir: dir,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> reqwest::Response {
        self.client
            .post(self.url(path))
            .json(body)
            .send()
            .await
            .expect("request")
    }

    pub async fn get(&self, path: &str, token: Option<&str>) -> reqwest::Response {
        let mut request = self.client.get(self.url(path));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.expect("request")
    }

    pub async fn delete(&self, path: &str, token: Option<&str>) -> reqwest::Response {
        let mut request = self.client.delete(self.url(path));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.expect("request")
    }

    pub async fn patch(&self, path: &str, token: Option<&str>, body: &Value) -> reqwest::Response {
        let mut request = self.client.patch(self.url(path));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.json(body).send().await.expect("request")
    }

    pub async fn post_bytes(
        &self,
        path: &str,
        token: Option<&str>,
        body: &[u8],
    ) -> reqwest::Response {
        let mut request = self.client.post(self.url(path)).body(body.to_vec());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.expect("request")
    }
}

/// Register a user; the test fails if the server does not answer 201.
pub async fn register(app: &TestApp, username: &str, password: &str) -> Value {
    let response = app
        .post_json(
            "/v1/accounts",
            &serde_json::json!({ "username": username, "password": password }),
        )
        .await;
    assert_eq!(response.status(), 201, "register {username}");
    response.json().await.expect("register body")
}

/// Log in; the test fails if the server does not answer 200.
pub async fn login(app: &TestApp, username: &str, password: &str, label: &str) -> Value {
    let response = app
        .post_json(
            "/v1/sessions",
            &serde_json::json!({
                "username": username,
                "password": password,
                "device_label": label,
            }),
        )
        .await;
    assert_eq!(response.status(), 200, "login {username}");
    response.json().await.expect("session body")
}
