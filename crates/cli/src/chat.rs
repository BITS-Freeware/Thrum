//! Chat orchestration: ties the HTTP API, the WebSocket relay and the MLS
//! layer together for the CLI commands.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thrum_core::envelope::{AppMessage, Envelope, decode_envelope, encode_envelope};
use thrum_core::mls::{DeviceCrypto, Incoming};

use crate::api::Client;
use crate::session::Session;
use crate::store::Store;

/// Keep at least this many key packages on the server.
const KEY_PACKAGE_MIN: i64 = 4;
/// Keep at most this many key packages on the server.
const KEY_PACKAGE_STOCK: i64 = 8;

/// Everything one logged-in device needs to chat.
pub struct Chat {
    client: Client,
    pub session: Session,
    crypto: DeviceCrypto,
    mls_dir: PathBuf,
    /// The device's view of what has been said; survives restarts.
    store: Mutex<Store>,
}

impl Chat {
    /// Open the MLS state for this session, creating fresh key material the
    /// first time. The state lives next to the session state file.
    pub fn open(client: Client, session: Session, state_file: &Path) -> Result<Self, String> {
        let mls_dir = state_file_mls_dir(state_file);
        let crypto = if mls_dir.join("identity.json").exists() {
            DeviceCrypto::load(&mls_dir).map_err(|error| error.to_string())?
        } else {
            let crypto =
                DeviceCrypto::generate(format!("{}#{}", session.username, session.device_id));
            crypto.save(&mls_dir).map_err(|error| error.to_string())?;
            crypto
        };
        Ok(Self {
            client,
            session,
            crypto,
            mls_dir,
            store: Mutex::new(Store::load(state_file)),
        })
    }

    /// The device's MLS identity (`username#device-id`).
    pub fn identity(&self) -> &str {
        self.crypto.identity()
    }

    /// Persist the MLS state after any group operation.
    fn persist(&self) -> Result<(), String> {
        self.crypto
            .save(&self.mls_dir)
            .map_err(|error| error.to_string())
    }

    /// Upload key packages until the server holds a healthy stock.
    pub async fn top_up_key_packages(&self) -> Result<(), String> {
        let count = self.client.key_package_count(&self.session.token).await?;
        if count >= KEY_PACKAGE_MIN {
            return Ok(());
        }
        for _ in count..KEY_PACKAGE_STOCK {
            let package = self
                .crypto
                .new_key_package()
                .map_err(|error| error.to_string())?;
            self.client
                .upload_key_package(&self.session.token, &package)
                .await?;
        }
        // Each generated package carries the private keys that will later
        // unwrap a Welcome; they must reach disk before the process ends.
        self.persist()
    }

    /// Create a conversation on the server and its MLS group, inviting every
    /// current device of every member. The caller becomes its admin.
    pub async fn create_conversation(
        &self,
        members: &[String],
        name: &str,
    ) -> Result<String, String> {
        let (conversation_id, all_members) = self
            .client
            .create_conversation_with_members(&self.session.token, members, name)
            .await?;
        self.crypto
            .create_group(&conversation_id)
            .map_err(|error| error.to_string())?;
        self.persist()?;
        self.sync_group(&conversation_id, &all_members).await?;
        Ok(conversation_id)
    }

    /// Add members to a conversation: server membership first, then MLS adds
    /// with commits and welcomes. Admin only, enforced by the server.
    pub async fn add_members(
        &self,
        conversation_id: &str,
        members: &[String],
    ) -> Result<(), String> {
        let all_members = self
            .client
            .add_members(&self.session.token, conversation_id, members)
            .await?;
        self.sync_group(conversation_id, &all_members).await?;
        Ok(())
    }

    /// Remove a member: the MLS removal commit goes out first, so the removed
    /// member still receives it and loses access, then the server stops
    /// delivering to them. Admin only, enforced by the server.
    pub async fn remove_member(&self, conversation_id: &str, username: &str) -> Result<(), String> {
        let commit = self.remove_member_mls(conversation_id, username).await?;
        self.client
            .remove_member(&self.session.token, conversation_id, username)
            .await?;
        let _ = commit;
        Ok(())
    }

