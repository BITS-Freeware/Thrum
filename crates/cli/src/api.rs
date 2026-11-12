//! HTTP calls to the Geist3 server.

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

    /// Create a conversation; returns the id and the full member list.
    pub async fn create_conversation_with_members(
        &self,
        token: &str,
        members: &[String],
        name: &str,
    ) -> Result<(String, Vec<String>), String> {
        let response = self
            .http
            .post(format!("{}/v1/conversations", self.base))
            .bearer_auth(token)
            .json(&serde_json::json!({ "members": members, "name": name }))
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad conversation response: {error}"))?;
        let id = body["id"]
            .as_str()
            .ok_or("server sent no conversation id")?
            .to_string();
        let members = body["members"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|member| member.as_str().map(str::to_string))
                    .collect()
            })
            .ok_or("server sent no member list")?;
        Ok((id, members))
    }

    /// List the caller's conversations as (id, members).
    pub async fn list_conversations(
        &self,
        token: &str,
    ) -> Result<Vec<(String, Vec<String>)>, String> {
        let response = self
            .http
            .get(format!("{}/v1/conversations", self.base))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad conversation list: {error}"))?;
        Ok(body
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|entry| {
                        let id = entry["id"].as_str()?.to_string();
                        let members = entry["members"]
                            .as_array()?
                            .iter()
                            .filter_map(|member| member.as_str().map(str::to_string))
                            .collect();
                        Some((id, members))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Add members to a conversation; returns the new member list.
    pub async fn add_members(
        &self,
        token: &str,
        conversation_id: &str,
        members: &[String],
    ) -> Result<Vec<String>, String> {
        let response = self
            .http
            .post(format!(
                "{}/v1/conversations/{conversation_id}/members",
                self.base
            ))
            .bearer_auth(token)
            .json(&serde_json::json!({ "members": members }))
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad member response: {error}"))?;
        Ok(body["members"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|member| member.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Remove a member from a conversation.
    pub async fn remove_member(
        &self,
        token: &str,
        conversation_id: &str,
        username: &str,
    ) -> Result<(), String> {
        let response = self
            .http
            .delete(format!(
                "{}/v1/conversations/{conversation_id}/members/{username}",
                self.base
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// Device ids of a user that currently have key packages to give out.
    pub async fn devices_with_key_packages(
        &self,
        token: &str,
        username: &str,
    ) -> Result<Vec<String>, String> {
        let response = self
            .http
            .get(format!(
                "{}/v1/users/{username}/key-package-devices",
                self.base
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad device list: {error}"))?;
        Ok(body["devices"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|device| device.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Fetch and consume one key package of a device; `None` when the device
    /// has none left.
    pub async fn fetch_key_package(
        &self,
        token: &str,
        device_id: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        let response = self
            .http
            .get(format!("{}/v1/devices/{device_id}/key-package", self.base))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = Self::checked(response).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("bad key package: {error}"))?;
        Ok(Some(bytes.to_vec()))
    }

    /// Upload one key package for the caller's device.
    pub async fn upload_key_package(&self, token: &str, bytes: &[u8]) -> Result<(), String> {
        let response = self
            .http
            .post(format!("{}/v1/key-packages", self.base))
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// How many key packages the caller's device still has in stock.
    pub async fn key_package_count(&self, token: &str) -> Result<i64, String> {
        let response = self
            .http
            .get(format!("{}/v1/key-packages/count", self.base))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad count response: {error}"))?;
        body["count"]
            .as_i64()
            .ok_or_else(|| "server sent no count".to_string())
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

    /// Rename a conversation; admin only.
    pub async fn rename_conversation(
        &self,
        token: &str,
        conversation_id: &str,
        name: &str,
    ) -> Result<(), String> {
        let response = self
            .http
            .patch(format!(
                "{}/v1/conversations/{conversation_id}/name",
                self.base
            ))
            .bearer_auth(token)
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// Mint an invite token for a conversation; admin only.
    pub async fn create_invite(
        &self,
        token: &str,
        conversation_id: &str,
    ) -> Result<String, String> {
        let response = self
            .http
            .post(format!(
                "{}/v1/conversations/{conversation_id}/invites",
                self.base
            ))
            .bearer_auth(token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad invite response: {error}"))?;
        body["token"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "server sent no token".to_string())
    }

    /// Revoke an invite token; admin only.
    pub async fn revoke_invite(
        &self,
        token: &str,
        conversation_id: &str,
        invite_token: &str,
    ) -> Result<(), String> {
        let response = self
            .http
            .delete(format!(
                "{}/v1/conversations/{conversation_id}/invites/{invite_token}",
                self.base
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// Redeem an invite token and join the conversation.
    pub async fn redeem_invite(
        &self,
        token: &str,
        invite_token: &str,
    ) -> Result<(String, Vec<String>), String> {
        let response = self
            .http
            .post(format!("{}/v1/invites/redeem", self.base))
            .bearer_auth(token)
            .json(&serde_json::json!({ "token": invite_token }))
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad invite response: {error}"))?;
        let id = body["id"]
            .as_str()
            .ok_or("server sent no conversation id")?
            .to_string();
        let members = body["members"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|member| member.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok((id, members))
    }

    /// Block a user.
    pub async fn block(&self, token: &str, username: &str) -> Result<(), String> {
        let response = self
            .http
            .post(format!("{}/v1/blocks", self.base))
            .bearer_auth(token)
            .json(&serde_json::json!({ "username": username }))
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// Lift a block.
    pub async fn unblock(&self, token: &str, username: &str) -> Result<(), String> {
        let response = self
            .http
            .delete(format!("{}/v1/blocks/{username}", self.base))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// Upload an encrypted attachment blob under a client-chosen id.
    pub async fn upload_blob(
        &self,
        token: &str,
        blob_id: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let response = self
            .http
            .post(format!("{}/v1/blobs", self.base))
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .header("x-geist3-blob-id", blob_id)
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// Fetch an encrypted attachment blob; `None` when it is gone (expired).
    pub async fn fetch_blob(&self, token: &str, blob_id: &str) -> Result<Option<Vec<u8>>, String> {
        let response = self
            .http
            .get(format!("{}/v1/blobs/{blob_id}", self.base))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = Self::checked(response).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("bad blob response: {error}"))?;
        Ok(Some(bytes.to_vec()))
    }
}
