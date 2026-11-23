//! Chat orchestration: ties the HTTP API, the WebSocket relay and the MLS
//! layer together for the CLI commands.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rand::RngCore;
use sha2::{Digest, Sha256};
use thrum_core::attachment::{FileKey, KEY_LEN, NONCE_LEN};
use thrum_core::envelope::{AppMessage, Envelope, decode_envelope, encode_envelope};
use thrum_core::membership::ServerRoster;
use thrum_core::mls::{DeviceCrypto, Incoming, split_identity};
use thrum_core::roster::{RosterAction, departed_members, plan_roster};

use crate::api::Client;
use crate::lock::DeviceLock;
use crate::session::Session;
use crate::store::{AttachmentMeta, Store};

/// Keep at least this many key packages on the server.
const KEY_PACKAGE_MIN: i64 = 4;
/// Keep at most this many key packages on the server.
const KEY_PACKAGE_STOCK: i64 = 8;

/// What a sync changed in one or more conversations.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub pending: Vec<String>,
}

impl SyncReport {
    fn push(&mut self, conversation_id: &str, kind: &str, identity: &str) {
        let line = format!("{kind} {identity} in {conversation_id}");
        match kind {
            "added" => self.added.push(line),
            "removed" => self.removed.push(line),
            _ => self.pending.push(line),
        }
    }

    fn log(&self) {
        for line in self.removed.iter().chain(&self.added).chain(&self.pending) {
            crate::errln!("{line}");
        }
    }
}

/// Everything one logged-in device needs to chat.
pub struct Chat {
    client: Client,
    pub session: Session,
    crypto: DeviceCrypto,
    mls_dir: PathBuf,
    /// Where received attachments are saved by default.
    attachments_dir: PathBuf,
    /// The device's view of what has been said; survives restarts.
    store: Mutex<Store>,
    /// The server's view of each conversation, used to check welcomes and
    /// commits; refreshed when a check fails.
    rosters: Mutex<HashMap<String, ServerRoster>>,
    /// Held while this process uses the device, so no other one does.
    _lock: DeviceLock,
}