    /// Rename a conversation; admin only.
    pub async fn rename(&self, conversation_id: &str, name: &str) -> Result<(), String> {
        self.client
            .rename_conversation(&self.session.token, conversation_id, name)
            .await
    }

    /// Mint an invite token for a conversation; admin only.
    pub async fn create_invite(&self, conversation_id: &str) -> Result<String, String> {
        self.client
            .create_invite(&self.session.token, conversation_id)
            .await
    }

    /// Revoke an invite token; admin only.
    pub async fn revoke_invite(
        &self,
        conversation_id: &str,
        invite_token: &str,
    ) -> Result<(), String> {
        self.client
            .revoke_invite(&self.session.token, conversation_id, invite_token)
            .await
    }

    /// Redeem an invite token: join on the server. The MLS group follows
    /// when a member's device runs `sync` and sends this device a welcome.
    pub async fn join(&self, invite_token: &str) -> Result<String, String> {
        let (conversation_id, _members) = self
            .client
            .redeem_invite(&self.session.token, invite_token)
            .await?;
        Ok(conversation_id)
    }

    /// Block a user.
    pub async fn block(&self, username: &str) -> Result<(), String> {
        self.client.block(&self.session.token, username).await
    }

    /// Lift a block.
    pub async fn unblock(&self, username: &str) -> Result<(), String> {
        self.client.unblock(&self.session.token, username).await
    }

    /// The MLS half of a removal: build the commit, send it, merge locally.
    async fn remove_member_mls(
        &self,
        conversation_id: &str,
        username: &str,
    ) -> Result<Vec<u8>, String> {
        let roster = self
            .crypto
            .group_members(conversation_id)
            .map_err(|error| error.to_string())?;
        let targets: Vec<String> = roster
            .iter()
            .filter(|identity| {
                thrum_core::mls::split_identity(identity)
                    .is_some_and(|(member, _)| member == username)
            })
            .cloned()
            .collect();
        if targets.is_empty() {
            return Err(format!("{username} has no devices in this group"));
        }
        let mut commit = Vec::new();
        for target in targets {
            commit = self
                .crypto
                .remove_member(conversation_id, &target)
                .map_err(|error| error.to_string())?;
            self.persist()?;
            let payload = encode_envelope(&Envelope::mls(&commit));
            self.client
                .send_message(&self.session.token, conversation_id, &payload)
                .await?;
        }
        Ok(commit)
    }

    /// Bring one conversation's MLS group in line with the server's member
    /// list: every device of every member that is missing from the group gets
    /// added with one commit and one welcome.
    async fn sync_group(&self, conversation_id: &str, members: &[String]) -> Result<(), String> {
        let roster = self
            .crypto
            .group_members(conversation_id)
            .map_err(|error| error.to_string())?;
        let mut packages = Vec::new();
        for member in members {
            for device_id in self
                .client
                .devices_with_key_packages(&self.session.token, member)
                .await?
            {
                let identity = format!("{member}#{device_id}");
                if roster.contains(&identity) || identity == self.identity() {
                    continue;
                }
                if let Some(package) = self
                    .client
                    .fetch_key_package(&self.session.token, &device_id)
                    .await?
                {
                    // The server picks which bytes to hand out; the claimed
                    // identity must be the device we asked about.
                    let claimed = self
                        .crypto
                        .key_package_identity(&package)
                        .map_err(|error| error.to_string())?;
                    if claimed != identity {
                        return Err(format!(
                            "server returned a key package for {claimed} instead of {identity}"
                        ));
                    }
                    packages.push(package);
                }
            }
        }
        if packages.is_empty() {
            return Ok(());
        }
        let (commit, welcome) = self
            .crypto
            .add_members(conversation_id, &packages)
            .map_err(|error| error.to_string())?;
        self.persist()?;
        if let Some(welcome) = welcome {
            let payload = encode_envelope(&Envelope::welcome(&welcome));
            self.client
                .send_message(&self.session.token, conversation_id, &payload)
                .await?;
        }
        let payload = encode_envelope(&Envelope::mls(&commit));
        self.client
            .send_message(&self.session.token, conversation_id, &payload)
            .await?;
        Ok(())
    }

