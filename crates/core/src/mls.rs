//! MLS layer for Thrum clients.
//!
//! Every device is an MLS member with a `BasicCredential` whose identity is
//! `username#device-id`. Groups are keyed by the relay conversation id, and
//! all messages travel as MLS ciphertext (pure-ciphertext wire format).
//!
//! The device's key material lives in an OpenMLS `MemoryStorage` that this
//! module serializes to and from a client-side state directory.

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use crate::membership::{CommitChanges, Leaf, ServerRoster, check_commit, check_welcome};
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

/// Run an OpenMLS parse over attacker-controllable bytes behind a panic
/// guard.
///
/// The TLS-codec layer under OpenMLS asserts on some malformed length
/// prefixes instead of returning an error (for example a vector length
/// larger than the remaining input). Every byte sequence handed to
/// [`key_package_identity`](DeviceCrypto::key_package_identity),
/// [`add_members`](DeviceCrypto::add_members),
/// [`process_incoming`](DeviceCrypto::process_incoming) and
/// [`process_welcome`](DeviceCrypto::process_welcome) can come from
/// another user or from the server, so such an assert would be a remote
/// crash. A caught panic surfaces as a typed error. Most guards wrap only
/// parsing and validation, before any group state is touched; the welcome
/// guard also covers the join itself, which is safe because
/// [`process_welcome`](DeviceCrypto::process_welcome) restores the
/// device's storage from a snapshot on any error. Panics from other,
/// already-validated steps stay loud, as they would be genuine bugs.
fn guard_panics<T>(work: impl FnOnce() -> Result<T, MlsError>) -> Result<T, MlsError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|panic| {
        let detail = panic
            .downcast_ref::<&'static str>()
            .map(|text| (*text).to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unparseable MLS input".to_string());
        Err(MlsError::Other(format!("malformed MLS input: {detail}")))
    })
}

/// Whether `storage` holds the bundle (private keys included) of the key
/// package `hash_ref` names. The bound brings the storage methods into
/// scope without a direct dependency on `openmls_traits`.
fn stored_key_package<S: openmls::storage::StorageProvider>(
    storage: &S,
    hash_ref: &KeyPackageRef,
) -> Result<bool, MlsError> {
    let bundle: Option<KeyPackageBundle> = storage
        .key_package(hash_ref)
        .map_err(|error| MlsError::Other(format!("{error:?}")))?;
    Ok(bundle.is_some())
}

/// A MLS group member as Thrum sees it: the credential identity string.
pub fn member_identity(credential: &Credential) -> Result<String, MlsError> {
    if credential.credential_type() != CredentialType::Basic {
        return Err(MlsError::BadIdentity);
    }
    String::from_utf8(credential.serialized_content().to_vec()).map_err(|_| MlsError::BadIdentity)
}

/// A leaf as the membership checks see it. Never fails: an identity that is
/// not a UTF-8 basic credential is kept, replaced lossily, and marked.
fn leaf_of(credential: &Credential, signature_key: &[u8]) -> Leaf {
    let valid = credential.credential_type() == CredentialType::Basic
        && std::str::from_utf8(credential.serialized_content()).is_ok();
    Leaf {
        identity: String::from_utf8_lossy(credential.serialized_content()).into_owned(),
        valid_utf8: valid,
        signature_key: signature_key.to_vec(),
    }
}