impl Chat {
    /// Open the MLS state for this session, creating fresh key material the
    /// first time. The state lives next to the session state file.
    ///
    /// Waits for any other process that has the same device open, so one
    /// device never runs twice at once.
    pub fn open(client: Client, session: Session, state_file: &Path) -> Result<Self, String> {
        let lock = DeviceLock::acquire(state_file, crate::lock::WAIT)?;
        let mls_dir = state_file_mls_dir(state_file);
        let crypto = if mls_dir.join("state.json").exists() {
            let crypto = DeviceCrypto::load(&mls_dir).map_err(|error| error.to_string())?;
            let expected = format!("{}#{}", session.username, session.device_id);
            if crypto.identity() != expected {
                // Keys that do not belong to this session's device would
                // otherwise be uploaded under it, and other members would
                // reject the mismatched identity. Failing loudly beats
                // silently corrupt state.
                return Err(format!(
                    "state file holds keys for {} but this session is {}; \
                     remove {} to provision the device anew",
                    crypto.identity(),
                    expected,
                    mls_dir.display()
                ));
            }
            crypto
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
            attachments_dir: state_file_attachments_dir(state_file),
            store: Mutex::new(Store::load(state_file)?),
            rosters: Mutex::new(HashMap::new()),
            _lock: lock,
        })
    }

    /// The device's MLS identity (`username#device-id`).
    pub fn identity(&self) -> &str {
        self.crypto.identity()
    }

    /// The server's view of a conversation: its members, their active
    /// devices, and each device's registered signature key. Cached; pass
    /// `refresh` to fetch it again.
    async fn server_roster(
        &self,
        conversation_id: &str,
        refresh: bool,
    ) -> Result<ServerRoster, String> {
        if !refresh
            && let Some(roster) = self
                .rosters
                .lock()
                .expect("roster lock")
                .get(conversation_id)
        {
            return Ok(roster.clone());
        }
        let conversations = self.client.list_conversations(&self.session.token).await?;
        let mut roster = ServerRoster::new();
        if let Some((_, members)) = conversations.iter().find(|(id, _)| id == conversation_id) {
            for member in members {
                let devices = self
                    .client
                    .active_devices_with_keys(&self.session.token, member)
                    .await?;
                roster.add_account(member, devices);
            }
        }
        self.apply_pins(&mut roster);
        self.rosters
            .lock()
            .expect("roster lock")
            .insert(conversation_id.to_string(), roster.clone());
        Ok(roster)
    }

    /// Hold every listed device to the key this client pinned for it.
    /// Returns the identities whose key the server now reports differently.
    fn apply_pins(&self, roster: &mut ServerRoster) -> Vec<String> {
        let pins = self.store.lock().expect("store lock").pinned_keys();
        let mut changed = Vec::new();
        for (identity, key) in pins {
            if roster.pin(&identity, &key) {
                crate::errln!(
                    "warning: the server reports a new signature key for {identity}; \
                     it is not accepted until you run `thrum trust --device {identity}`"
                );
                changed.push(identity);
            }
        }
        changed
    }

    /// Pin the keys of the conversation's current leaves that have none
    /// yet. Called after this device accepted the group's state.
    fn pin_group_keys(&self, conversation_id: &str) {
        let Ok(leaves) = self.crypto.group_leaves(conversation_id) else {
            return;
        };
        let leaves: Vec<(String, Vec<u8>)> = leaves
            .into_iter()
            .filter(|leaf| leaf.well_formed())
            .map(|leaf| (leaf.identity, leaf.signature_key))
            .collect();
        self.store.lock().expect("store lock").pin_keys(&leaves);
    }

    /// Persist the MLS state after any group operation.
    fn persist(&self) -> Result<(), String> {
        self.crypto
            .save(&self.mls_dir)
            .map_err(|error| error.to_string())
    }

    /// Upload key packages until the server holds a healthy stock, including
    /// one last-resort package that survives a drain.
    ///
    /// Packages the server holds that this device cannot open are withdrawn
    /// first. That happens after the state file is restored from a copy
    /// taken before the last top-up: a Welcome built on such a package
    /// could never be read, so the device would miss the conversation.
    pub async fn top_up_key_packages(&self) -> Result<(), String> {
        let mut withdrawn = 0usize;
        for (id, content) in self.client.own_key_packages(&self.session.token).await? {
            let held = self
                .crypto
                .holds_key_package(&content)
                .map_err(|error| error.to_string())?;
            if !held {
                self.client
                    .delete_key_package(&self.session.token, &id)
                    .await?;
                withdrawn += 1;
            }
        }
        if withdrawn > 0 {
            crate::errln!(
                "note: withdrew {withdrawn} key package(s) this device can no longer open \
                 (its state was restored from an older copy)"
            );
        }
        let stock = self.client.key_package_stock(&self.session.token).await?;
        if !stock.last_resort {
            let package = self
                .crypto
                .new_last_resort_key_package()
                .map_err(|error| error.to_string())?;
            self.client
                .upload_last_resort_key_package(&self.session.token, &package)
                .await?;
        }
        if stock.count < KEY_PACKAGE_MIN {
            for _ in stock.count..KEY_PACKAGE_STOCK {
                let package = self
                    .crypto
                    .new_key_package()
                    .map_err(|error| error.to_string())?;
                self.client
                    .upload_key_package(&self.session.token, &package)
                    .await?;
            }
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
        let admit: HashSet<String> = all_members.iter().cloned().collect();
        let report = self
            .sync_group(&conversation_id, &all_members, &admit)
            .await?;
        report.log();
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
        let admit: HashSet<String> = members.iter().cloned().collect();
        self.catch_up().await?;
        let report = self
            .sync_group(conversation_id, &all_members, &admit)
            .await?;
        report.log();
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
        self.catch_up().await?;
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

    /// Credential identities currently in a conversation's MLS group.
    pub fn group_members(&self, conversation_id: &str) -> Result<Vec<String>, String> {
        self.crypto
            .group_members(conversation_id)
            .map_err(|error| error.to_string())
    }

    /// Devices waiting for this user to approve them.
    pub fn pending_devices(&self) -> Vec<(String, String)> {
        self.store.lock().expect("store lock").pending()
    }

    /// Trust a device identity (`username#device-id`) so the next sync may
    /// add it, with whatever signature key it presents then. Does not add
    /// it by itself.
    pub fn trust_device(&self, identity: &str) -> Result<(), String> {
        if thrum_core::mls::split_identity(identity).is_none() {
            return Err(format!(
                "{identity} is not a device identity (expected username#device-id)"
            ));
        }
        self.store
            .lock()
            .expect("store lock")
            .trust_new_key(identity);
        Ok(())
    }

    /// Bring one conversation's MLS group in line with the server.
    ///
    /// Accounts in `admit` have their current devices added without a prior
    /// trust decision: the user just named them. Every other new device is
    /// recorded as pending. Devices the server no longer lists, and devices
    /// of accounts that left, are removed.
    async fn sync_group(
        &self,
        conversation_id: &str,
        members: &[String],
        admit: &HashSet<String>,
    ) -> Result<SyncReport, String> {
        let roster = self.group_members(conversation_id)?;
        let trusted = self.store.lock().expect("store lock").trusted();
        let mut report = SyncReport::default();
        let mut to_remove = departed_members(self.identity(), &roster, members);
        let mut to_add: Vec<(String, String)> = Vec::new();
        let mut pending = Vec::new();
        let mut newly_trusted = Vec::new();
        let mut newly_known = Vec::new();
        let mut server = ServerRoster::new();
        for member in members {
            let devices = self
                .client
                .active_devices_with_keys(&self.session.token, member)
                .await?;
            let active: Vec<String> = devices.iter().map(|(id, _)| id.clone()).collect();
            server.add_account(member, devices);
            let admit_member = admit.iter().any(|name| name.eq_ignore_ascii_case(member));
            let known = self
                .store
                .lock()
                .expect("store lock")
                .knows_account_anywhere(member);
            for action in plan_roster(
                self.identity(),
                &roster,
                member,
                &active,
                &trusted,
                admit_member,
                known,
            ) {
                match action {
                    RosterAction::Remove(identity) => to_remove.push(identity),
                    RosterAction::Pending(identity) => pending.push(identity),
                    RosterAction::Add(identity) => {
                        let device_id = identity
                            .split_once('#')
                            .map(|(_, device_id)| device_id.to_string())
                            .unwrap_or_default();
                        newly_trusted.push(identity.clone());
                        to_add.push((identity, device_id));
                    }
                }
            }
            let in_group = roster.iter().any(|identity| {
                thrum_core::mls::split_identity(identity)
                    .is_some_and(|(account, _)| account.eq_ignore_ascii_case(member))
            });
            if !active.is_empty() || in_group {
                newly_known.push(member.clone());
            }
        }
        {
            let mut store = self.store.lock().expect("store lock");
            for identity in &newly_trusted {
                store.trust(identity);
            }
            for member in &newly_known {
                store.mark_known(conversation_id, member);
            }
        }
        let changed_keys = self.apply_pins(&mut server);
        self.rosters
            .lock()
            .expect("roster lock")
            .insert(conversation_id.to_string(), server.clone());
        for identity in to_remove {
            // A device that left the server's list is no longer trusted: if
            // it is ever listed again, it waits for the user.
            self.store.lock().expect("store lock").untrust(&identity);
            match self.remove_identity(conversation_id, &identity).await {
                Ok(()) => report.push(conversation_id, "removed", &identity),
                Err(error) => {
                    // Another member may have removed it in the same moment.
                    crate::errln!("note: could not remove {identity}: {error}");
                }
            }
        }
        let mut packages = Vec::new();
        let mut added = Vec::new();
        for (identity, device_id) in to_add {
            // A device whose key changed since this client pinned it waits
            // for the user, like a new device.
            if changed_keys.contains(&identity) {
                pending.push(identity);
                continue;
            }
            let Some(package) = self
                .client
                .fetch_key_package(&self.session.token, &device_id)
                .await?
            else {
                continue;
            };
            match self.crypto.check_key_package(&package, &identity, &server) {
                Ok(()) => {
                    packages.push(package);
                    added.push(identity);
                }
                // One bad package must not stop the rest of the sync, which
                // may hold removals and other conversations.
                Err(error) => {
                    crate::errln!("warning: not adding {identity}: {error}");
                    let pinned = self
                        .store
                        .lock()
                        .expect("store lock")
                        .pinned_key(&identity)
                        .is_some();
                    if pinned {
                        pending.push(identity);
                    }
                }
            }
        }
        self.store
            .lock()
            .expect("store lock")
            .set_pending(conversation_id, &pending);
        for identity in &pending {
            report.push(conversation_id, "pending", identity);
        }
        if !packages.is_empty() {
            let (commit, welcome) = self
                .crypto
                .add_members(conversation_id, &packages)
                .map_err(|error| error.to_string())?;
            self.persist()?;
            self.pin_group_keys(conversation_id);
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
            for identity in added {
                report.push(conversation_id, "added", &identity);
            }
        }
        Ok(report)
    }

    /// Remove one device identity and send the commit. The group is merged
    /// locally before the function returns.
    async fn remove_identity(&self, conversation_id: &str, identity: &str) -> Result<(), String> {
        let commit = self
            .crypto
            .remove_member(conversation_id, identity)
            .map_err(|error| error.to_string())?;
        self.persist()?;
        let payload = encode_envelope(&Envelope::mls(&commit));
        self.client
            .send_message(&self.session.token, conversation_id, &payload)
            .await?;
        Ok(())
    }

    /// Bring every conversation in line with the server and top up this
    /// device's key package stock. New device identities are not added
    /// unless this user has trusted them (or just added that account).
    ///
    /// Everything queued for this device is processed first: planning
    /// against a roster that misses a commit already waiting would commit
    /// a second change in an old epoch and split the group.
    pub async fn sync(&self) -> Result<SyncReport, String> {
        self.catch_up().await?;
        self.top_up_key_packages().await?;
        let mut report = SyncReport::default();
        let empty = HashSet::new();
        for (conversation_id, members) in
            self.client.list_conversations(&self.session.token).await?
        {
            if self
                .crypto
                .has_group(&conversation_id)
                .map_err(|error| error.to_string())?
            {
                let part = self.sync_group(&conversation_id, &members, &empty).await?;
                report.added.extend(part.added);
                report.removed.extend(part.removed);
                report.pending.extend(part.pending);
            }
        }
        Ok(report)
    }

    /// Process everything queued for this device before doing something
    /// that needs a current group state (sending, removing). Returns once
    /// the server has sent the whole queue and every frame is applied.
    pub async fn catch_up(&self) -> Result<(), String> {
        crate::ws::drain(
            &self.session,
            async |conversation: &str, message_id: &str, sender: &str, payload: &[u8]| {
                self.interpret(conversation, message_id, sender, payload)
                    .await
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
        if store.entry(conversation_id, &id).is_none() {
            match app {
                AppMessage::Text { text } | AppMessage::Reply { text, .. } => {
                    store.record(conversation_id, &id, &self.session.username, text);
                }
                AppMessage::Attachment {
                    blob_id,
                    key_b64,
                    nonce_b64,
                    name,
                    mime,
                    size,
                    sha256,
                } => {
                    store.record_attachment(
                        conversation_id,
                        &id,
                        &self.session.username,
                        AttachmentMeta {
                            blob_id: blob_id.clone(),
                            key_b64: key_b64.clone(),
                            nonce_b64: nonce_b64.clone(),
                            name: name.clone(),
                            mime: mime.clone(),
                            size: *size,
                            sha256: sha256.clone(),
                        },
                    );
                }
                _ => {}
            }
        }
        Ok(id)
    }

    /// Encrypt and send a file as an attachment. Images are resized and
    /// recompressed first; the file is then encrypted with a fresh per-file
    /// key, the ciphertext is uploaded under a fresh id, and the key and id
    /// travel to the group inside the MLS message.
    pub async fn send_attachment(
        &self,
        conversation_id: &str,
        path: &Path,
    ) -> Result<String, String> {
        let prepared = crate::files::prepare_upload(path)?;
        let key = FileKey::generate();
        let blob_id = new_blob_id();
        let ciphertext = thrum_core::attachment::encrypt_file(&key, &blob_id, &prepared.bytes);
        self.client
            .upload_blob(&self.session.token, &blob_id, &ciphertext)
            .await?;
        let app = AppMessage::Attachment {
            blob_id,
            key_b64: encode_b64(&key.key),
            nonce_b64: encode_b64(&key.nonce),
            name: prepared.name,
            mime: prepared.mime,
            size: prepared.bytes.len() as u64,
            sha256: sha256_hex(&prepared.bytes),
        };
        self.send_app(conversation_id, &app).await
    }

    /// Fetch, decrypt, checksum-verify and save the attachment of a stored
    /// message; returns the path written. Without `out`, it lands in this
    /// device's attachments directory under a collision-free name.
    pub async fn fetch_attachment(
        &self,
        message_id: &str,
        out: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let (conversation, entry) = self
            .find_message(message_id)
            .ok_or_else(|| format!("message {message_id} is not in this device's store"))?;
        let _ = conversation;
        let meta = entry
            .attachment
            .ok_or_else(|| format!("message {message_id} carries no attachment"))?;
        if !thrum_core::attachment::is_blob_id(&meta.blob_id) {
            return Err("the attachment has an invalid id; not fetching".to_string());
        }
        let key = FileKey {
            key: decode_b64(&meta.key_b64)
                .ok()
                .and_then(|bytes| <[u8; KEY_LEN]>::try_from(bytes).ok())
                .ok_or_else(|| "stored attachment key is corrupt".to_string())?,
            nonce: decode_b64(&meta.nonce_b64)
                .ok()
                .and_then(|bytes| <[u8; NONCE_LEN]>::try_from(bytes).ok())
                .ok_or_else(|| "stored attachment nonce is corrupt".to_string())?,
        };
        let ciphertext = self
            .client
            .fetch_blob(&self.session.token, &meta.blob_id)
            .await?
            .ok_or_else(|| "the attachment has expired on the server".to_string())?;
        let plaintext = thrum_core::attachment::decrypt_file(&key, &meta.blob_id, &ciphertext)
            .ok_or_else(|| {
                "the attachment did not decrypt; key and blob do not match".to_string()
            })?;
        if sha256_hex(&plaintext) != meta.sha256 {
            return Err("the attachment failed its checksum; not saving".to_string());
        }
        match out {
            Some(out) => {
                if let Some(parent) = out.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| format!("cannot create {parent:?}: {error}"))?;
                }
                thrum_core::mls::write_private_file(out, &plaintext)
                    .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
                Ok(out.to_path_buf())
            }
            None => crate::files::save_attachment(&self.attachments_dir, &meta.name, &plaintext),
        }
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
    ///
    /// Welcomes and commits are checked against the server's roster of the
    /// conversation; a refused one is tried once more with a fresh roster,
    /// because the server changes membership just before the MLS commit
    /// that follows it. Application messages are attributed to their MLS
    /// sender; a relay label that names someone else is treated as
    /// tampering and the message is dropped.
    ///
    /// The line is safe to print: peer-chosen text in it has its control
    /// characters escaped (see [`crate::display::clean`]).
    pub async fn interpret(
        &self,
        conversation_id: &str,
        message_id: &str,
        sender_username: &str,
        payload: &[u8],
    ) -> Option<String> {
        self.interpret_raw(conversation_id, message_id, sender_username, payload)
            .await
            .map(|line| crate::display::clean(&line))
    }

    async fn interpret_raw(
        &self,
        conversation_id: &str,
        message_id: &str,
        sender_username: &str,
        payload: &[u8],
    ) -> Option<String> {
        let envelope = decode_envelope(payload)?;
        let data = envelope.data()?;
        match envelope {
            Envelope::Welcome { .. } => {
                let mut outcome = Err(String::new());
                for refresh in [false, true] {
                    let roster = match self.server_roster(conversation_id, refresh).await {
                        Ok(roster) => roster,
                        Err(error) => {
                            crate::errln!("welcome not applied: {error}");
                            return None;
                        }
                    };
                    outcome = self
                        .crypto
                        .process_welcome(conversation_id, &data, &roster)
                        .map_err(|error| error.to_string());
                    if outcome.is_ok() {
                        break;
                    }
                }
                match outcome {
                    Ok(group_conversation) => {
                        // The new group state must reach disk before the
                        // process ends, or the next run knows nothing about
                        // the join.
                        if let Err(error) = self.persist() {
                            crate::errln!("warning: could not save MLS state: {error}");
                        }
                        self.pin_group_keys(&group_conversation);
                        Some(format!("joined {group_conversation}"))
                    }
                    Err(error) => {
                        crate::errln!("welcome not applied: {error}"); // stale, foreign or forged
                        None
                    }
                }
            }
            Envelope::Mls { .. } => {
                let before = self
                    .crypto
                    .group_members(conversation_id)
                    .unwrap_or_default();
                let mut incoming = Ok(Incoming::Ignored);
                for refresh in [false, true] {
                    let roster = match self.server_roster(conversation_id, refresh).await {
                        Ok(roster) => roster,
                        Err(error) => {
                            crate::errln!("message not applied: {error}");
                            return None;
                        }
                    };
                    incoming = self
                        .crypto
                        .process_incoming(conversation_id, &data, &roster);
                    if !matches!(incoming, Ok(Incoming::Rejected(_))) {
                        break;
                    }
                }
                match incoming {
                    Ok(Incoming::Application { sender, payload }) => {
                        let (username, _) = split_identity(&sender)?;
                        if !username.eq_ignore_ascii_case(sender_username) {
                            crate::errln!(
                                "warning: dropped a message labeled {sender_username} \
                                 that {sender} sent"
                            );
                            return None;
                        }
                        self.apply_app_message(conversation_id, message_id, username, &payload)
                    }
                    Ok(Incoming::Commit) => {
                        let after = self
                            .crypto
                            .group_members(conversation_id)
                            .unwrap_or_default();
                        self.persist().ok()?;
                        self.pin_group_keys(conversation_id);
                        let note = roster_note(&before, &after);
                        if note.is_empty() { None } else { Some(note) }
                    }
                    Ok(Incoming::Rejected(reason)) => {
                        crate::errln!("commit refused: {reason}");
                        None
                    }
                    Ok(Incoming::Ignored) => None,
                    Err(error) => {
                        crate::errln!("message not applied: {error}"); // not for this device
                        None
                    }
                }
            }
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
                let short: String = reply_to.chars().take(8).collect();
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
            AppMessage::Attachment {
                blob_id,
                key_b64,
                nonce_b64,
                name,
                mime,
                size,
                sha256,
            } => {
                if !thrum_core::attachment::is_blob_id(&blob_id) {
                    return Some(format!(
                        "{sender} sent an attachment with an invalid id; ignored"
                    ));
                }
                if store.entry(conversation_id, message_id).is_none() {
                    let meta = AttachmentMeta {
                        blob_id,
                        key_b64,
                        nonce_b64,
                        mime,
                        size,
                        sha256,
                        name: name.clone(),
                    };
                    store.record_attachment(conversation_id, message_id, sender, meta);
                    Some(format!(
                        "{sender} sent {name} ({size} bytes); \
                         run `thrum fetch --message {message_id}` to save it"
                    ))
                } else {
                    // This device's own echo; it already has the metadata.
                    None
                }
            }
        }
    }
}

/// A one-line description of who joined or left in a commit.
fn roster_note(before: &[String], after: &[String]) -> String {
    let mut parts = Vec::new();
    for identity in after {
        if !before.iter().any(|member| member == identity) {
            parts.push(format!("{identity} joined"));
        }
    }
    for identity in before {
        if !after.iter().any(|member| member == identity) {
            parts.push(format!("{identity} left"));
        }
    }
    parts.join(", ")
}

/// Remove everything this device keeps next to `state_file`: the session
/// (its token), the MLS keys, the message store with its history and trust
/// decisions, and saved attachments. Returns what was removed; a path that
/// was not there is skipped.
pub fn forget_device(state_file: &Path) -> Result<Vec<PathBuf>, String> {
    let mut removed = Vec::new();
    for path in [
        state_file.to_path_buf(),
        crate::store::store_path(state_file),
        state_file_mls_dir(state_file),
        state_file_attachments_dir(state_file),
    ] {
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => removed.push(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot remove {}: {error}", path.display())),
        }
    }
    Ok(removed)
}

/// The MLS state directory for a state file: `<state-file>.mls/`.
fn state_file_mls_dir(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".mls");
    PathBuf::from(name)
}

/// The attachments directory for a state file: `<state-file>.attachments/`.
fn state_file_attachments_dir(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".attachments");
    PathBuf::from(name)
}

