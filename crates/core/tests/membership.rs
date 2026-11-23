//! Membership changes that clients must refuse, and the honest paths that
//! must keep working. Each attack here used to succeed.

use openmls::prelude::tls_codec::Serialize as _;
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use thrum_core::membership::ServerRoster;
use thrum_core::mls::{CIPHERSUITE, DeviceCrypto, Incoming};

fn device(identity: &str) -> DeviceCrypto {
    DeviceCrypto::generate(identity.to_string())
}

/// The server's view: these devices are active members, keys bound.
fn roster_of(devices: &[&DeviceCrypto]) -> ServerRoster {
    let mut roster = ServerRoster::new();
    for device in devices {
        roster.add_device(device.identity(), Some(device.signature_key()));
    }
    roster
}

/// A key package whose basic credential carries arbitrary identity bytes.
fn raw_key_package(identity: Vec<u8>) -> Vec<u8> {
    let provider = OpenMlsRustCrypto::default();
    let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).unwrap();
    let credential_with_key = CredentialWithKey {
        credential: BasicCredential::new(identity).into(),
        signature_key: signer.to_public_vec().into(),
    };
    KeyPackage::builder()
        .build(CIPHERSUITE, &provider, &signer, credential_with_key)
        .unwrap()
        .key_package()
        .tls_serialize_detached()
        .unwrap()
}

fn rejected(incoming: Incoming) -> bool {
    matches!(incoming, Incoming::Rejected(_))
}

/// `admin` creates `conv` and adds `others`; each of them joins.
fn group(conv: &str, admin: &DeviceCrypto, others: &[&DeviceCrypto], roster: &ServerRoster) {
    admin.create_group(conv).unwrap();
    let packages: Vec<Vec<u8>> = others
        .iter()
        .map(|d| d.new_key_package().unwrap())
        .collect();
    let (_commit, welcome) = admin.add_members(conv, &packages).unwrap();
    let welcome = welcome.unwrap();
    for other in others {
        assert_eq!(other.process_welcome(conv, &welcome, roster).unwrap(), conv);
    }
}

#[test]
fn a_member_cannot_add_a_leaf_that_claims_another_member() {
    let conv = "conv-ghost";
    let alice = device("alice#a1");
    let mallory = device("mallory#m1");
    let carol = device("carol#c1");
    let roster = roster_of(&[&alice, &mallory, &carol]);
    group(conv, &alice, &[&mallory, &carol], &roster);

    let ghost = device("alice#a1");
    let (commit, _welcome) = mallory
        .add_members(conv, &[ghost.new_key_package().unwrap()])
        .unwrap();
    assert!(rejected(
        alice.process_incoming(conv, &commit, &roster).unwrap()
    ));
    assert!(rejected(
        carol.process_incoming(conv, &commit, &roster).unwrap()
    ));
    assert_eq!(carol.group_members(conv).unwrap().len(), 3);

    // Nothing was merged: carol still reads alice in the current epoch.
    let wire = alice.encrypt(conv, b"still the real group").unwrap();
    assert!(matches!(
        carol.process_incoming(conv, &wire, &roster).unwrap(),
        Incoming::Application { sender, .. } if sender == "alice#a1"
    ));
}

#[test]
fn a_member_cannot_take_a_listed_device_slot_with_its_own_key() {
    let conv = "conv-slot";
    let alice = device("alice#a1");
    let alice_phone = device("alice#a2");
    let mallory = device("mallory#m1");
    let carol = device("carol#c1");
    let roster = roster_of(&[&alice, &alice_phone, &mallory, &carol]);
    group(conv, &alice, &[&mallory, &carol], &roster);

    // alice#a2 is listed but not yet in the group; mallory forges it.
    let forged = device("alice#a2");
    let (commit, _welcome) = mallory
        .add_members(conv, &[forged.new_key_package().unwrap()])
        .unwrap();
    assert!(rejected(
        carol.process_incoming(conv, &commit, &roster).unwrap()
    ));

    // The real device, with its registered key, is accepted. (Mallory's
    // own state moved on when she merged her forged commit, so another
    // member makes the honest add.)
    let (commit, _welcome) = alice
        .add_members(conv, &[alice_phone.new_key_package().unwrap()])
        .unwrap();
    assert_eq!(
        carol.process_incoming(conv, &commit, &roster).unwrap(),
        Incoming::Commit
    );
}

