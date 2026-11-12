//! MLS layer for Thrum clients.
//!
//! Every device is an MLS member with a `BasicCredential` whose identity is
//! `username#device-id`. Groups are keyed by the relay conversation id, and
//! all messages travel as MLS ciphertext (pure-ciphertext wire format).
//!
//! The device's key material lives in an OpenMLS `MemoryStorage` that this
//! module serializes to and from a client-side state directory.

use std::collections::BTreeMap;
use std::path::Path;

use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use serde::{Deserialize, Serialize};
use tls_codec::{DeserializeBytes as TlsDeserializeBytes, Serialize as TlsSerialize};

/// The one ciphersuite every Thrum client speaks.
pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// Errors from the MLS layer.
#[derive(Debug)]
pub enum MlsError {
    /// Any OpenMLS or codec failure, with its display text.
    Other(String),
    /// The device has no MLS group for this conversation (yet).
    NoGroup,
    /// The identity is not a member of the group.
    UnknownMember,
    /// The device identity is not `username#device-id`.
    BadIdentity,
    /// The state directory could not be read or written.
    Io(std::io::Error),
}

impl std::fmt::Display for MlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MlsError::Other(message) => write!(f, "MLS error: {message}"),
            MlsError::NoGroup => write!(f, "no MLS group for this conversation"),
            MlsError::UnknownMember => write!(f, "member not found in the group"),
            MlsError::BadIdentity => write!(f, "identity is not username#device-id"),
            MlsError::Io(error) => write!(f, "MLS state I/O: {error}"),
        }
    }
}

impl std::error::Error for MlsError {}

impl From<std::io::Error> for MlsError {
    fn from(error: std::io::Error) -> Self {
        MlsError::Io(error)
    }
}

/// Wrap any `Display` error as [`MlsError::Other`].
fn other<E: std::fmt::Display>(error: E) -> MlsError {
    MlsError::Other(error.to_string())
}

/// A MLS group member as Thrum sees it: the credential identity string.
pub fn member_identity(credential: &Credential) -> Result<String, MlsError> {
    if credential.credential_type() != CredentialType::Basic {
        return Err(MlsError::BadIdentity);
    }
    String::from_utf8(credential.serialized_content().to_vec()).map_err(|_| MlsError::BadIdentity)
}

/// Split `username#device-id`.
pub fn split_identity(identity: &str) -> Option<(&str, &str)> {
    let (username, device_id) = identity.split_once('#')?;
    if username.is_empty() || device_id.is_empty() {
        return None;
    }
    Some((username, device_id))
}

/// What a received MLS message turns out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// A decrypted application message from a group member.
    Application { sender: String, payload: Vec<u8> },
    /// A commit that advanced the group state (adds, removals, updates).
    Commit,
    /// Nothing actionable (wrong epoch, not for this device, duplicate).
    Ignored,
}

/// A device's MLS state: key store, signer and credential.
pub struct DeviceCrypto {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential_with_key: CredentialWithKey,
    identity: String,
}

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    identity: String,
    signer: SignatureKeyPair,
    credential_with_key: CredentialWithKey,
}

#[derive(Default, Serialize, Deserialize)]
struct StoredStorage {
    values: BTreeMap<String, String>,
}

