//! Chat orchestration: ties the HTTP API, the WebSocket relay and the MLS
//! layer together for the CLI commands.

use std::path::{Path, PathBuf};

use thrum_core::envelope::{AppMessage, Envelope, decode_envelope, encode_envelope};
use thrum_core::mls::{DeviceCrypto, Incoming};
use thrum_core::protocol::encode_payload;

use crate::api::Client;
use crate::session::Session;

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
    /// current device of every member.
    pub async fn create_conversation(&self, members: &[String]) -> Result<String, String> {
        let (conversation_id, all_members) = self
            .client
            .create_conversation_with_members(&self.session.token, members)
            .await?;
        self.crypto
            .create_group(&conversation_id)
            .map_err(|error| error.to_string())?;
        self.persist()?;
        self.sync_group(&conversation_id, &all_members).await?;
        Ok(conversation_id)
    }

    /// Add members to a conversation: server membership first, then MLS adds
    /// with commits and welcomes.
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
    /// delivering to them.
    pub async fn remove_member(&self, conversation_id: &str, username: &str) -> Result<(), String> {
        let commit = self.remove_member_mls(conversation_id, username).await?;
        self.client
            .remove_member(&self.session.token, conversation_id, username)
            .await?;
        let _ = commit;
        Ok(())
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
                            "server returned a key package for {claimed} \
                             instead of {identity}"
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
            |conversation, sender, payload| self.interpret(conversation, sender, payload),
            |_| {},
        )
        .await
    }

    /// Encrypt and send a text message; returns the relay message id.
    pub async fn send_text(&self, conversation_id: &str, text: &str) -> Result<String, String> {
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
        let app = serde_json::to_vec(&AppMessage::Text {
            text: text.to_string(),
        })
        .expect("app message serializes");
        let wire = self
            .crypto
            .encrypt(conversation_id, &app)
            .map_err(|error| error.to_string())?;
        self.persist()?;
        let payload = encode_envelope(&Envelope::mls(&wire));
        self.client
            .send_message(&self.session.token, conversation_id, &payload)
            .await
    }

    /// Turn one relay payload into a display line, applying the MLS state
    /// changes it carries. `None` means nothing to show.
    pub fn interpret(
        &self,
        conversation_id: &str,
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
                    match serde_json::from_slice::<AppMessage>(&payload) {
                        Ok(AppMessage::Text { text }) => {
                            self.persist().ok()?;
                            Some(format!("{sender_username}: {text}"))
                        }
                        Err(_) => None,
                    }
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
}

/// The MLS state directory for a state file: `<state-file>.mls/`.
fn state_file_mls_dir(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".mls");
    PathBuf::from(name)
}

/// Encode payload bytes for tests and debugging.
pub fn payload_to_base64(bytes: &[u8]) -> String {
    encode_payload(bytes)
}