#[test]
fn leaves_with_malformed_identities_are_refused() {
    let identities = [
        b"thrum".to_vec(),
        vec![0xff, 0xfe, b'#', b'x'],
        b"alice#a1#x".to_vec(),
    ];
    for (n, identity) in identities.into_iter().enumerate() {
        let conv = format!("conv-malformed-{n}");
        let alice = device("alice#a1");
        let mallory = device("mallory#m1");
        let roster = roster_of(&[&alice, &mallory]);
        group(&conv, &alice, &[&mallory], &roster);
        let (commit, _welcome) = mallory
            .add_members(&conv, &[raw_key_package(identity.clone())])
            .unwrap();
        assert!(
            rejected(alice.process_incoming(&conv, &commit, &roster).unwrap()),
            "{identity:?}"
        );
        assert_eq!(alice.group_members(&conv).unwrap().len(), 2);
    }
}

#[test]
fn a_member_cannot_remove_someone_the_server_still_lists() {
    let conv = "conv-kick";
    let alice = device("alice#a1");
    let mallory = device("mallory#m1");
    let carol = device("carol#c1");
    let roster = roster_of(&[&alice, &mallory, &carol]);
    group(conv, &alice, &[&mallory, &carol], &roster);

    let kick = mallory.remove_member(conv, "carol#c1").unwrap();
    assert!(rejected(
        alice.process_incoming(conv, &kick, &roster).unwrap()
    ));
    assert!(rejected(
        carol.process_incoming(conv, &kick, &roster).unwrap()
    ));
    assert!(carol.has_group(conv).unwrap());
}

#[test]
fn a_removed_device_drops_its_group_and_can_be_added_back() {
    let conv = "conv-return";
    let alice = device("alice#a1");
    let carol = device("carol#c1");
    let full = roster_of(&[&alice, &carol]);
    group(conv, &alice, &[&carol], &full);

    // The server removes carol; alice's sync removes her leaf.
    let without = roster_of(&[&alice]);
    let remove = alice.remove_member(conv, "carol#c1").unwrap();
    assert_eq!(
        carol.process_incoming(conv, &remove, &without).unwrap(),
        Incoming::Commit
    );
    assert!(
        !carol.has_group(conv).unwrap(),
        "the dead group must be dropped"
    );

    // Carol is added back on the server and in MLS.
    let (_commit, welcome) = alice
        .add_members(conv, &[carol.new_key_package().unwrap()])
        .unwrap();
    assert_eq!(
        carol
            .process_welcome(conv, &welcome.unwrap(), &full)
            .unwrap(),
        conv
    );
    let wire = alice.encrypt(conv, b"welcome back").unwrap();
    assert!(matches!(
        carol.process_incoming(conv, &wire, &full).unwrap(),
        Incoming::Application { .. }
    ));
}

#[test]
fn a_commit_refused_on_a_stale_roster_applies_after_a_refresh() {
    let conv = "conv-refresh";
    let alice = device("alice#a1");
    let bob = device("bob#b1");
    let dave = device("dave#d1");
    let before = roster_of(&[&alice, &bob]);
    group(conv, &alice, &[&bob], &before);

    let (commit, _welcome) = alice
        .add_members(conv, &[dave.new_key_package().unwrap()])
        .unwrap();
    assert!(rejected(
        bob.process_incoming(conv, &commit, &before).unwrap()
    ));
    let after = roster_of(&[&alice, &bob, &dave]);
    assert_eq!(
        bob.process_incoming(conv, &commit, &after).unwrap(),
        Incoming::Commit
    );
    assert_eq!(bob.group_members(conv).unwrap().len(), 3);
}

