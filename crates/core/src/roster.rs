//! Decide which device identities a client should add, hold, or remove
//! while bringing an MLS group in line with the server.
//!
//! The server chooses the device list, so a client that adds every listed
//! device hands a compromised server a way into the group. Devices that
//! appear after a member was explicitly added stay pending until this user
//! trusts them. Devices the server no longer lists as active are removals:
//! that is how revocation leaves the group.

use std::collections::BTreeSet;

/// One change to apply to a conversation's MLS roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterAction {
    /// Fetch a key package and add this identity.
    Add(String),
    /// The server lists this identity, and this user has not trusted it.
    Pending(String),
    /// This identity is in the group and should be removed.
    Remove(String),
}

/// Identities in `group_members` whose account is no longer in
/// `server_members`. The caller's own identity is never returned.
pub fn departed_members(
    self_identity: &str,
    group_members: &[String],
    server_members: &[String],
) -> Vec<String> {
    group_members
        .iter()
        .filter(|identity| {
            if identity.as_str() == self_identity {
                return false;
            }
            let Some((username, _)) = crate::mls::split_identity(identity) else {
                return false;
            };
            !server_members
                .iter()
                .any(|member| member.eq_ignore_ascii_case(username))
        })
        .cloned()
        .collect()
}

/// Plan adds, pending approvals, and revocations for one account.
///
/// `admit_untrusted` is set for an account the user just explicitly added.
/// `known_account` is set once this client has already seen the account in
/// this conversation. The first time an account appears — the user named
/// them, or they redeemed an invite — every device they have then is
/// trusted on first use. A device that shows up after that stays
/// [`RosterAction::Pending`] until `trusted` contains it. Group members of
/// this account whose device is not in `active_device_ids` are removals
/// (the device was revoked or deleted).
pub fn plan_roster(
    self_identity: &str,
    group_members: &[String],
    username: &str,
    active_device_ids: &[String],
    trusted: &BTreeSet<String>,
    admit_untrusted: bool,
    known_account: bool,
) -> Vec<RosterAction> {
    let active: BTreeSet<String> = active_device_ids
        .iter()
        .filter(|device_id| !device_id.is_empty())
        .map(|device_id| format!("{username}#{device_id}"))
        .collect();

    let mut actions = Vec::new();
    for identity in group_members {
        if identity == self_identity {
            continue;
        }
        let Some((member, _)) = crate::mls::split_identity(identity) else {
            continue;
        };
        if !member.eq_ignore_ascii_case(username) {
            continue;
        }
        if !active.contains(identity) {
            actions.push(RosterAction::Remove(identity.clone()));
        }
    }
    for identity in &active {
        if identity == self_identity || group_members.iter().any(|member| member == identity) {
            continue;
        }
        // A leaf that is already in the group means this client has seen the
        // account, even if that fact has not been written to disk yet.
        let in_group = group_members.iter().any(|existing| {
            crate::mls::split_identity(existing)
                .is_some_and(|(member, _)| member.eq_ignore_ascii_case(username))
        });
        let first_contact = admit_untrusted || !(known_account || in_group);
        if first_contact || trusted.contains(identity) {
            actions.push(RosterAction::Add(identity.clone()));
        } else {
            actions.push(RosterAction::Pending(identity.clone()));
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn later_devices_wait_and_revoked_devices_are_removed() {
        let actions = plan_roster(
            "alice#laptop",
            &["alice#laptop".into(), "bob#phone".into()],
            "bob",
            &["phone".into(), "tablet".into()],
            &trusted(&[]),
            false,
            false,
        );
        assert_eq!(actions, vec![RosterAction::Pending("bob#tablet".into())]);

        let actions = plan_roster(
            "alice#laptop",
            &[
                "alice#laptop".into(),
                "bob#phone".into(),
                "bob#tablet".into(),
            ],
            "bob",
            &["phone".into()],
            &trusted(&[]),
            false,
            true,
        );
        assert_eq!(actions, vec![RosterAction::Remove("bob#tablet".into())]);
    }

    #[test]
    fn an_explicit_add_admits_current_devices_only() {
        let actions = plan_roster(
            "alice#laptop",
            &["alice#laptop".into()],
            "bob",
            &["phone".into()],
            &trusted(&[]),
            true,
            false,
        );
        assert_eq!(actions, vec![RosterAction::Add("bob#phone".into())]);
    }

    #[test]
    fn the_first_sight_of_an_account_admits_its_current_devices() {
        let actions = plan_roster(
            "alice#laptop",
            &["alice#laptop".into(), "bob#phone".into()],
            "carol",
            &["web".into()],
            &trusted(&[]),
            false,
            false,
        );
        assert_eq!(actions, vec![RosterAction::Add("carol#web".into())]);
    }

    #[test]
    fn a_replacement_device_stays_pending_after_the_leaf_is_gone() {
        let actions = plan_roster(
            "alice#laptop",
            &["alice#laptop".into()],
            "bob",
            &["tablet".into()],
            &trusted(&[]),
            false,
            true,
        );
        assert_eq!(actions, vec![RosterAction::Pending("bob#tablet".into())]);
    }

    #[test]
    fn a_trusted_device_is_added_on_a_later_sync() {
        let actions = plan_roster(
            "alice#laptop",
            &["alice#laptop".into(), "bob#phone".into()],
            "bob",
            &["phone".into(), "tablet".into()],
            &trusted(&["bob#tablet"]),
            false,
            true,
        );
        assert_eq!(actions, vec![RosterAction::Add("bob#tablet".into())]);
    }

    #[test]
    fn departed_accounts_are_removed_except_self() {
        let gone = departed_members(
            "alice#laptop",
            &[
                "alice#laptop".into(),
                "bob#phone".into(),
                "carol#web".into(),
            ],
            &["alice".into(), "carol".into()],
        );
        assert_eq!(gone, vec!["bob#phone".to_string()]);
    }
}
