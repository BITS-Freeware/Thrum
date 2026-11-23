//! Check MLS membership changes against the server's view of a
//! conversation before a client accepts them.
//!
//! MLS lets any member commit adds and removals, and every member merges
//! what it receives. Thrum's rules about who belongs in a conversation live
//! on the server: who is a member, which of their devices are signed in, and
//! which signature key each device registered. A client therefore checks
//! every welcome it joins and every commit it merges against a
//! [`ServerRoster`], so that a member cannot add a leaf that claims someone
//! else, remove another member, or slip in a leaf that no client can
//! remove. Against an honest server this stops a malicious member. Against
//! the server itself, device trust still rests on first sight (decision
//! 0013) and on the keys a client pins.

use std::collections::{BTreeMap, BTreeSet};

use crate::mls::split_identity;

/// The server's view of one conversation: each member account and, for
/// each of its signed-in devices, the signature key the server bound to it
/// (`None` while the device has not uploaded a key package yet).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerRoster {
    accounts: BTreeMap<String, BTreeMap<String, Option<Vec<u8>>>>,
}

impl ServerRoster {
    /// An empty roster.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a member account and its active devices.
    pub fn add_account(
        &mut self,
        username: &str,
        devices: impl IntoIterator<Item = (String, Option<Vec<u8>>)>,
    ) {
        let entry = self.accounts.entry(username.to_lowercase()).or_default();
        for (device_id, key) in devices {
            if !device_id.is_empty() {
                entry.insert(device_id, key);
            }
        }
    }

    /// Record one active device (`username#device-id`) of a member account,
    /// with its bound signature key if known.
    pub fn add_device(&mut self, identity: &str, signature_key: Option<Vec<u8>>) {
        if let Some((username, device_id)) = split_identity(identity) {
            self.add_account(username, [(device_id.to_string(), signature_key)]);
        }
    }

    /// Whether the account is a member of the conversation.
    pub fn has_account(&self, username: &str) -> bool {
        self.accounts.contains_key(&username.to_lowercase())
    }

    /// The member accounts, lowercased.
    pub fn accounts(&self) -> impl Iterator<Item = &str> {
        self.accounts.keys().map(String::as_str)
    }

    /// Whether `identity` (`username#device-id`) is an active device of a
    /// member account.
    pub fn lists(&self, identity: &str) -> bool {
        self.device(identity).is_some()
    }

    /// Hold a listed device to the signature key this client pinned for it,
    /// whatever key the server reports. Returns true when the server
    /// reported a different key: the server, or the device, has changed
    /// keys, and only the user can accept that. Unlisted identities are
    /// left out.
    pub fn pin(&mut self, identity: &str, pinned: &[u8]) -> bool {
        let Some((username, device_id)) = split_identity(identity) else {
            return false;
        };
        let Some(key) = self
            .accounts
            .get_mut(&username.to_lowercase())
            .and_then(|devices| devices.get_mut(device_id))
        else {
            return false;
        };
        let changed = key.as_deref().is_some_and(|reported| reported != pinned);
        *key = Some(pinned.to_vec());
        changed
    }

    /// `Some(key)` for a listed device, where `key` is the bound signature
    /// key if the server has one; `None` when the device is not listed.
    fn device(&self, identity: &str) -> Option<&Option<Vec<u8>>> {
        let (username, device_id) = split_identity(identity)?;
        self.accounts.get(&username.to_lowercase())?.get(device_id)
    }

    /// Check a leaf that is joining or present in a group: it must be a
    /// well-formed identity, an active device of a member, and carry the
    /// signature key the server bound to that device.
    fn check_listed(&self, leaf: &Leaf) -> Result<(), String> {
        if !leaf.well_formed() {
            return Err(format!(
                "leaf {:?} is not username#device-id",
                leaf.identity
            ));
        }
        match self.device(&leaf.identity) {
            None => Err(format!(
                "{} is not an active device of a member of this conversation",
                leaf.identity
            )),
            Some(Some(bound)) if bound != &leaf.signature_key => Err(format!(
                "{} does not carry the signature key the server registered for it",
                leaf.identity
            )),
            Some(_) => Ok(()),
        }
    }
}