#[test]
fn a_welcome_for_another_conversation_is_refused_and_the_real_one_still_works() {
    let real = "conv-real";
    let alice = device("alice#a1");
    let bob = device("bob#b1");
    let mallory = device("mallory#m1");
    let roster = roster_of(&[&alice, &bob]);

    // Mallory squats the real id with a leaf that claims alice and sends
    // the welcome inside another conversation.
    let fake_alice = device("alice#a1");
    fake_alice.create_group(real).unwrap();
    let (_c, squat) = fake_alice
        .add_members(real, &[bob.new_key_package().unwrap()])
        .unwrap();
    let squat = squat.unwrap();
    let elsewhere = roster_of(&[&mallory, &bob, &fake_alice]);
    assert!(
        bob.process_welcome("conv-other", &squat, &elsewhere)
            .is_err()
    );
    // Relayed in the real conversation, the forged key gives it away.
    assert!(bob.process_welcome(real, &squat, &roster).is_err());
    assert!(!bob.has_group(real).unwrap());

    // The real admin's welcome joins.
    alice.create_group(real).unwrap();
    let (_c, welcome) = alice
        .add_members(real, &[bob.new_key_package().unwrap()])
        .unwrap();
    assert_eq!(
        bob.process_welcome(real, &welcome.unwrap(), &roster)
            .unwrap(),
        real
    );
}

#[test]
fn a_failed_welcome_does_not_use_up_the_key_package() {
    let conv = "conv-kp";
    let alice = device("alice#a1");
    let bob = device("bob#b1");
    let roster = roster_of(&[&alice, &bob]);
    alice.create_group(conv).unwrap();
    let (_c, welcome) = alice
        .add_members(conv, &[bob.new_key_package().unwrap()])
        .unwrap();
    let welcome = welcome.unwrap();
    let mut corrupted = welcome.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0x01;
    assert!(bob.process_welcome(conv, &corrupted, &roster).is_err());
    assert_eq!(bob.process_welcome(conv, &welcome, &roster).unwrap(), conv);
}

#[test]
fn a_held_message_from_a_removed_device_is_not_attributed_to_its_successor() {
    let conv = "conv-epoch";
    let alice = device("alice#a1");
    let bob = device("bob#b1");
    let dave = device("dave#d1");
    let carol = device("carol#c1");
    let roster = roster_of(&[&alice, &bob, &dave]);
    group(conv, &alice, &[&bob, &dave], &roster);

    let held = bob.encrypt(conv, b"signed: bob").unwrap();
    let without_bob = roster_of(&[&alice, &dave, &carol]);
    let remove = alice.remove_member(conv, "bob#b1").unwrap();
    assert_eq!(
        dave.process_incoming(conv, &remove, &without_bob).unwrap(),
        Incoming::Commit
    );
    let (add, _w) = alice
        .add_members(conv, &[carol.new_key_package().unwrap()])
        .unwrap();
    assert_eq!(
        dave.process_incoming(conv, &add, &without_bob).unwrap(),
        Incoming::Commit
    );

    assert_eq!(
        dave.process_incoming(conv, &held, &without_bob).unwrap(),
        Incoming::Ignored
    );
}

#[test]
fn a_commit_from_an_account_that_has_since_left_still_applies() {
    let conv = "conv-departed";
    let alice = device("alice#a1");
    let bob = device("bob#b1");
    let carol = device("carol#c1");
    let dave = device("dave#d1");
    let roster = roster_of(&[&alice, &bob, &carol, &dave]);
    group(conv, &alice, &[&bob, &carol], &roster);

    // Alice adds dave, then deletes her account before bob catches up.
    let (add, _welcome) = alice
        .add_members(conv, &[dave.new_key_package().unwrap()])
        .unwrap();
    let after = roster_of(&[&bob, &carol, &dave]);
    assert_eq!(
        bob.process_incoming(conv, &add, &after).unwrap(),
        Incoming::Commit
    );

    // Her leftover leaf still cannot remove someone the server lists.
    let kick = alice.remove_member(conv, "carol#c1").unwrap();
    assert!(rejected(bob.process_incoming(conv, &kick, &after).unwrap()));
}