/// Validate a serialized key package without any device state and return
/// the identity it claims and its signature key. The server uses this to
/// bind each device to one signature key.
pub fn key_package_binding(package: &[u8]) -> Result<(String, Vec<u8>), MlsError> {
    let provider = OpenMlsRustCrypto::default();
    let key_package = guard_panics(|| {
        KeyPackageIn::tls_deserialize_exact_bytes(package)
            .map_err(other)?
            .validate(provider.crypto(), ProtocolVersion::Mls10)
            .map_err(other)
    })?;
    let leaf = key_package.leaf_node();
    let identity = member_identity(leaf.credential())?;
    Ok((identity, leaf.signature_key().as_slice().to_vec()))
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
    /// A commit that the server's roster does not allow. Nothing was
    /// merged and the device state is as it was before the call, so the
    /// caller may refresh the roster and try the same message again.
    Rejected(String),
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

/// What [`DeviceCrypto::to_json`] writes and [`DeviceCrypto::from_json`] reads.
#[derive(Serialize, Deserialize)]
struct Stored {
    identity: StoredIdentity,
    storage: StoredStorage,
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

    /// This device's signature public key.
    pub fn signature_key(&self) -> Vec<u8> {
        self.credential_with_key.signature_key.as_slice().to_vec()
    }

    /// A copy of every stored value, to restore if an operation on
    /// untrusted input has to be undone.
    fn snapshot(&self) -> std::collections::HashMap<Vec<u8>, Vec<u8>> {
        self.provider.storage().values.read().unwrap().clone()
    }

    fn restore(&self, snapshot: std::collections::HashMap<Vec<u8>, Vec<u8>>) {
        *self.provider.storage().values.write().unwrap() = snapshot;
    }

    /// The username part of the device identity.
    pub fn username(&self) -> &str {
        self.identity
            .split_once('#')
            .map_or(self.identity.as_str(), |(u, _)| u)
    }

    /// Serialize the key store and identity to a JSON string. This is what a
    /// client stores privately (a file on disk for the CLI, IndexedDB for the
    /// web client); it contains every private key of the device.
    pub fn to_json(&self) -> Result<String, MlsError> {
        let mut storage = StoredStorage::default();
        for (key, value) in self.provider.storage().values.read().unwrap().iter() {
            storage
                .values
                .insert(base64_encode(key), base64_encode(value));
        }
        serde_json::to_string(&Stored {
            identity: StoredIdentity {
                identity: self.identity.clone(),
                signer: self.signer.clone(),
                credential_with_key: self.credential_with_key.clone(),
            },
            storage,
        })
        .map_err(MlsError::from)
    }

    /// Reconstruct a device from [`DeviceCrypto::to_json`] output.
    pub fn from_json(json: &str) -> Result<Self, MlsError> {
        let stored: Stored = serde_json::from_str(json).map_err(other)?;
        let provider = OpenMlsRustCrypto::default();
        {
            let mut values = provider.storage().values.write().unwrap();
            for (key, value) in stored.storage.values {
                let key = base64_decode(&key).map_err(other)?;
                let value = base64_decode(&value).map_err(other)?;
                values.insert(key, value);
            }
        }
        Ok(Self {
            provider,
            signer: stored.identity.signer,
            credential_with_key: stored.identity.credential_with_key,
            identity: stored.identity.identity,
        })
    }

    /// Save the key store and identity to a directory (0700/0600 on unix).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self, dir: &Path) -> Result<(), MlsError> {
        std::fs::create_dir_all(dir).map_err(MlsError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        write_private(&dir.join("state.json"), self.to_json()?.as_bytes())
    }

    /// Load the key store and identity from a directory. Reads the single
    /// `state.json`; older state directories with `identity.json` and
    /// `storage.json` still load.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(dir: &Path) -> Result<Self, MlsError> {
        let state_path = dir.join("state.json");
        if state_path.exists() {
            let json = read_private(&state_path)?;
            let text =
                String::from_utf8(json).map_err(|_| MlsError::Other("bad state json".into()))?;
            return Self::from_json(&text);
        }
        // Legacy layout: two separate files.
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
        let key_package = guard_panics(|| {
            KeyPackageIn::tls_deserialize_exact_bytes(package)
                .map_err(other)?
                .validate(self.provider.crypto(), ProtocolVersion::Mls10)
                .map_err(other)
        })?;
        let credential = key_package.leaf_node().credential();
        member_identity(credential)
    }

    /// Validate a fetched key package and return the identity it claims and
    /// its signature key, so the caller can compare both with the server's
    /// roster before adding it.
    pub fn key_package_leaf(&self, package: &[u8]) -> Result<Leaf, MlsError> {
        let key_package = guard_panics(|| {
            KeyPackageIn::tls_deserialize_exact_bytes(package)
                .map_err(other)?
                .validate(self.provider.crypto(), ProtocolVersion::Mls10)
                .map_err(other)
        })?;
        let leaf = key_package.leaf_node();
        Ok(leaf_of(leaf.credential(), leaf.signature_key().as_slice()))
    }

    /// Check a key package the server returned for `identity` before this
    /// device adds it: the package must be valid, claim exactly that
    /// identity, and carry the signature key the server bound to it. The server
    /// picks which bytes to hand out, so this keeps a malicious server from
    /// steering a commit toward a device it controls.
    pub fn check_key_package(
        &self,
        package: &[u8],
        identity: &str,
        server: &ServerRoster,
    ) -> Result<(), String> {
        let leaf = self
            .key_package_leaf(package)
            .map_err(|error| error.to_string())?;
        if leaf.identity != identity {
            return Err(format!(
                "server returned a key package for {} instead of {identity}",
                leaf.identity
            ));
        }
        let changes = CommitChanges {
            sender: Some(Leaf::new(self.identity(), self.signature_key())),
            added: vec![leaf.clone()],
            members_after: vec![leaf],
            ..Default::default()
        };
        let mut with_self = server.clone();
        with_self.add_device(self.identity(), None);
        check_commit(&with_self, &changes)
    }

    /// Whether this device holds the private keys for `package`, one of its
    /// own key packages, so a Welcome built on it could be opened. A device
    /// restored from an older copy of its state does not hold the keys of
    /// packages it uploaded after that copy was taken. Bytes that are not a
    /// valid key package are not held. Only a failure to read the key store
    /// is an error, so a caller never withdraws a package it might hold.
    pub fn holds_key_package(&self, package: &[u8]) -> Result<bool, MlsError> {
        let hash_ref = guard_panics(|| {
            let key_package = KeyPackageIn::tls_deserialize_exact_bytes(package)
                .map_err(other)?
                .validate(self.provider.crypto(), ProtocolVersion::Mls10)
                .map_err(other)?;
            key_package.hash_ref(self.provider.crypto()).map_err(other)
        });
        match hash_ref {
            Ok(hash_ref) => stored_key_package(self.provider.storage(), &hash_ref),
            Err(_) => Ok(false),
        }
    }

    /// A fresh one-time key package for this device.
    pub fn new_key_package(&self) -> Result<Vec<u8>, MlsError> {
        self.build_key_package(false)
    }

    /// A last-resort key package. OpenMLS keeps its init key after a join,
    /// so the server can hand the same package out again once the one-time
    /// stock is gone.
    pub fn new_last_resort_key_package(&self) -> Result<Vec<u8>, MlsError> {
        self.build_key_package(true)
    }

    fn build_key_package(&self, last_resort: bool) -> Result<Vec<u8>, MlsError> {
        let mut builder = KeyPackage::builder();
        if last_resort {
            // The leaf has to advertise the extension or the commit that adds
            // this package is rejected.
            builder = builder
                .mark_as_last_resort()
                .leaf_node_capabilities(Capabilities::new(
                    None,
                    None,
                    Some(&[ExtensionType::LastResort]),
                    None,
                    None,
                ));
        }
        let bundle = builder
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
                guard_panics(|| {
                    KeyPackageIn::tls_deserialize_exact_bytes(bytes)
                        .map_err(other)?
                        .validate(self.provider.crypto(), ProtocolVersion::Mls10)
                        .map_err(other)
                })
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
                let leaf = leaf_of(&member.credential, &member.signature_key);
                (leaf.identity == identity).then_some(member.index)
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
    ///
    /// A commit is merged only when [`check_commit`] accepts it against
    /// `roster`, the server's current view of the conversation. Otherwise
    /// the device state is rolled back and [`Incoming::Rejected`] says why.
    /// A commit that removes this device deletes the local group, so that a
    /// later welcome can bring the device back.
    ///
    /// The sender of an application message is the credential of the leaf
    /// that sent it in the epoch it was sent. A message from a leaf that is
    /// no longer in the group, or whose slot another device now holds, is
    /// ignored: removal ends a device's ability to speak as well as read.
    pub fn process_incoming(
        &self,
        conversation_id: &str,
        message: &[u8],
        roster: &ServerRoster,
    ) -> Result<Incoming, MlsError> {
        let snapshot = self.snapshot();
        let result = self.process_incoming_inner(conversation_id, message, roster);
        if matches!(result, Ok(Incoming::Rejected(_)) | Err(_)) {
            self.restore(snapshot);
        }
        result
    }

    fn process_incoming_inner(
        &self,
        conversation_id: &str,
        message: &[u8],
        roster: &ServerRoster,
    ) -> Result<Incoming, MlsError> {
        let mut group = match self.group(conversation_id)? {
            Some(group) => group,
            None => return Ok(Incoming::Ignored),
        };
        let message = guard_panics(|| {
            MlsMessageIn::tls_deserialize_exact_bytes(message)
                .map_err(other)?
                .try_into_protocol_message()
                .map_err(other)
        })?;
        let processed = match group.process_message(&self.provider, message) {
            Ok(processed) => processed,
            Err(_) => return Ok(Incoming::Ignored),
        };
        let Sender::Member(sender_index) = *processed.sender() else {
            return Ok(Incoming::Ignored);
        };
        let credential = processed.credential().clone();
        let Some(current) = group.member_at(sender_index) else {
            return Ok(Incoming::Ignored);
        };
        let sender_leaf = leaf_of(&current.credential, &current.signature_key);
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(application) => {
                if current.credential != credential || !sender_leaf.valid_utf8 {
                    return Ok(Incoming::Ignored);
                }
                Ok(Incoming::Application {
                    sender: sender_leaf.identity,
                    payload: application.into_bytes(),
                })
            }
            ProcessedMessageContent::StagedCommitMessage(staged_commit) => {
                let changes = commit_changes(&group, sender_index, &staged_commit);
                if let Err(reason) = check_commit(roster, &changes) {
                    return Ok(Incoming::Rejected(reason));
                }
                let removes_self = staged_commit.self_removed();
                group
                    .merge_staged_commit(&self.provider, *staged_commit)
                    .map_err(other)?;
                if removes_self || !group.is_active() {
                    group.delete(self.provider.storage()).map_err(other)?;
                }
                Ok(Incoming::Commit)
            }
            _ => Ok(Incoming::Ignored),
        }
    }

    /// Process a serialized welcome that arrived in `conversation_id`: join
    /// the group. Returns the group's conversation id.
    ///
    /// The welcome is refused unless [`check_welcome`] accepts it: its group
    /// must be this conversation, and each of its leaves an active device of
    /// a member with its registered key. A device that already holds an
    /// active group for the conversation refuses it too. On any refusal or
    /// error the device state is rolled back, so the one-time key package
    /// the welcome was meant for stays usable.
    pub fn process_welcome(
        &self,
        conversation_id: &str,
        welcome_bytes: &[u8],
        roster: &ServerRoster,
    ) -> Result<String, MlsError> {
        let snapshot = self.snapshot();
        let result =
            guard_panics(|| self.process_welcome_inner(conversation_id, welcome_bytes, roster));
        if result.is_err() {
            self.restore(snapshot);
        }
        result
    }

    fn process_welcome_inner(
        &self,
        conversation_id: &str,
        welcome_bytes: &[u8],
        roster: &ServerRoster,
    ) -> Result<String, MlsError> {
        let welcome = Welcome::tls_deserialize_exact_bytes(welcome_bytes).map_err(other)?;
        let processed = ProcessedWelcome::new_from_welcome(&self.provider, &join_config(), welcome)
            .map_err(other)?;
        let group_id =
            String::from_utf8_lossy(processed.unverified_group_info().group_id().as_slice())
                .into_owned();
        if group_id != conversation_id {
            return Err(MlsError::Other(format!(
                "welcome names group {group_id:?} but arrived in conversation {conversation_id:?}"
            )));
        }
        if let Some(mut existing) = self.group(conversation_id)? {
            if existing.is_active() {
                return Err(MlsError::Other(
                    "this device is already in the conversation's group".into(),
                ));
            }
            // This device was removed earlier; drop the dead group so the
            // new welcome can bring it back.
            existing.delete(self.provider.storage()).map_err(other)?;
        }
        let staged = processed
            .into_staged_welcome(&self.provider, None)
            .map_err(other)?;
        let members: Vec<Leaf> = staged
            .members()
            .map(|member| leaf_of(&member.credential, &member.signature_key))
            .collect();
        check_welcome(roster, conversation_id, &group_id, &self.identity, &members)
            .map_err(MlsError::Other)?;
        staged.into_group(&self.provider).map_err(other)?;
        Ok(group_id)
    }

    /// The credential identities of all current group members. A leaf
    /// whose identity is not UTF-8 is listed with replacement characters
    /// rather than failing the whole call, so that it can still be removed.
    pub fn group_members(&self, conversation_id: &str) -> Result<Vec<String>, MlsError> {
        let group = self.group_mut(conversation_id)?;
        Ok(group
            .members()
            .map(|member| leaf_of(&member.credential, &member.signature_key).identity)
            .collect())
    }

    /// Every current leaf with its signature key.
    pub fn group_leaves(&self, conversation_id: &str) -> Result<Vec<Leaf>, MlsError> {
        let group = self.group_mut(conversation_id)?;
        Ok(group
            .members()
            .map(|member| leaf_of(&member.credential, &member.signature_key))
            .collect())
    }
}

