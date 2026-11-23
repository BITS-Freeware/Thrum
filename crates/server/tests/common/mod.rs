#![allow(dead_code)] // shared helpers; each test binary uses a subset

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use serde_json::Value;
use thrum_core::mls::DeviceCrypto;

/// MLS keys per device id, so that every package a test uploads for one
/// device carries the same signature key, as a real client's do.
static DEVICE_KEYS: LazyLock<Mutex<HashMap<String, Arc<DeviceCrypto>>>> =
    LazyLock::new(Default::default);

/// The MLS keys of the device a login session belongs to.
pub fn device_keys(session: &Value) -> Arc<DeviceCrypto> {
    let username = session["username"].as_str().expect("session username");
    let device_id = session["device_id"].as_str().expect("session device id");
    DEVICE_KEYS
        .lock()
        .unwrap()
        .entry(device_id.to_string())
        .or_insert_with(|| Arc::new(DeviceCrypto::generate(format!("{username}#{device_id}"))))
        .clone()
}

/// A valid one-time key package for the session's device.
pub fn key_package(session: &Value) -> Vec<u8> {
    device_keys(session).new_key_package().expect("key package")
}

/// A valid last-resort key package for the session's device.
pub fn last_resort_package(session: &Value) -> Vec<u8> {
    device_keys(session)
        .new_last_resort_key_package()
        .expect("last-resort key package")
}

/// A running server on a random loopback port with its own database.
pub struct TestApp {
    pub base: String,
    pub client: reqwest::Client,
    pub db_path: std::path::PathBuf,
    /// Directory holding the server's blob store (disk backend).
    pub blob_dir: std::path::PathBuf,
    pub state: thrum_server::state::AppState,
    _dir: tempfile::TempDir,
}

impl TestApp {
    /// Spawn a server with default configuration.
    pub async fn spawn() -> Self {
        Self::spawn_with(thrum_server::config::Config::default()).await
    }

    /// Spawn a server with custom configuration. The blob directory is
    /// always redirected inside the test's temp directory so tests never
    /// write into the source tree.
    pub async fn spawn_with(mut cfg: thrum_server::config::Config) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        cfg.blob_dir = dir.path().join("blobs").to_string_lossy().into_owned();
        let (db_url, db_path) = match std::env::var("THRUM_TEST_DB_URL") {
            // Run the same suite against PostgreSQL: every test gets its
            // own database so parallel tests cannot interfere.
            Ok(url) => {
                sqlx::any::install_default_drivers();
                static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let base = url
                    .rsplit_once('/')
                    .map(|(_, db)| db.split('?').next().unwrap_or(db))
                    .filter(|db| !db.is_empty())
                    .unwrap_or("thrum_test")
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect::<String>();
                let db_name = format!("{base}_{}_{}", std::process::id(), seq);
                let admin_url = swap_database(&url, "postgres");
                let admin = sqlx::any::AnyPoolOptions::new()
                    .max_connections(1)
                    .connect(&admin_url)
                    .await
                    .expect("admin database");
                sqlx::query(&format!("CREATE DATABASE \"{db_name}\""))
                    .execute(&admin)
                    .await
                    .expect("create test database");
                drop(admin);
                let db_url = swap_database(&url, &db_name);
                let db_path = std::path::PathBuf::from(&db_url);
                (db_url, db_path)
            }
            Err(_) => {
                let db_path = dir.path().join("test.sqlite");
                (db_path.to_string_lossy().into_owned(), db_path.clone())
            }
        };
        let state = thrum_server::state::AppState::connect_database(&db_url, cfg)
            .await
            .expect("test database");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn({
            let state = state.clone();
            async move {
                thrum_server::serve(state, listener)
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

/// Replace the database name in a connection URL.
fn swap_database(url: &str, database: &str) -> String {
    let (scheme, rest) = url.split_once("://").expect("url scheme");
    let (hostpart, query) = match rest.split_once('?') {
        Some((host, query)) => (host, Some(query)),
        None => (rest, None),
    };
    let host = hostpart.split('/').next().unwrap_or(hostpart);
    match query {
        Some(query) => format!("{scheme}://{host}/{database}?{query}"),
        None => format!("{scheme}://{host}/{database}"),
    }
}