    /// Bring every conversation in line with the server: add missing devices
    /// of all members, and top up this device's key package stock.
    pub async fn sync(&self) -> Result<(), String> {
        self.top_up_key_packages().await?;
        for (conversation_id, members) in
            self.client.list_conversations(&self.session.token).await?
        {
            if self
                .crypto
                .has_group(&conversation_id)
                .map_err(|error| error.to_string())?
            {
                self.sync_group(&conversation_id, &members).await?;
            }
        }
        Ok(())
    }

    /// Process everything queued for this device before doing something
    /// that needs a current group state (sending, removing). Short timeout:
    /// the queue drains immediately after the WebSocket handshake.
    pub async fn catch_up(&self) -> Result<(), String> {
        crate::ws::recv(
            &self.session,
            &crate::ws::RecvOptions {
                once: false,
                timeout_secs: 1,
            },
            |conversation, message_id, sender, payload| {
                self.interpret(conversation, message_id, sender, payload)
            },
            |_| {},
        )
        .await
    }

    /// Encrypt and send any application message; returns the relay message id.
    async fn send_app(&self, conversation_id: &str, app: &AppMessage) -> Result<String, String> {
        if !self
            .crypto
            .has_group(conversation_id)
            .map_err(|error| error.to_string())?
        {
            return Err("this device has not joined this conversation's group yet; \
                 run `thrum recv` once to catch up"
                .to_string());
        }
        self.catch_up().await?;
        let app_bytes = serde_json::to_vec(app).expect("app message serializes");
        let wire = self
            .crypto
            .encrypt(conversation_id, &app_bytes)
            .map_err(|error| error.to_string())?;
        self.persist()?;
        let payload = encode_envelope(&Envelope::mls(&wire));
        let id = self
            .client
            .send_message(&self.session.token, conversation_id, &payload)
            .await?;
        // Remember what this device said, so `edit` and `delete` work at
        // once and read receipts skip own messages.
        let mut store = self.store.lock().expect("store lock");
        if let AppMessage::Text { text } | AppMessage::Reply { text, .. } = app
            && store.entry(conversation_id, &id).is_none()
        {
            store.record(conversation_id, &id, &self.session.username, text);
        }
        Ok(id)
    }

    /// Send a chat text.
    pub async fn send_text(&self, conversation_id: &str, text: &str) -> Result<String, String> {
        self.send_app(
            conversation_id,
            &AppMessage::Text {
                text: text.to_string(),
            },
        )
        .await
    }

    /// Send a reply quoting an earlier message.
    pub async fn send_reply(
        &self,
        conversation_id: &str,
        reply_to: &str,
        text: &str,
    ) -> Result<String, String> {
        self.send_app(
            conversation_id,
            &AppMessage::Reply {
                reply_to: reply_to.to_string(),
                text: text.to_string(),
            },
        )
        .await
    }