/// One MLS leaf as the membership checks see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    /// The credential identity; invalid UTF-8 is replaced, and
    /// `valid_utf8` is false.
    pub identity: String,
    /// Whether the credential was a basic credential holding UTF-8.
    pub valid_utf8: bool,
    /// The leaf's signature public key.
    pub signature_key: Vec<u8>,
}

impl Leaf {
    /// A leaf from its parts.
    pub fn new(identity: impl Into<String>, signature_key: Vec<u8>) -> Self {
        Self {
            identity: identity.into(),
            valid_utf8: true,
            signature_key,
        }
    }

    /// True when the identity is valid UTF-8 of the form
    /// `username#device-id`, with no second `#`.
    pub fn well_formed(&self) -> bool {
        self.valid_utf8
            && split_identity(&self.identity).is_some_and(|(username, device_id)| {
                !device_id.contains('#') && !username.is_empty()
            })
    }
}

/// What one commit does to the group, read from the staged commit before it
/// is merged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitChanges {
    /// The committing leaf.
    pub sender: Option<Leaf>,
    /// Leaves the commit adds.
    pub added: Vec<Leaf>,
    /// Leaves the commit removes.
    pub removed: Vec<Leaf>,
    /// Leaves the commit replaces (update proposals and the committer's
    /// path), as (leaf before, leaf after).
    pub updated: Vec<(Leaf, Leaf)>,
    /// Every leaf of the group once the commit is merged.
    pub members_after: Vec<Leaf>,
}

/// Accept or refuse a commit.
///
/// - The committer must be an active device of a member, unless the commit
///   only removes the committer itself, or the committer's account is no
///   longer a member at all (it left, or was deleted, after committing).
/// - Every added leaf must be an active device of a member and carry its
///   registered signature key.
/// - A removed leaf must be the committer, a malformed leaf, or a device the
///   server no longer lists. Only the server decides who leaves, and a
///   removal that follows it reaches every client the same way.
/// - An update must keep the leaf's identity, and its key when one is bound.
/// - No two leaves may share an identity once the commit is merged.
pub fn check_commit(roster: &ServerRoster, changes: &CommitChanges) -> Result<(), String> {
    let sender = changes
        .sender
        .as_ref()
        .ok_or_else(|| "commit has no member sender".to_string())?;
    let self_removal_only = changes.added.is_empty()
        && !changes.removed.is_empty()
        && changes
            .removed
            .iter()
            .all(|leaf| leaf.identity == sender.identity);
    // A committer whose account has left the conversation (or was deleted)
    // may still have sent a commit a member missed while offline. Its leaf
    // is a group member, which MLS has checked, and the rules below hold it
    // to the server's roster like anyone else, so it is accepted; a member
    // account's device must still be listed with its key.
    let account_left =
        split_identity(&sender.identity).is_some_and(|(username, _)| !roster.has_account(username));
    if !(self_removal_only || (sender.well_formed() && account_left)) {
        roster
            .check_listed(sender)
            .map_err(|reason| format!("committer refused: {reason}"))?;
    }
    for leaf in &changes.added {
        roster
            .check_listed(leaf)
            .map_err(|reason| format!("add refused: {reason}"))?;
    }
    for leaf in &changes.removed {
        let allowed = leaf.identity == sender.identity
            || !leaf.well_formed()
            || !roster.lists(&leaf.identity);
        if !allowed {
            return Err(format!(
                "removal of {} refused: the server still lists it in this conversation",
                leaf.identity
            ));
        }
    }
    for (before, after) in &changes.updated {
        if before.identity != after.identity || before.valid_utf8 != after.valid_utf8 {
            return Err(format!(
                "update refused: {} would become {}",
                before.identity, after.identity
            ));
        }
        if after.well_formed()
            && let Some(Some(bound)) = roster.device(&after.identity)
            && bound != &after.signature_key
        {
            return Err(format!(
                "update refused: {} would drop its registered signature key",
                after.identity
            ));
        }
    }
    unique_identities(&changes.members_after)
}