/// Read what a staged commit would change, before merging it.
fn commit_changes(
    group: &MlsGroup,
    sender_index: LeafNodeIndex,
    staged: &StagedCommit,
) -> CommitChanges {
    let leaf_at = |index: LeafNodeIndex| {
        group
            .member_at(index)
            .map(|member| leaf_of(&member.credential, &member.signature_key))
    };
    let sender = leaf_at(sender_index);
    let added: Vec<Leaf> = staged
        .add_proposals()
        .map(|queued| {
            let leaf = queued.add_proposal().key_package().leaf_node();
            leaf_of(leaf.credential(), leaf.signature_key().as_slice())
        })
        .collect();
    let removed_indexes: Vec<LeafNodeIndex> = staged
        .remove_proposals()
        .map(|queued| queued.remove_proposal().removed())
        .collect();
    let removed: Vec<Leaf> = removed_indexes
        .iter()
        .filter_map(|index| leaf_at(*index))
        .collect();
    let mut replaced: Vec<(LeafNodeIndex, Leaf)> = Vec::new();
    let mut updated = Vec::new();
    for queued in staged.update_proposals() {
        if let Sender::Member(index) = queued.sender() {
            let leaf = queued.update_proposal().leaf_node();
            let after = leaf_of(leaf.credential(), leaf.signature_key().as_slice());
            if let Some(before) = leaf_at(*index) {
                updated.push((before, after.clone()));
            }
            replaced.push((*index, after));
        }
    }
    if let Some(path_leaf) = staged.update_path_leaf_node() {
        let after = leaf_of(path_leaf.credential(), path_leaf.signature_key().as_slice());
        if let Some(before) = sender.clone() {
            updated.push((before, after.clone()));
        }
        replaced.push((sender_index, after));
    }
    let mut members_after: Vec<Leaf> = group
        .members()
        .filter(|member| !removed_indexes.contains(&member.index))
        .map(|member| {
            replaced
                .iter()
                .rev()
                .find(|(index, _)| *index == member.index)
                .map(|(_, leaf)| leaf.clone())
                .unwrap_or_else(|| leaf_of(&member.credential, &member.signature_key))
        })
        .collect();
    members_after.extend(added.iter().cloned());
    CommitChanges {
        sender,
        added,
        removed,
        updated,
        members_after,
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
#[cfg(not(target_arch = "wasm32"))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), MlsError> {
    write_private_file(path, bytes).map_err(MlsError::Io)
}

/// Replace `path` with `bytes` so that a crash, power loss or full disk
/// leaves either the old file or the new one, never a truncated mix. The
/// file is readable by its owner only (mode 0600 on Unix) from the moment
/// it exists.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?
        .to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let temp = path.with_file_name(name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        if let Some(parent) = path.parent()
            && let Ok(dir) = std::fs::File::open(if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            })
        {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Read bytes from a state file.
#[cfg(not(target_arch = "wasm32"))]
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

    #[test]
    fn a_restored_device_knows_which_of_its_key_packages_it_can_open() {
        let bob = device("bob", 1);
        let early = bob.new_key_package().unwrap();
        let backup = bob.to_json().unwrap();
        let late = bob.new_key_package().unwrap();
        let last_resort = bob.new_last_resort_key_package().unwrap();
        assert!(bob.holds_key_package(&early).unwrap());
        assert!(bob.holds_key_package(&late).unwrap());
        assert!(bob.holds_key_package(&last_resort).unwrap());

        let restored = DeviceCrypto::from_json(&backup).unwrap();
        assert!(restored.holds_key_package(&early).unwrap());
        assert!(!restored.holds_key_package(&late).unwrap());
        assert!(!restored.holds_key_package(&last_resort).unwrap());

        let carol = device("carol", 1);
        assert!(
            !restored
                .holds_key_package(&carol.new_key_package().unwrap())
                .unwrap()
        );
        assert!(!restored.holds_key_package(b"not a key package").unwrap());
    }

    /// A server roster listing these devices with their keys bound.
    fn roster_of(devices: &[&DeviceCrypto]) -> ServerRoster {
        let mut roster = ServerRoster::new();
        for device in devices {
            roster.add_device(device.identity(), Some(device.signature_key()));
        }
        roster
    }

    /// alice creates the group and adds bob's device.
    fn setup_two(alice: &DeviceCrypto, bob: &DeviceCrypto) -> (Vec<u8>, Vec<u8>) {
        let conversation = "conv-1";
        alice.create_group(conversation).expect("create");
        let (commit, welcome) = alice
            .add_members(conversation, &[bob.new_key_package().expect("kp")])
            .expect("add");
        let joined = bob
            .process_welcome(
                conversation,
                &welcome.expect("welcome"),
                &roster_of(&[alice, bob]),
            )
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
        let roster = roster_of(&[&alice, &bob]);

        let wire = alice.encrypt(&conversation, &earlier).expect("encrypt");
        match bob
            .process_incoming(&conversation, &wire, &roster)
            .expect("process")
        {
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
        let roster = roster_of(&[&alice, &bob, &carol]);

        // Carol joins after the earlier message was encrypted.
        let (commit, welcome) = alice
            .add_members(&conversation, &[carol.new_key_package().expect("kp")])
            .expect("add carol");
        // Bob applies the commit so the group stays in sync.
        assert!(matches!(
            bob.process_incoming(&conversation, &commit, &roster)
                .expect("process"),
            Incoming::Commit
        ));
        let joined = carol
            .process_welcome(&conversation, &welcome.expect("welcome"), &roster)
            .expect("join");
        assert_eq!(joined, conversation);

        // Even with the earlier ciphertext in hand, carol cannot read it.
        assert!(matches!(
            carol.process_incoming(&conversation, &earlier, &roster),
            Ok(Incoming::Ignored) | Err(_)
        ));
    }

    #[test]
    fn removed_member_cannot_read_later_messages() {
        let alice = device("alice", 1);
        let bob = device("bob", 1);
        let (earlier, conversation) = setup_two(&alice, &bob);
        let conversation = String::from_utf8(conversation).unwrap();
        let mut roster = roster_of(&[&alice, &bob]);
        let earlier_wire = alice.encrypt(&conversation, &earlier).expect("encrypt");
        assert!(matches!(
            bob.process_incoming(&conversation, &earlier_wire, &roster)
                .expect("process"),
            Incoming::Application { .. }
        ));

        // The server removes bob, then alice removes his leaf; bob
        // processes the removal commit.
        roster = roster_of(&[&alice]);
        let commit = alice
            .remove_member(&conversation, "bob#device-1")
            .expect("remove");
        assert!(matches!(
            bob.process_incoming(&conversation, &commit, &roster)
                .expect("process"),
            Incoming::Commit
        ));

        // A later message cannot be decrypted by bob, even with the bytes.
        let later = alice
            .encrypt(&conversation, b"after removal")
            .expect("encrypt");
        assert!(matches!(
            bob.process_incoming(&conversation, &later, &roster),
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
        let roster = roster_of(&[&alice, &bob2]);
        assert_eq!(bob2.identity(), bob.identity());

        alice.create_group("conv-1").expect("create");
        let (_commit, welcome) = alice.add_members("conv-1", &[package]).expect("add");
        let joined = bob2
            .process_welcome("conv-1", &welcome.expect("welcome"), &roster)
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

    /// A last-resort package can join more than one group. That is what
    /// lets a device be added after its one-time packages are gone.
    #[test]
    fn last_resort_key_package_joins_a_second_group() {
        let alice = device("alice", 1);
        let bob = device("bob", 1);
        let package = bob.new_last_resort_key_package().expect("last resort");

        alice.create_group("conv-1").expect("create");
        let (_commit, welcome) = alice
            .add_members("conv-1", std::slice::from_ref(&package))
            .expect("first add");
        assert_eq!(
            bob.process_welcome(
                "conv-1",
                &welcome.expect("welcome"),
                &roster_of(&[&alice, &bob])
            )
            .expect("join"),
            "conv-1"
        );

        alice.create_group("conv-2").expect("create");
        let (_commit, welcome) = alice.add_members("conv-2", &[package]).expect("second add");
        assert_eq!(
            bob.process_welcome(
                "conv-2",
                &welcome.expect("welcome"),
                &roster_of(&[&alice, &bob])
            )
            .expect("rejoin"),
            "conv-2"
        );
    }
}