    /// Edit a message this device sent.
    pub async fn send_edit(
        &self,
        conversation_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<String, String> {
        if !self.is_own_message(conversation_id, message_id) {
            return Err(format!("message {message_id} is not yours to edit"));
        }
        self.send_app(
            conversation_id,
            &AppMessage::Edit {
                message_id: message_id.to_string(),
                text: text.to_string(),
            },
        )
        .await
    }

    /// Delete a message this device sent.
    pub async fn send_delete(
        &self,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<String, String> {
        if !self.is_own_message(conversation_id, message_id) {
            return Err(format!("message {message_id} is not yours to delete"));
        }
        self.send_app(
            conversation_id,
            &AppMessage::Delete {
                message_id: message_id.to_string(),
            },
        )
        .await
    }

    /// Send read receipts for everything this device has not marked read.
    pub async fn send_read(&self, conversation_id: &str) -> Result<usize, String> {
        let unread = self
            .store
            .lock()
            .expect("store lock")
            .unread_for(conversation_id, &self.session.username);
        if unread.is_empty() {
            return Ok(0);
        }
        let count = unread.len();
        self.send_app(
            conversation_id,
            &AppMessage::Read {
                message_ids: unread,
            },
        )
        .await?;
        Ok(count)
    }

    /// Whether the local store knows this message as one this user sent.
    fn is_own_message(&self, conversation_id: &str, message_id: &str) -> bool {
        self.store
            .lock()
            .expect("store lock")
            .entry(conversation_id, message_id)
            .is_some_and(|entry| entry.sender == self.session.username)
    }

    /// Find a message anywhere in the local store: its conversation id and a
    /// snapshot of its entry.
    pub fn find_message(&self, message_id: &str) -> Option<(String, crate::store::Entry)> {
        self.store
            .lock()
            .expect("store lock")
            .find(message_id)
            .map(|(conversation, entry)| (conversation, entry.clone()))
    }

    /// Turn one relay payload into a display line, applying the MLS state
    /// changes it carries. `None` means nothing to show.
    pub fn interpret(
        &self,
        conversation_id: &str,
        message_id: &str,
        sender_username: &str,
        payload: &[u8],
    ) -> Option<String> {
        let envelope = decode_envelope(payload)?;
        let data = envelope.data()?;
        match envelope {
            Envelope::Welcome { .. } => match self.crypto.process_welcome(&data) {
                Ok(group_conversation) => {
                    // The new group state must reach disk before the process
                    // ends, or the next run knows nothing about the join.
                    if let Err(error) = self.persist() {
                        eprintln!("warning: could not save MLS state: {error}");
                    }
                    Some(format!("joined {group_conversation}"))
                }
                Err(error) => {
                    eprintln!("welcome not applied: {error}"); // stale or foreign
                    None
                }
            },
            Envelope::Mls { .. } => match self.crypto.process_incoming(conversation_id, &data) {
                Ok(Incoming::Application { payload, .. }) => {
                    self.apply_app_message(conversation_id, message_id, sender_username, &payload)
                }
                Ok(Incoming::Commit) => {
                    self.persist().ok()?;
                    None
                }
                Ok(Incoming::Ignored) => None,
                Err(error) => {
                    eprintln!("message not applied: {error}"); // not for this device
                    None
                }
            },
        }
    }

    /// Apply a decrypted application message to the store and produce the
    /// line to display.
    fn apply_app_message(
        &self,
        conversation_id: &str,
        message_id: &str,
        sender: &str,
        payload: &[u8],
    ) -> Option<String> {
        let message: AppMessage = match serde_json::from_slice(payload) {
            Ok(message) => message,
            Err(_) => return None,
        };
        self.persist().ok()?;
        let mut store = self.store.lock().expect("store lock");
        match message {
            AppMessage::Text { text } => {
                if store.entry(conversation_id, message_id).is_none() {
                    store.record(conversation_id, message_id, sender, &text);
                }
                Some(format!("{sender}: {text}"))
            }
            AppMessage::Reply { reply_to, text } => {
                if store.entry(conversation_id, message_id).is_none() {
                    store.record(conversation_id, message_id, sender, &text);
                }
                let short = &reply_to[..reply_to.len().min(8)];
                Some(format!("{sender}: [re:{short}] {text}"))
            }
            AppMessage::Edit {
                message_id: target,
                text,
            } => {
                if store.edit(conversation_id, &target, sender, &text) {
                    Some(format!("{sender} edited {target}"))
                } else {
                    None
                }
            }
            AppMessage::Delete { message_id: target } => {
                if store.delete(conversation_id, &target, sender) {
                    Some(format!("{sender} deleted {target}"))
                } else {
                    None
                }
            }
            AppMessage::Read { message_ids } => {
                let count = message_ids.len();
                store.mark_read(conversation_id, sender, &message_ids);
                Some(format!("{sender} read {count} message(s)"))
            }
        }
    }
}

/// The MLS state directory for a state file: `<state-file>.mls/`.
fn state_file_mls_dir(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".mls");
    PathBuf::from(name)
}

/// Encode payload bytes for tests and debugging.
pub fn payload_to_base64(bytes: &[u8]) -> String {
    thrum_core::protocol::encode_payload(bytes)
}