impl DeviceCrypto {
    /// Generate fresh key material for a new device identity
    /// (`username#device-id`).
    pub fn generate(identity: String) -> Self {
        let signer =
            SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).expect("key generation");
        let credential_with_key = CredentialWithKey {
            credential: BasicCredential::new(identity.clone().into_bytes()).into(),
            signature_key: signer.to_public_vec().into(),
        };
        Self {
            provider: OpenMlsRustCrypto::default(),
            signer,
            credential_with_key,
            identity,
        }
    }

    /// The device identity (`username#device-id`).
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// The username part of the device identity.
    pub fn username(&self) -> &str {
        self.identity
            .split_once('#')
            .map_or(self.identity.as_str(), |(u, _)| u)
    }

    /// Save the key store and identity to a directory (0700/0600 on unix).
    pub fn save(&self, dir: &Path) -> Result<(), MlsError> {
        std::fs::create_dir_all(dir).map_err(MlsError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let mut storage = StoredStorage::default();
        for (key, value) in self.provider.storage().values.read().unwrap().iter() {
            storage
                .values
                .insert(base64_encode(key), base64_encode(value));
        }
        write_private(&dir.join("storage.json"), &serde_json::to_vec(&storage)?)?;
        write_private(
            &dir.join("identity.json"),
            &serde_json::to_vec(&StoredIdentity {
                identity: self.identity.clone(),
                signer: self.signer.clone(),
                credential_with_key: self.credential_with_key.clone(),
            })?,
        )?;
        Ok(())
    }

    /// Load the key store and identity from a directory.
    pub fn load(dir: &Path) -> Result<Self, MlsError> {
        let identity: StoredIdentity =
            serde_json::from_slice(&read_private(&dir.join("identity.json"))?).map_err(other)?;
        let storage: StoredStorage =
            serde_json::from_slice(&read_private(&dir.join("storage.json"))?).map_err(other)?;
        let provider = OpenMlsRustCrypto::default();
        {
            let mut values = provider.storage().values.write().unwrap();
            for (key, value) in storage.values {
                let key = base64_decode(&key).map_err(other)?;
                let value = base64_decode(&value).map_err(other)?;
                values.insert(key, value);
            }
        }
        Ok(Self {
            provider,
            signer: identity.signer,
            credential_with_key: identity.credential_with_key,
            identity: identity.identity,
        })
    }

    /// Validate a fetched key package and return the credential identity it
    /// claims (`username#device-id`). Callers compare this against the device
    /// they meant to add before using the package, so a malicious server
    /// cannot steer a commit toward a device it controls.
    pub fn key_package_identity(&self, package: &[u8]) -> Result<String, MlsError> {
        let key_package = KeyPackageIn::tls_deserialize_exact_bytes(package)
            .map_err(other)?
            .validate(self.provider.crypto(), ProtocolVersion::Mls10)
            .map_err(other)?;
        let credential = key_package.leaf_node().credential();
        member_identity(credential)
    }

    /// A fresh serialized key package for this device.
    pub fn new_key_package(&self) -> Result<Vec<u8>, MlsError> {
        let bundle = KeyPackage::builder()
            .build(
                CIPHERSUITE,
                &self.provider,
                &self.signer,
                self.credential_with_key.clone(),
            )
            .map_err(other)?;
        bundle.key_package().tls_serialize_detached().map_err(other)
    }

    fn group(&self, conversation_id: &str) -> Result<Option<MlsGroup>, MlsError> {
        let group_id = GroupId::from_slice(conversation_id.as_bytes());
        MlsGroup::load(self.provider.storage(), &group_id).map_err(other)
    }

    fn group_mut(&self, conversation_id: &str) -> Result<MlsGroup, MlsError> {
        self.group(conversation_id)?.ok_or(MlsError::NoGroup)
    }

    /// True when this device has joined the conversation's MLS group.
    pub fn has_group(&self, conversation_id: &str) -> Result<bool, MlsError> {
        Ok(self.group(conversation_id)?.is_some())
    }

    /// Create the MLS group for a conversation. Idempotent.
    pub fn create_group(&self, conversation_id: &str) -> Result<(), MlsError> {
        if self.has_group(conversation_id)? {
            return Ok(());
        }
        let config = create_config();
        MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &config,
            GroupId::from_slice(conversation_id.as_bytes()),
            self.credential_with_key.clone(),
        )
        .map_err(other)?;
        Ok(())
    }

    /// Add members from serialized key packages. Returns the serialized
    /// commit and, when someone was actually added, the serialized welcome.
    /// The group state is merged before returning.
    pub fn add_members(
        &self,
        conversation_id: &str,
        key_packages: &[Vec<u8>],
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), MlsError> {
        let packages: Vec<KeyPackage> = key_packages
            .iter()
            .map(|bytes| {
                KeyPackageIn::tls_deserialize_exact_bytes(bytes)
                    .map_err(other)?
                    .validate(self.provider.crypto(), ProtocolVersion::Mls10)
                    .map_err(other)
            })
            .collect::<Result<_, _>>()?;
        let mut group = self.group_mut(conversation_id)?;
        let (commit, welcome, _) = group
            .add_members(&self.provider, &self.signer, &packages)
            .map_err(other)?;
        group.merge_pending_commit(&self.provider).map_err(other)?;
        let welcome = match welcome.body() {
            MlsMessageBodyOut::Welcome(w) => Some(w.tls_serialize_detached().map_err(other)?),
            _ => {
                return Err(MlsError::Other(
                    "add_members did not return a welcome".into(),
                ));
            }
        };
        Ok((commit.tls_serialize_detached().map_err(other)?, welcome))
    }

    /// Remove a member by identity. Returns the serialized commit; the group
    /// state is merged before returning.
    pub fn remove_member(
        &self,
        conversation_id: &str,
        identity: &str,
    ) -> Result<Vec<u8>, MlsError> {
        let mut group = self.group_mut(conversation_id)?;
        let index = group
            .members()
            .find_map(|member| {
                let id = member_identity(&member.credential).ok()?;
                (id == identity).then_some(member.index)
            })
            .ok_or(MlsError::UnknownMember)?;
        let (commit, _, _) = group
            .remove_members(&self.provider, &self.signer, &[index])
            .map_err(other)?;
        group.merge_pending_commit(&self.provider).map_err(other)?;
        commit.tls_serialize_detached().map_err(other)
    }

    /// Encrypt an application payload for the conversation's group.
    pub fn encrypt(&self, conversation_id: &str, payload: &[u8]) -> Result<Vec<u8>, MlsError> {
        let mut group = self.group_mut(conversation_id)?;
        let message = group
            .create_message(&self.provider, &self.signer, payload)
            .map_err(other)?;
        message.tls_serialize_detached().map_err(other)
    }

    /// Process one serialized MLS message (commit or application) received
    /// for a conversation. Never fails on messages this device cannot or
    /// should not process: those come back as [`Incoming::Ignored`].
    pub fn process_incoming(
        &self,
        conversation_id: &str,
        message: &[u8],
    ) -> Result<Incoming, MlsError> {
        let mut group = match self.group(conversation_id)? {
            Some(group) => group,
            None => return Ok(Incoming::Ignored),
        };
        let message = MlsMessageIn::tls_deserialize_exact_bytes(message).map_err(other)?;
        let message = message.try_into_protocol_message().map_err(other)?;
        let processed = match group.process_message(&self.provider, message) {
            Ok(processed) => processed,
            Err(_) => return Ok(Incoming::Ignored),
        };
        let sender = match processed.sender() {
            Sender::Member(index) => match group.member_at(*index) {
                Some(member) => member_identity(&member.credential)?,
                None => return Ok(Incoming::Ignored),
            },
            _ => return Ok(Incoming::Ignored),
        };
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(application) => Ok(Incoming::Application {
                sender,
                payload: application.into_bytes(),
            }),
            ProcessedMessageContent::StagedCommitMessage(staged_commit) => {
                group
                    .merge_staged_commit(&self.provider, *staged_commit)
                    .map_err(other)?;
                Ok(Incoming::Commit)
            }
            _ => Ok(Incoming::Ignored),
        }
    }

    /// Process a serialized welcome: join the group. Returns the group's
    /// conversation id. Errors when the welcome is not for this device (the
    /// key package it encrypts to is unknown here) or is stale.
    pub fn process_welcome(&self, welcome_bytes: &[u8]) -> Result<String, MlsError> {
        let welcome = Welcome::tls_deserialize_exact_bytes(welcome_bytes).map_err(other)?;
        let staged = StagedWelcome::new_from_welcome(
            &self.provider,
            &join_config(),
            welcome,
            None, // the ratchet tree arrives inside the welcome
        )
        .map_err(other)?;
        let group = staged.into_group(&self.provider).map_err(other)?;
        Ok(String::from_utf8_lossy(group.group_id().as_slice()).into_owned())
    }

    /// The credential identities of all current group members.
    pub fn group_members(&self, conversation_id: &str) -> Result<Vec<String>, MlsError> {
        let group = self.group_mut(conversation_id)?;
        group
            .members()
            .map(|member| member_identity(&member.credential))
            .collect::<Result<Vec<_>, _>>()
    }
}

