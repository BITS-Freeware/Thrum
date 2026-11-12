//! The device's local view of conversations: what messages it has seen,
//! their current text, and read state. Edits and deletes arrive as encrypted
//! messages; this store applies them. The server keeps no history, so this
//! file is the only record.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
}

/// Conversation id → message id → entry.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    conversations: BTreeMap<String, BTreeMap<String, Entry>>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Store {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the store from `<state-file>.messages.json` when it exists.
    pub fn load(state_file: &Path) -> Self {
        let path = store_path(state_file);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let mut store: Store = serde_json::from_slice(&bytes).unwrap_or_default();
                store.path = Some(path);
                store
            }
            Err(_) => Self {
                conversations: BTreeMap::new(),
                path: Some(path),
            },
        }
    }

    /// Record a message someone else (or this device) sent.
    pub fn record(&mut self, conversation_id: &str, message_id: &str, sender: &str, text: &str) {
        self.conversations
            .entry(conversation_id.to_string())
            .or_default()
            .insert(
                message_id.to_string(),
                Entry {
                    sender: sender.to_string(),
                    text: text.to_string(),
                    deleted: false,
                    read_by: Vec::new(),
                },
            );
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

    /// Persist to disk; failures are swallowed, the store is a cache.
    fn save(&self) {
        if let Some(path) = &self.path
            && let Ok(bytes) = serde_json::to_vec(self)
        {
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// The store file lives next to the session state file.
fn store_path(state_file: &Path) -> PathBuf {
    let mut name = state_file.as_os_str().to_os_string();
    name.push(".messages.json");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

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
