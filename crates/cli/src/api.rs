//! HTTP calls to the Thrum server.

use serde_json::Value;

use crate::session::Session;

/// A thin client over the server's HTTPS JSON API.
#[derive(Debug, Clone)]
pub struct Client {
    base: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base: String) -> Self {
        Self {
            base,
            http: reqwest::Client::new(),
        }
    }

    async fn checked(response: reqwest::Response) -> Result<reqwest::Response, String> {
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else {
            let code = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|body| body["error"].as_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            Err(format!("server said {}: {code}", status.as_u16()))
        }
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<reqwest::Response, String> {
        self.http
            .post(format!("{}{}", self.base, path))
            .json(body)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))
    }

    /// Create an account.
    pub async fn register(&self, username: &str, password: &str) -> Result<(), String> {
        let response = self
            .post_json(
                "/v1/accounts",
                &serde_json::json!({ "username": username, "password": password }),
            )
            .await?;
        Self::checked(response).await.map(|_| ())
    }

    /// Log in, creating a new device session.
    pub async fn login(
        &self,
        username: &str,
        password: &str,
        label: &str,
    ) -> Result<Session, String> {
        let response = self
            .post_json(
                "/v1/sessions",
                &serde_json::json!({
                    "username": username,
                    "password": password,
                    "device_label": label,
                }),
            )
            .await?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad session response: {error}"))?;
        Ok(Session {
            server: self.base.clone(),
            username: body["username"].as_str().unwrap_or(username).to_string(),
            token: body["token"]
                .as_str()
                .ok_or("server sent no token")?
                .to_string(),
            device_id: body["device_id"]
                .as_str()
                .ok_or("server sent no device id")?
                .to_string(),
        })
    }

    /// Create a conversation with the given member usernames.
    pub async fn create_conversation(
        &self,
        token: &str,
        members: &[String],
    ) -> Result<String, String> {
        let response = self
            .http
            .post(format!("{}/v1/conversations", self.base))
            .bearer_auth(token)
            .json(&serde_json::json!({ "members": members }))
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad conversation response: {error}"))?;
        body["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "server sent no conversation id".to_string())
    }

    /// Send an opaque payload to a conversation; returns the message id.
    pub async fn send_message(
        &self,
        token: &str,
        conversation_id: &str,
        payload: &[u8],
    ) -> Result<String, String> {
        let response = self
            .http
            .post(format!(
                "{}/v1/conversations/{conversation_id}/messages",
                self.base
            ))
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(payload.to_vec())
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad send response: {error}"))?;
        body["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "server sent no message id".to_string())
    }
}