/// Accept or refuse a welcome before joining its group.
///
/// The welcome must be for the conversation it was relayed in, and every
/// leaf it brings must be an active device of a member with its registered
/// key, with no identity twice. `self_identity` must be among the leaves.
pub fn check_welcome(
    roster: &ServerRoster,
    conversation_id: &str,
    group_id: &str,
    self_identity: &str,
    members: &[Leaf],
) -> Result<(), String> {
    if group_id != conversation_id {
        return Err(format!(
            "welcome names group {group_id:?} but arrived in conversation {conversation_id:?}"
        ));
    }
    if !members.iter().any(|leaf| leaf.identity == self_identity) {
        return Err("welcome does not hold this device".to_string());
    }
    for leaf in members {
        if leaf.identity == self_identity {
            continue;
        }
        roster
            .check_listed(leaf)
            .map_err(|reason| format!("welcome refused: {reason}"))?;
    }
    unique_identities(members)
}

fn unique_identities(members: &[Leaf]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for leaf in members {
        if !seen.insert(leaf.identity.as_str()) {
            return Err(format!("two leaves would claim {}", leaf.identity));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pinned_key_wins_over_the_key_the_server_reports() {
        let mut roster = ServerRoster::new();
        roster.add_device("bob#phone", Some(vec![9; 32]));
        assert!(roster.pin("bob#phone", &[1; 32]));
        assert!(!roster.pin("bob#phone", &[1; 32]));
        assert!(
            !roster.pin("carol#phone", &[1; 32]),
            "unlisted stays unlisted"
        );
        assert!(!roster.lists("carol#phone"));
        let changes = CommitChanges {
            sender: Some(Leaf::new("bob#phone", vec![1; 32])),
            added: vec![Leaf::new("bob#phone", vec![9; 32])],
            ..Default::default()
        };
        assert!(check_commit(&roster, &changes).is_err());
    }

    fn roster() -> ServerRoster {
        let mut roster = ServerRoster::new();
        roster.add_account("alice", [("a1".to_string(), Some(vec![1]))]);
        roster.add_account(
            "Bob",
            [("b1".to_string(), Some(vec![2])), ("b2".to_string(), None)],
        );
        roster
    }

    fn leaf(identity: &str, key: u8) -> Leaf {
        Leaf::new(identity, vec![key])
    }

    fn commit(
        sender: Leaf,
        added: Vec<Leaf>,
        removed: Vec<Leaf>,
        after: Vec<Leaf>,
    ) -> CommitChanges {
        CommitChanges {
            sender: Some(sender),
            added,
            removed,
            updated: Vec::new(),
            members_after: after,
        }
    }

    #[test]
    fn an_add_of_a_listed_device_with_its_key_passes() {
        let changes = commit(
            leaf("alice#a1", 1),
            vec![leaf("bob#b1", 2)],
            vec![],
            vec![leaf("alice#a1", 1), leaf("bob#b1", 2)],
        );
        assert_eq!(check_commit(&roster(), &changes), Ok(()));
    }

    #[test]
    fn a_device_without_a_bound_key_is_checked_by_identity() {
        let changes = commit(
            leaf("alice#a1", 1),
            vec![leaf("BOB#b2", 9)],
            vec![],
            vec![leaf("alice#a1", 1), leaf("BOB#b2", 9)],
        );
        assert_eq!(check_commit(&roster(), &changes), Ok(()));
    }

    #[test]
    fn an_add_that_claims_a_listed_device_with_another_key_is_refused() {
        let changes = commit(
            leaf("bob#b1", 2),
            vec![leaf("alice#a1", 7)],
            vec![],
            vec![leaf("alice#a1", 1), leaf("bob#b1", 2), leaf("alice#a1", 7)],
        );
        assert!(check_commit(&roster(), &changes).is_err());
    }

    #[test]
    fn an_add_of_an_unlisted_or_malformed_identity_is_refused() {
        for identity in ["alice#evil", "mallory#m1", "thrum", "alice#a1#x", "#a1"] {
            let changes = commit(
                leaf("alice#a1", 1),
                vec![leaf(identity, 3)],
                vec![],
                vec![leaf("alice#a1", 1), leaf(identity, 3)],
            );
            assert!(check_commit(&roster(), &changes).is_err(), "{identity}");
        }
        let mut bad = leaf("bob#b2", 3);
        bad.valid_utf8 = false;
        let changes = commit(leaf("alice#a1", 1), vec![bad.clone()], vec![], vec![bad]);
        assert!(check_commit(&roster(), &changes).is_err());
    }

    #[test]
    fn a_duplicate_identity_is_refused_even_without_a_bound_key() {
        let changes = commit(
            leaf("alice#a1", 1),
            vec![leaf("bob#b2", 3)],
            vec![],
            vec![leaf("alice#a1", 1), leaf("bob#b2", 4), leaf("bob#b2", 3)],
        );
        assert!(check_commit(&roster(), &changes).is_err());
    }

    #[test]
    fn removing_a_device_the_server_still_lists_is_refused() {
        let changes = commit(
            leaf("bob#b1", 2),
            vec![],
            vec![leaf("alice#a1", 1)],
            vec![leaf("bob#b1", 2)],
        );
        assert!(check_commit(&roster(), &changes).is_err());
    }

    #[test]
    fn removing_unlisted_malformed_or_own_leaves_passes() {
        for removed in ["carol#c1", "alice#old", "thrum"] {
            let changes = commit(
                leaf("alice#a1", 1),
                vec![],
                vec![leaf(removed, 5)],
                vec![leaf("alice#a1", 1)],
            );
            assert_eq!(check_commit(&roster(), &changes), Ok(()), "{removed}");
        }
        let changes = commit(
            leaf("carol#c1", 5),
            vec![],
            vec![leaf("carol#c1", 5)],
            vec![],
        );
        assert_eq!(check_commit(&roster(), &changes), Ok(()));
    }

    #[test]
    fn a_committer_the_server_does_not_list_is_refused() {
        // A device of a member account that the server does not list.
        let changes = commit(
            leaf("alice#a9", 5),
            vec![leaf("bob#b2", 3)],
            vec![],
            vec![leaf("alice#a9", 5), leaf("bob#b2", 3)],
        );
        assert!(check_commit(&roster(), &changes).is_err());
        // A member's device with another key than the bound one.
        let changes = commit(
            leaf("alice#a1", 9),
            vec![leaf("bob#b2", 3)],
            vec![],
            vec![leaf("alice#a1", 9), leaf("bob#b2", 3)],
        );
        assert!(check_commit(&roster(), &changes).is_err());
    }

    #[test]
    fn a_committer_whose_account_left_is_held_to_the_roster() {
        // Carol's account is gone; her commit adding a listed device
        // applies, one adding an unlisted device does not.
        let changes = commit(
            leaf("carol#c1", 5),
            vec![leaf("bob#b2", 3)],
            vec![],
            vec![leaf("carol#c1", 5), leaf("bob#b2", 3)],
        );
        assert!(check_commit(&roster(), &changes).is_ok());
        let changes = commit(
            leaf("carol#c1", 5),
            vec![leaf("carol#c2", 6)],
            vec![],
            vec![leaf("carol#c1", 5), leaf("carol#c2", 6)],
        );
        assert!(check_commit(&roster(), &changes).is_err());
    }

    #[test]
    fn an_update_cannot_change_identity_or_drop_the_bound_key() {
        let mut changes = commit(
            leaf("alice#a1", 1),
            vec![],
            vec![],
            vec![leaf("alice#a1", 1)],
        );
        changes.updated = vec![(leaf("alice#a1", 1), leaf("bob#b1", 2))];
        assert!(check_commit(&roster(), &changes).is_err());
        changes.updated = vec![(leaf("alice#a1", 1), leaf("alice#a1", 8))];
        assert!(check_commit(&roster(), &changes).is_err());
        changes.updated = vec![(leaf("alice#a1", 1), leaf("alice#a1", 1))];
        assert_eq!(check_commit(&roster(), &changes), Ok(()));
    }

    #[test]
    fn a_welcome_must_match_its_conversation_and_list_every_leaf() {
        let members = vec![leaf("alice#a1", 1), leaf("bob#b1", 2)];
        assert_eq!(
            check_welcome(&roster(), "c", "c", "bob#b1", &members),
            Ok(())
        );
        assert!(check_welcome(&roster(), "c", "other", "bob#b1", &members).is_err());
        assert!(check_welcome(&roster(), "c", "c", "bob#b2", &members).is_err());
        let forged = vec![leaf("alice#a1", 7), leaf("bob#b1", 2)];
        assert!(check_welcome(&roster(), "c", "c", "bob#b1", &forged).is_err());
        let stranger = vec![leaf("mallory#m1", 7), leaf("bob#b1", 2)];
        assert!(check_welcome(&roster(), "c", "c", "bob#b1", &stranger).is_err());
    }
}
