#![allow(dead_code)] // shared helpers; each test binary uses a subset

use std::net::SocketAddr;

use serde_json::Value;

/// A running server on a random loopback port with its own database.
pub struct TestApp {
    pub base: String,
    pub client: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl TestApp {
    /// Spawn a server with default configuration.
    pub async fn spawn() -> Self {
        Self::spawn_with(geist3_server::config::Config::default()).await
    }

    /// Spawn a server with custom configuration.
    pub async fn spawn_with(cfg: geist3_server::config::Config) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("test.sqlite");
        let state = geist3_server::state::AppState::connect_sqlite(&db_path.to_string_lossy(), cfg)
            .await
            .expect("test database");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let app = geist3_server::build_router(state)
                .into_make_service_with_connect_info::<SocketAddr>();
            axum::serve(listener, app)
                .await
                .expect("server stopped unexpectedly");
        });
        Self {
            base: format!("http://{addr}"),
            client: reqwest::Client::new(),
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