fn encode_b64(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

fn decode_b64(text: &str) -> Result<Vec<u8>, base64::DecodeError> {
    BASE64.decode(text)
}

/// SHA-256 of some bytes, hex encoded; attachments carry this so receivers
/// can check what they decrypt against what was sent.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A fresh 128-bit blob id, hex encoded. Chosen before encryption because
/// the per-file key is bound to it as AES-GCM associated data.
fn new_blob_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(32);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Encode payload bytes for tests and debugging.
pub fn payload_to_base64(bytes: &[u8]) -> String {
    thrum_core::protocol::encode_payload(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use thrum_core::mls::DeviceCrypto;

    #[test]
    fn a_mismatched_key_package_is_rejected() {
        let alice = DeviceCrypto::generate("alice#device-1".to_string());
        let bob = DeviceCrypto::generate("bob#device-1".to_string());
        let mut server = ServerRoster::new();
        server.add_device("bob#device-1", Some(bob.signature_key()));
        server.add_device("carol#device-1", None);
        // The right pairing passes through unchanged.
        let package = bob.new_key_package().expect("key package");
        alice
            .check_key_package(&package, "bob#device-1", &server)
            .expect("matching identity");
        // The same bytes offered for a different device are refused.
        let error = alice
            .check_key_package(&package, "carol#device-1", &server)
            .expect_err("swapped identity must fail");
        assert!(
            error.contains("bob#device-1") && error.contains("carol#device-1"),
            "error names both identities: {error}"
        );
    }

    #[test]
    fn a_key_package_with_another_key_than_the_registered_one_is_rejected() {
        let alice = DeviceCrypto::generate("alice#device-1".to_string());
        let bob = DeviceCrypto::generate("bob#device-1".to_string());
        let impostor = DeviceCrypto::generate("bob#device-1".to_string());
        let mut server = ServerRoster::new();
        server.add_device("bob#device-1", Some(bob.signature_key()));
        let package = impostor.new_key_package().expect("key package");
        assert!(
            alice
                .check_key_package(&package, "bob#device-1", &server)
                .is_err()
        );
    }
}
