//! The device's local view of conversations: what messages it has seen,
//! their current text, and read state. Edits and deletes arrive as encrypted
//! messages; this store applies them. The server keeps no history, so this
//! file is the only record.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What a recipient device needs to fetch and decrypt one attachment.
/// Key and nonce travel here from inside the encrypted MLS message; the
/// server never sees them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub blob_id: String,
    pub key_b64: String,
    pub nonce_b64: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// SHA-256 of the plaintext, hex encoded; checked after decryption.
    pub sha256: String,
}

/// One known message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Username of the original sender.
    pub sender: String,
    /// Current text (after edits; empty once deleted).
    pub text: String,
    /// The message was retracted by its sender.
    pub deleted: bool,
    /// Every member device has told us they read it.
    pub read_by: Vec<String>,
    /// Present when the message carries an attachment.
    #[serde(default)]
    pub attachment: Option<AttachmentMeta>,
}

/// Conversation id → message id → entry.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    conversations: BTreeMap<String, BTreeMap<String, Entry>>,
    /// Device identities (`username#device-id`) this user has agreed to add.
    #[serde(default)]
    trusted_devices: BTreeSet<String>,
    /// Conversation id → device identities waiting for approval.
    #[serde(default)]
    pending_devices: BTreeMap<String, Vec<String>>,
    /// Conversation id → accounts whose devices this client has already
    /// considered. A device that appears after that stays pending.
    #[serde(default)]
    known_accounts: BTreeMap<String, BTreeSet<String>>,
    /// Device identity → the signature key (base64) this client first saw
    /// it with. A later leaf or key package under the same identity must
    /// carry the same key.
    #[serde(default)]
    pinned_keys: BTreeMap<String, String>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Store {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the store from `<state-file>.messages.json` when it exists.
    ///
    /// A file that cannot be read or parsed is an error, and is left as it
    /// is: starting empty would silently forget which devices this user
    /// trusts, and the next save would overwrite the evidence.
    pub fn load(state_file: &Path) -> Result<Self, String> {
        let path = store_path(state_file);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let mut store: Store = serde_json::from_slice(&bytes).map_err(|error| {
                    format!(
                        "{} is damaged ({error}); move it aside to start over, which \
                         forgets this device's history and trust decisions",
                        path.display()
                    )
                })?;
                store.path = Some(path);
                Ok(store)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                path: Some(path),
                ..Self::default()
            }),
            Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        }
    }

    /// Record a message someone else (or this device) sent.
    pub fn record(&mut self, conversation_id: &str, message_id: &str, sender: &str, text: &str) {
        self.insert_entry(
            conversation_id,
            message_id,
            Entry {
                sender: sender.to_string(),
                text: text.to_string(),
                deleted: false,
                read_by: Vec::new(),
                attachment: None,
            },
        );
    }

    /// Record a message that carries an attachment; the description doubles
    /// as the entry text, and the metadata lets `fetch` save the file later.
    pub fn record_attachment(
        &mut self,
        conversation_id: &str,
        message_id: &str,
        sender: &str,
        attachment: AttachmentMeta,
    ) {
        self.insert_entry(
            conversation_id,
            message_id,
            Entry {
                sender: sender.to_string(),
                text: format!(
                    "[attachment: {} ({} bytes)]",
                    attachment.name, attachment.size
                ),
                deleted: false,
                read_by: Vec::new(),
                attachment: Some(attachment),
            },
        );
    }

    fn insert_entry(&mut self, conversation_id: &str, message_id: &str, entry: Entry) {
        self.conversations
            .entry(conversation_id.to_string())
            .or_default()
            .insert(message_id.to_string(), entry);
        self.save();
    }

    /// Apply an edit; only the original sender may edit. True when applied.
    pub fn edit(
        &mut self,
        conversation_id: &str,
        message_id: &str,
        sender: &str,
        text: &str,
    ) -> bool {
        let Some(entry) = self.entry(conversation_id, message_id) else {
            return false;
        };
        if entry.sender != sender || entry.deleted {
            return false;
        }
        self.entry_mut(conversation_id, message_id).text = text.to_string();
        self.save();
        true
    }

    /// Apply a delete; only the original sender may delete. True when applied.
    pub fn delete(&mut self, conversation_id: &str, message_id: &str, sender: &str) -> bool {
        let Some(entry) = self.entry(conversation_id, message_id) else {
            return false;
        };
        if entry.sender != sender || entry.deleted {
            return false;
        }
        let entry = self.entry_mut(conversation_id, message_id);
        entry.deleted = true;
        entry.text.clear();
        entry.read_by.clear();
        self.save();
        true
    }

    /// Record that `reader` has read these messages.
    pub fn mark_read(&mut self, conversation_id: &str, reader: &str, message_ids: &[String]) {
        let Some(messages) = self.conversations.get_mut(conversation_id) else {
            return;
        };
        for message_id in message_ids {
            if let Some(entry) = messages.get_mut(message_id)
                && entry.sender != reader
                && !entry.read_by.iter().any(|r| r == reader)
            {
                entry.read_by.push(reader.to_string());
            }
        }
        self.save();
    }

    /// The ids of a conversation's messages this device has seen, oldest
    /// insertion first; used for `read` receipts and tests.
    pub fn ids(&self, conversation_id: &str) -> Vec<String> {
        self.conversations
            .get(conversation_id)
            .map(|messages| messages.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The entry for one message.
    pub fn entry(&self, conversation_id: &str, message_id: &str) -> Option<&Entry> {
        self.conversations
            .get(conversation_id)
            .and_then(|messages| messages.get(message_id))
    }

    /// Find a message anywhere; returns its conversation id and entry.
    pub fn find(&self, message_id: &str) -> Option<(String, &Entry)> {
        self.conversations
            .iter()
            .find_map(|(conversation, messages)| {
                messages
                    .get(message_id)
                    .map(|entry| (conversation.clone(), entry))
            })
    }

    fn entry_mut(&mut self, conversation_id: &str, message_id: &str) -> &mut Entry {
        self.conversations
            .get_mut(conversation_id)
            .and_then(|messages| messages.get_mut(message_id))
            .expect("checked by caller")
    }

    /// The ids of messages in a conversation that `username` has neither
    /// sent nor read; `read` receipts cover exactly these.
    pub fn unread_for(&self, conversation_id: &str, username: &str) -> Vec<String> {
        self.conversations
            .get(conversation_id)
            .map(|messages| {
                messages
                    .iter()
                    .filter(|(id, entry)| {
                        entry.sender != username
                            && !entry.deleted
                            && entry.read_by.iter().all(|r| r != username)
                            && !id.is_empty()
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remember that this user accepts `identity` into their groups.
    pub fn trust(&mut self, identity: &str) {
        self.trusted_devices.insert(identity.to_string());
        for pending in self.pending_devices.values_mut() {
            pending.retain(|item| item != identity);
        }
        self.pending_devices
            .retain(|_, pending| !pending.is_empty());
        self.save();
    }

    /// Stop trusting a device identity, for example once the server no
    /// longer lists it: if it comes back, it waits for the user again.
    pub fn untrust(&mut self, identity: &str) {
        if self.trusted_devices.remove(identity) {
            self.save();
        }
    }

    /// Trust `identity` with whatever key it presents next, replacing the
    /// key pinned for it. This is the user's explicit decision.
    pub fn trust_new_key(&mut self, identity: &str) {
        self.pinned_keys.remove(identity);
        self.trust(identity);
    }

    /// The signature key pinned for a device identity.
    pub fn pinned_key(&self, identity: &str) -> Option<Vec<u8>> {
        use base64::Engine as _;
        self.pinned_keys
            .get(identity)
            .and_then(|key| base64::engine::general_purpose::STANDARD.decode(key).ok())
    }

    /// Every pinned identity with its key.
    pub fn pinned_keys(&self) -> Vec<(String, Vec<u8>)> {
        self.pinned_keys
            .keys()
            .filter_map(|identity| Some((identity.clone(), self.pinned_key(identity)?)))
            .collect()
    }

    /// Pin the key of each identity that has none yet. Returns the
    /// identities whose presented key differs from the pinned one.
    pub fn pin_keys(&mut self, leaves: &[(String, Vec<u8>)]) -> Vec<String> {
        use base64::Engine as _;
        let mut changed = Vec::new();
        let mut added = false;
        for (identity, key) in leaves {
            match self.pinned_key(identity) {
                Some(pinned) if &pinned != key => changed.push(identity.clone()),
                Some(_) => {}
                None => {
                    self.pinned_keys.insert(
                        identity.clone(),
                        base64::engine::general_purpose::STANDARD.encode(key),
                    );
                    added = true;
                }
            }
        }
        if added {
            self.save();
        }
        changed
    }

    /// Device identities this user has agreed to add.
    pub fn trusted(&self) -> BTreeSet<String> {
        self.trusted_devices.clone()
    }

    /// Whether this conversation has already introduced `username`.
    pub fn knows_account(&self, conversation_id: &str, username: &str) -> bool {
        self.known_accounts
            .get(conversation_id)
            .is_some_and(|names| names.iter().any(|name| name.eq_ignore_ascii_case(username)))
    }

    /// Whether this client has already introduced `username` in any
    /// conversation. Trust on first use is per account: a new conversation
    /// with a known account does not admit its new devices unasked.
    pub fn knows_account_anywhere(&self, username: &str) -> bool {
        self.known_accounts
            .values()
            .any(|names| names.iter().any(|name| name.eq_ignore_ascii_case(username)))
    }

    /// Remember that `username` has been seen in this conversation.
    pub fn mark_known(&mut self, conversation_id: &str, username: &str) {
        self.known_accounts
            .entry(conversation_id.to_string())
            .or_default()
            .insert(username.to_string());
        self.save();
    }

    /// Replace the pending list for one conversation.
    pub fn set_pending(&mut self, conversation_id: &str, identities: &[String]) {
        if identities.is_empty() {
            self.pending_devices.remove(conversation_id);
        } else {
            self.pending_devices
                .insert(conversation_id.to_string(), identities.to_vec());
        }
        self.save();
    }

    /// `(conversation id, identity)` pairs waiting for approval.
    pub fn pending(&self) -> Vec<(String, String)> {
        self.pending_devices
            .iter()
            .flat_map(|(conversation, identities)| {
                identities
                    .iter()
                    .map(|identity| (conversation.clone(), identity.clone()))
            })
            .collect()
    }

    /// Persist to disk, atomically and readable by the owner only. The
    /// store is the only record of history and trust, so a failure is
    /// reported rather than swallowed.
    fn save(&self) {
        let Some(path) = &self.path else { return };
        let result = serde_json::to_vec(self)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                thrum_core::mls::write_private_file(path, &bytes).map_err(|e| e.to_string())
            });
        if let Err(error) = result {
            crate::errln!("warning: could not save {}: {error}", path.display());
        }
    }
}

/// The store file lives next to the session state file.
pub(crate) fn store_path(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".messages.json");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_store_is_refused_and_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state.json");
        let mut store = Store::load(&state).unwrap();
        store.trust("alice#phone");
        store.mark_known("c1", "alice");
        let path = store_path(&state);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        let error = Store::load(&state).expect_err("a damaged store must not load");
        assert!(error.contains("damaged"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), &bytes[..bytes.len() / 2]);
    }

    #[test]
    fn saves_replace_the_file_whole_and_privately() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state.json");
        let mut store = Store::load(&state).unwrap();
        for n in 0..20 {
            store.record("c1", &format!("m{n}"), "alice", "hello");
        }
        let loaded = Store::load(&state).unwrap();
        assert_eq!(loaded.entry("c1", "m19").unwrap().text, "hello");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(
            names,
            ["state.json.messages.json"],
            "no temporary files left"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store_path(&state))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn accounts_are_known_across_conversations_and_keys_stay_pinned() {
        let mut store = Store::new();
        store.mark_known("c1", "Bob");
        assert!(store.knows_account_anywhere("bob"));
        assert!(!store.knows_account_anywhere("carol"));

        let key = vec![1u8; 32];
        assert!(
            store
                .pin_keys(&[("bob#phone".into(), key.clone())])
                .is_empty()
        );
        let changed = store.pin_keys(&[("bob#phone".into(), vec![2u8; 32])]);
        assert_eq!(changed, ["bob#phone"]);
        assert_eq!(store.pinned_key("bob#phone"), Some(key));
        store.trust_new_key("bob#phone");
        assert!(store.pinned_key("bob#phone").is_none());
        assert!(store.trusted().contains("bob#phone"));
        store.untrust("bob#phone");
        assert!(!store.trusted().contains("bob#phone"));
    }

    #[test]
    fn edits_and_deletes_only_by_the_sender() {
        let mut store = Store::new();
        store.record("c1", "m1", "alice", "hello");
        assert!(!store.edit("c1", "m1", "bob", "hacked"));
        assert_eq!(store.entry("c1", "m1").unwrap().text, "hello");
        assert!(store.edit("c1", "m1", "alice", "hello there"));
        assert_eq!(store.entry("c1", "m1").unwrap().text, "hello there");
        assert!(!store.delete("c1", "m1", "bob"));
        assert!(store.delete("c1", "m1", "alice"));
        assert!(store.entry("c1", "m1").unwrap().deleted);
    }

    #[test]
    fn read_receipts_ignore_the_reader_and_deduplicate() {
        let mut store = Store::new();
        store.record("c1", "m1", "alice", "hello");
        store.mark_read("c1", "alice", &["m1".to_string()]);
        assert!(store.entry("c1", "m1").unwrap().read_by.is_empty());
        store.mark_read("c1", "bob", &["m1".to_string()]);
        store.mark_read("c1", "bob", &["m1".to_string()]);
        assert_eq!(store.entry("c1", "m1").unwrap().read_by, ["bob"]);
        assert_eq!(store.unread_for("c1", "bob"), Vec::<String>::new());
        assert_eq!(store.unread_for("c1", "carol"), ["m1"]);
    }

    #[test]
    fn roundtrip_through_json() {
        let mut store = Store::new();
        store.record("c1", "m1", "alice", "hello");
        store.mark_read("c1", "bob", &["m1".to_string()]);
        let bytes = serde_json::to_vec(&store).unwrap();
        let back: Store = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.entry("c1", "m1").unwrap().read_by, ["bob"]);
    }
}
