//! HTTP calls to the Thrum server.

use serde_json::Value;

use crate::session::Session;

/// One-time key-package stock plus whether a last-resort package is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPackageStock {
    pub count: i64,
    pub last_resort: bool,
}

/// A thin client over the server's HTTPS JSON API.
#[derive(Debug, Clone)]
pub struct Client {
    base: String,
    http: reqwest::Client,
}

impl Client {
    /// The URL of an API route. Each segment is percent-encoded on its
    /// own, so an id from a peer or the server cannot add path segments,
    /// and the segments `.` and `..` are refused rather than resolved.
    fn endpoint(&self, segments: &[&str]) -> Result<url::Url, String> {
        if segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
        {
            return Err("invalid id in request path".to_string());
        }
        let mut url =
            url::Url::parse(&self.base).map_err(|error| format!("bad server URL: {error}"))?;
        url.path_segments_mut()
            .map_err(|_| "bad server URL".to_string())?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

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
            .post(self.endpoint(&["v1", "conversations"])?)
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
            .post(self.endpoint(&["v1", "conversations"])?)
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
            .get(self.endpoint(&["v1", "conversations"])?)
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
            .post(self.endpoint(&["v1", "conversations", conversation_id, "members"])?)
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
            .delete(self.endpoint(&[
                "v1",
                "conversations",
                conversation_id,
                "members",
                username,
            ])?)
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
            .get(self.endpoint(&["v1", "users", username, "key-package-devices"])?)
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
            .get(self.endpoint(&["v1", "devices", device_id, "key-package"])?)
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
            .post(self.endpoint(&["v1", "key-packages"])?)
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// How many one-time key packages the caller's device still has, and
    /// whether a last-resort package is stored.
    pub async fn key_package_stock(&self, token: &str) -> Result<KeyPackageStock, String> {
        let response = self
            .http
            .get(self.endpoint(&["v1", "key-packages", "count"])?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad count response: {error}"))?;
        let count = body["count"]
            .as_i64()
            .ok_or_else(|| "server sent no count".to_string())?;
        Ok(KeyPackageStock {
            count,
            last_resort: body["last_resort"].as_bool().unwrap_or(false),
        })
    }

    /// The key packages the server holds for the caller's device, as
    /// `(id, content)`, the last-resort one included.
    pub async fn own_key_packages(&self, token: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
        use base64::Engine as _;
        let response = self
            .http
            .get(self.endpoint(&["v1", "key-packages"])?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad key package list: {error}"))?;
        let packages = body["packages"]
            .as_array()
            .ok_or_else(|| "server sent no key package list".to_string())?;
        packages
            .iter()
            .map(|package| {
                let id = package["id"]
                    .as_str()
                    .ok_or_else(|| "key package without an id".to_string())?;
                let content = base64::engine::general_purpose::STANDARD
                    .decode(package["content"].as_str().unwrap_or_default())
                    .map_err(|error| format!("bad key package content: {error}"))?;
                Ok((id.to_string(), content))
            })
            .collect()
    }

    /// Withdraw one of the caller's device's stored key packages. A package
    /// that is already gone (someone fetched it meanwhile) is not an error.
    pub async fn delete_key_package(&self, token: &str, id: &str) -> Result<(), String> {
        let response = self
            .http
            .delete(self.endpoint(&["v1", "key-packages", id])?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Self::checked(response).await.map(|_| ())
    }

    /// Upload the device's last-resort key package. The server keeps it and
    /// serves it again after the one-time stock is empty.
    pub async fn upload_last_resort_key_package(
        &self,
        token: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let response = self
            .http
            .post(self.endpoint(&["v1", "key-packages"])?)
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .header("x-thrum-key-package-role", "last-resort")
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// Device ids of a user that are still signed in, whether or not they
    /// have a key package left.
    pub async fn active_devices(&self, token: &str, username: &str) -> Result<Vec<String>, String> {
        Ok(self
            .active_devices_with_keys(token, username)
            .await?
            .into_iter()
            .map(|(device_id, _)| device_id)
            .collect())
    }

    /// A user's signed-in devices, each with the signature key the server
    /// bound to it (`None` before the device's first key package).
    pub async fn active_devices_with_keys(
        &self,
        token: &str,
        username: &str,
    ) -> Result<Vec<(String, Option<Vec<u8>>)>, String> {
        use base64::Engine as _;
        let response = self
            .http
            .get(self.endpoint(&["v1", "users", username, "devices"])?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad device list: {error}"))?;
        let keys = &body["signature_keys"];
        Ok(body["devices"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|device| device.as_str().map(str::to_string))
                    .map(|device_id| {
                        let key = keys[&device_id].as_str().and_then(|encoded| {
                            base64::engine::general_purpose::STANDARD
                                .decode(encoded)
                                .ok()
                        });
                        (device_id, key)
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Revoke one of the caller's devices.
    pub async fn revoke_device(&self, token: &str, device_id: &str) -> Result<(), String> {
        let response = self
            .http
            .delete(self.endpoint(&["v1", "devices", device_id])?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }

    /// Send an opaque payload to a conversation; returns the message id.
    /// Send an MLS commit built in `epoch`. The server takes one commit per
    /// epoch: `Ok(Some(id))` when it took this one (or had taken it before,
    /// for a retry), `Ok(None)` when another commit took the epoch first.
    pub async fn send_commit(
        &self,
        token: &str,
        conversation_id: &str,
        payload: &[u8],
        epoch: u64,
    ) -> Result<Option<String>, String> {
        let mut url = self.endpoint(&["v1", "conversations", conversation_id, "messages"])?;
        url.query_pairs_mut()
            .append_pair("commit_epoch", &epoch.to_string());
        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .body(payload.to_vec())
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        if response.status() == reqwest::StatusCode::CONFLICT {
            return Ok(None);
        }
        let response = Self::checked(response).await?;
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("bad send response: {error}"))?;
        body["id"]
            .as_str()
            .map(|id| Some(id.to_string()))
            .ok_or_else(|| "server sent no message id".to_string())
    }

    pub async fn send_message(
        &self,
        token: &str,
        conversation_id: &str,
        payload: &[u8],
    ) -> Result<String, String> {
        let response = self
            .http
            .post(self.endpoint(&["v1", "conversations", conversation_id, "messages"])?)
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
            .patch(self.endpoint(&["v1", "conversations", conversation_id, "name"])?)
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
            .post(self.endpoint(&["v1", "conversations", conversation_id, "invites"])?)
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
            .post(self.endpoint(&["v1", "conversations", conversation_id, "invites", "revoke"])?)
            .bearer_auth(token)
            .json(&serde_json::json!({ "token": invite_token }))
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
            .post(self.endpoint(&["v1", "invites", "redeem"])?)
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
            .post(self.endpoint(&["v1", "blocks"])?)
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
            .delete(self.endpoint(&["v1", "blocks", username])?)
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
            .post(self.endpoint(&["v1", "blobs"])?)
            .bearer_auth(token)
            .header("content-type", "application/octet-stream")
            .header("x-thrum-blob-id", blob_id)
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
            .get(self.endpoint(&["v1", "blobs", blob_id])?)
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

    /// Download a JSON export of everything the server knows about the caller.
    pub async fn export_account(&self, token: &str) -> Result<serde_json::Value, String> {
        let response = self
            .http
            .get(self.endpoint(&["v1", "account", "export"])?)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        let response = Self::checked(response).await?;
        response
            .json()
            .await
            .map_err(|error| format!("bad export response: {error}"))
    }

    /// Permanently delete the account; requires the account password.
    pub async fn delete_account(&self, token: &str, password: &str) -> Result<(), String> {
        let response = self
            .http
            .delete(self.endpoint(&["v1", "account"])?)
            .bearer_auth(token)
            .json(&serde_json::json!({ "password": password }))
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        Self::checked(response).await.map(|_| ())
    }
}