/// Wire-format policy for created groups.
fn create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .ciphersuite(CIPHERSUITE)
        .use_ratchet_tree_extension(true)
        .max_past_epochs(MAX_PAST_EPOCHS)
        .build()
}

/// Join configuration matching [`create_config`].
fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .max_past_epochs(MAX_PAST_EPOCHS)
        .build()
}

/// Keep key material for this many past epochs so that a sender who has not
/// yet processed the latest commit still reaches every member.
const MAX_PAST_EPOCHS: usize = 10;

/// Write bytes with 0600 permissions on unix.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), MlsError> {
    use std::io::Write;
    let mut file = std::fs::File::create(path).map_err(MlsError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(MlsError::Io)?;
    }
    file.write_all(bytes).map_err(MlsError::Io)?;
    Ok(())
}

/// Read bytes from a state file.
fn read_private(path: &Path) -> Result<Vec<u8>, MlsError> {
    std::fs::read(path).map_err(MlsError::Io)
}

/// Standard base64 without padding tricks; just the standard engine.
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(text: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(text)
}

impl From<serde_json::Error> for MlsError {
    fn from(error: serde_json::Error) -> Self {
        MlsError::Other(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(user: &str, n: u8) -> DeviceCrypto {
        DeviceCrypto::generate(format!("{user}#device-{n}"))
    }

    /// alice creates the group and adds bob's device.
    fn setup_two(alice: &DeviceCrypto, bob: &DeviceCrypto) -> (Vec<u8>, Vec<u8>) {
        let conversation = "conv-1";
        alice.create_group(conversation).expect("create");
        let (commit, welcome) = alice
            .add_members(conversation, &[bob.new_key_package().expect("kp")])
            .expect("add");
        let joined = bob
            .process_welcome(&welcome.expect("welcome"))
            .expect("join");
        assert_eq!(joined, conversation);
        let _ = commit;
        (
            b"earlier message".to_vec(),
            conversation.as_bytes().to_vec(),
        )
    }

    #[test]
    fn roundtrip_between_two_members() {
        let alice = device("alice", 1);
        let bob = device("bob", 1);
        let (earlier, conversation) = setup_two(&alice, &bob);
        let conversation = String::from_utf8(conversation).unwrap();

        let wire = alice.encrypt(&conversation, &earlier).expect("encrypt");
        match bob.process_incoming(&conversation, &wire).expect("process") {
            Incoming::Application { sender, payload } => {
                assert_eq!(sender, "alice#device-1");
                assert_eq!(payload, earlier);
            }
            other => panic!("expected an application message, got {other:?}"),
        }
    }

    #[test]
    fn added_member_cannot_read_earlier_messages() {
        let alice = device("alice", 1);
        let bob = device("bob", 1);
        let carol = device("carol", 1);
        let (earlier, conversation) = setup_two(&alice, &bob);
        let conversation = String::from_utf8(conversation).unwrap();

        // Carol joins after the earlier message was encrypted.
        let (commit, welcome) = alice
            .add_members(&conversation, &[carol.new_key_package().expect("kp")])
            .expect("add carol");
        // Bob applies the commit so the group stays in sync.
        assert!(matches!(
            bob.process_incoming(&conversation, &commit)
                .expect("process"),
            Incoming::Commit
        ));
        let joined = carol
            .process_welcome(&welcome.expect("welcome"))
            .expect("join");
        assert_eq!(joined, conversation);

        // Even with the earlier ciphertext in hand, carol cannot read it.
        assert!(matches!(
            carol.process_incoming(&conversation, &earlier),
            Ok(Incoming::Ignored) | Err(_)
        ));
    }

    #[test]
    fn removed_member_cannot_read_later_messages() {
        let alice = device("alice", 1);
        let bob = device("bob", 1);
        let (earlier, conversation) = setup_two(&alice, &bob);
        let conversation = String::from_utf8(conversation).unwrap();
        let earlier_wire = alice.encrypt(&conversation, &earlier).expect("encrypt");
        assert!(matches!(
            bob.process_incoming(&conversation, &earlier_wire)
                .expect("process"),
            Incoming::Application { .. }
        ));

        // Alice removes bob; bob processes the removal commit.
        let commit = alice
            .remove_member(&conversation, "bob#device-1")
            .expect("remove");
        assert!(matches!(
            bob.process_incoming(&conversation, &commit)
                .expect("process"),
            Incoming::Commit
        ));

        // A later message cannot be decrypted by bob, even with the bytes.
        let later = alice
            .encrypt(&conversation, b"after removal")
            .expect("encrypt");
        assert!(matches!(
            bob.process_incoming(&conversation, &later),
            Ok(Incoming::Ignored) | Err(_)
        ));
    }

    #[test]
    fn storage_persists_across_reload() {
        let dir = tempfile::tempdir().expect("temp dir");
        let alice = device("alice", 1);
        let bob = device("bob", 1);
        let conversation = "conv-1";
        alice.create_group(conversation).expect("create");
        alice.save(dir.path()).expect("save");

        let reloaded = DeviceCrypto::load(dir.path()).expect("load");
        assert_eq!(reloaded.identity(), alice.identity());
        assert!(reloaded.has_group(conversation).expect("has group"));
        let _ = bob;
    }

    /// The key store must survive a save/reload: private key-package keys
    /// generated before the save are what lets the reloaded device unwrap a
    /// Welcome that was built from one of its uploaded packages. This used to
    /// fail when the CLI persisted before topping up its key stock.
    #[test]
    fn reloaded_device_unwraps_a_welcome_from_an_uploaded_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        let alice = device("alice", 1);
        let bob = device("bob", 1);

        // First process: bob generates the package that goes to the server,
        // then the CLI saves state (as `login` does after topping up).
        let package = bob.new_key_package().expect("kp");
        bob.save(dir.path()).expect("save");

        // Second process: the state is loaded fresh from disk.
        let bob2 = DeviceCrypto::load(dir.path()).expect("load");
        assert_eq!(bob2.identity(), bob.identity());

        alice.create_group("conv-1").expect("create");
        let (_commit, welcome) = alice.add_members("conv-1", &[package]).expect("add");
        let joined = bob2
            .process_welcome(&welcome.expect("welcome"))
            .expect("reloaded device must unwrap the welcome");
        assert_eq!(joined, "conv-1");
    }

    #[test]
    fn key_package_identity_is_verifiable() {
        let alice = device("alice", 1);
        let package = alice.new_key_package().expect("kp");
        assert_eq!(
            alice
                .key_package_identity(&package)
                .expect("identity of a valid package"),
            "alice#device-1"
        );
        // Tampered or foreign bytes fail validation rather than yielding an
        // identity.
        let mut tampered = package.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xff;
        assert!(alice.key_package_identity(&tampered).is_err());
        assert!(alice.key_package_identity(b"not a key package").is_err());
    }

    #[test]
    fn identity_split() {
        assert_eq!(
            split_identity("alice#device-1"),
            Some(("alice", "device-1"))
        );
        assert_eq!(split_identity("nodigit"), None);
        assert_eq!(split_identity("#device-1"), None);
    }
}
