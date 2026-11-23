//! WebAssembly bindings for the Thrum core.
//!
//! The web client gets exactly the same cryptographic operations as the CLI:
//! MLS group handling through [`DeviceCrypto`], the per-file attachment
//! crypto, and the application-message codec. Key material never leaves the
//! page; the caller stores [`DeviceState`] JSON in IndexedDB.

use js_sys::Uint8Array;
use serde::Serialize;
use serde_wasm_bindgen::to_value;
use thrum_core as core;
use thrum_core::attachment::{FileKey, decrypt_file, encrypt_file};
use thrum_core::envelope::AppMessage;
use thrum_core::membership::ServerRoster;
use thrum_core::mls::{DeviceCrypto, Incoming};
use thrum_core::protocol::{decode_payload, encode_payload};
use wasm_bindgen::prelude::*;

/// Turn a JS byte array into a plain `Vec<u8>`.
fn bytes(value: Uint8Array) -> Vec<u8> {
    value.to_vec()
}

/// Turn bytes into a fresh JS byte array.
fn array(bytes: &[u8]) -> Uint8Array {
    let array = Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    array
}

/// Build a JS object `{ [key]: value, ... }` from string pairs.
fn object(entries: &[(&str, JsValue)]) -> Result<JsValue, JsValue> {
    let object = js_sys::Object::new();
    for (key, value) in entries {
        js_sys::Reflect::set(&object, &(*key).into(), value)?;
    }
    Ok(object.into())
}

/// Parse the server's view of a conversation from JSON:
/// `{ "<username>": { "<device id>": "<base64 signature key>" | null } }`,
/// one entry per member account with its active devices.
fn roster(json: &str) -> Result<ServerRoster, JsValue> {
    use base64::Engine as _;
    use std::collections::BTreeMap;
    let accounts: BTreeMap<String, BTreeMap<String, Option<String>>> =
        serde_json::from_str(json).map_err(|error| JsValue::from_str(&error.to_string()))?;
    let mut roster = ServerRoster::new();
    for (username, devices) in accounts {
        let mut keys = Vec::new();
        for (device_id, key) in devices {
            let key = key
                .map(|key| base64::engine::general_purpose::STANDARD.decode(key))
                .transpose()
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            keys.push((device_id, key));
        }
        roster.add_account(&username, keys);
    }
    Ok(roster)
}

/// A device's MLS state. Create with [`new_device`] or
/// [`DeviceState::from_json`]; persist with [`DeviceState::to_json`].
#[wasm_bindgen]
pub struct DeviceState {
    inner: DeviceCrypto,
}

#[wasm_bindgen]
impl DeviceState {
    /// Serialize the device's private key material to JSON for IndexedDB.
    #[wasm_bindgen(js_name = toJSON)]
    pub fn to_json(&self) -> Result<String, JsValue> {
        self.inner.to_json().map_err(js_error)
    }

    /// The device identity (`username#device-id`).
    #[wasm_bindgen(getter, js_name = identity)]
    pub fn identity(&self) -> String {
        self.inner.identity().to_string()
    }

    /// The username part of the device identity.
    #[wasm_bindgen(getter, js_name = username)]
    pub fn username(&self) -> String {
        self.inner.username().to_string()
    }

    /// A fresh serialized one-time key package for this device.
    pub fn new_key_package(&self) -> Result<Uint8Array, JsValue> {
        self.inner
            .new_key_package()
            .map(|bytes| array(&bytes))
            .map_err(js_error)
    }

    /// A last-resort key package. The server keeps it and serves it again
    /// after the one-time stock is empty.
    pub fn new_last_resort_key_package(&self) -> Result<Uint8Array, JsValue> {
        self.inner
            .new_last_resort_key_package()
            .map(|bytes| array(&bytes))
            .map_err(js_error)
    }

    /// Whether this device holds the private keys for one of its own key
    /// packages. A device restored from older state does not hold those of
    /// packages it uploaded later.
    pub fn holds_key_package(&self, package: Uint8Array) -> Result<bool, JsValue> {
        self.inner
            .holds_key_package(&bytes(package))
            .map_err(js_error)
    }

    /// Validate a fetched key package and return the identity it claims.
    pub fn key_package_identity(&self, package: Uint8Array) -> Result<String, JsValue> {
        self.inner
            .key_package_identity(&bytes(package))
            .map_err(js_error)
    }

    /// Check a key package fetched for `identity` before adding it: it must
    /// claim that identity and carry the signature key the server bound to
    /// the device. `roster` is the JSON described at [`roster`]. Throws with
    /// the reason when the package must not be added.
    pub fn check_key_package(
        &self,
        package: Uint8Array,
        identity: &str,
        roster_json: &str,
    ) -> Result<(), JsValue> {
        self.inner
            .check_key_package(&bytes(package), identity, &roster(roster_json)?)
            .map_err(|reason| JsValue::from_str(&reason))
    }

    /// This device's public signature key, as the server binds it.
    pub fn signature_key(&self) -> Uint8Array {
        array(&self.inner.signature_key())
    }

    /// True when this device has joined the conversation's MLS group.
    pub fn has_group(&self, conversation_id: &str) -> Result<bool, JsValue> {
        self.inner.has_group(conversation_id).map_err(js_error)
    }

    /// Create the MLS group for a conversation. Idempotent.
    pub fn create_group(&self, conversation_id: &str) -> Result<(), JsValue> {
        self.inner.create_group(conversation_id).map_err(js_error)
    }

    /// Add members from serialized key packages. Returns
    /// `{ commit: Uint8Array, welcome: Uint8Array | null }`.
    pub fn add_members(
        &self,
        conversation_id: &str,
        key_packages: Vec<Uint8Array>,
    ) -> Result<JsValue, JsValue> {
        let packages: Vec<Vec<u8>> = key_packages.iter().map(|p| bytes(p.clone())).collect();
        let (commit, welcome) = self
            .inner
            .add_members(conversation_id, &packages)
            .map_err(js_error)?;
        let welcome_js = match welcome {
            Some(welcome) => array(&welcome).into(),
            None => JsValue::NULL,
        };
        object(&[("commit", array(&commit).into()), ("welcome", welcome_js)])
    }

    /// Remove a member by identity; returns the serialized commit.
    pub fn remove_member(
        &self,
        conversation_id: &str,
        identity: &str,
    ) -> Result<Uint8Array, JsValue> {
        self.inner
            .remove_member(conversation_id, identity)
            .map(|bytes| array(&bytes))
            .map_err(js_error)
    }

    /// Encrypt an application payload for the conversation's group.
    pub fn encrypt(
        &self,
        conversation_id: &str,
        payload: Uint8Array,
    ) -> Result<Uint8Array, JsValue> {
        self.inner
            .encrypt(conversation_id, &bytes(payload))
            .map(|bytes| array(&bytes))
            .map_err(js_error)
    }

    /// Process one serialized MLS message, checking commits against the
    /// server's current roster (JSON as for [`roster`]). Returns
    /// `{ kind: "application", sender, payload }`, `{ kind: "commit" }`,
    /// `{ kind: "ignored" }` or `{ kind: "rejected", reason }`. A rejected
    /// commit leaves the device unchanged; retry with a fresher roster.
    pub fn process_incoming(
        &self,
        conversation_id: &str,
        message: Uint8Array,
        roster_json: &str,
    ) -> Result<JsValue, JsValue> {
        let incoming = self
            .inner
            .process_incoming(conversation_id, &bytes(message), &roster(roster_json)?)
            .map_err(js_error)?;
        match incoming {
            Incoming::Application { sender, payload } => object(&[
                ("kind", "application".into()),
                ("sender", sender.into()),
                ("payload", array(&payload).into()),
            ]),
            Incoming::Commit => object(&[("kind", "commit".into())]),
            Incoming::Ignored => object(&[("kind", "ignored".into())]),
            Incoming::Rejected(reason) => {
                object(&[("kind", "rejected".into()), ("reason", reason.into())])
            }
        }
    }

    /// Process a serialized welcome that arrived in `conversation_id`: join
    /// the group if the welcome is for that conversation and every other
    /// member is on the server's roster with its bound key. Returns the
    /// conversation id; throws otherwise, leaving the device unchanged.
    pub fn process_welcome(
        &self,
        conversation_id: &str,
        welcome: Uint8Array,
        roster_json: &str,
    ) -> Result<String, JsValue> {
        self.inner
            .process_welcome(conversation_id, &bytes(welcome), &roster(roster_json)?)
            .map_err(js_error)
    }

    /// The current group leaves as `[{ identity, signatureKey }]`, the key
    /// base64 encoded, for pinning. Leaves whose identity is not
    /// `username#device-id` are left out.
    pub fn group_leaves(&self, conversation_id: &str) -> Result<js_sys::Array, JsValue> {
        use base64::Engine as _;
        let leaves = self.inner.group_leaves(conversation_id).map_err(js_error)?;
        let array = js_sys::Array::new();
        for leaf in leaves.into_iter().filter(|leaf| leaf.well_formed()) {
            let key = base64::engine::general_purpose::STANDARD.encode(&leaf.signature_key);
            array.push(&object(&[
                ("identity", leaf.identity.into()),
                ("signatureKey", key.into()),
            ])?);
        }
        Ok(array)
    }

    /// The credential identities of all current group members.
    pub fn group_members(&self, conversation_id: &str) -> Result<js_sys::Array, JsValue> {
        let members = self
            .inner
            .group_members(conversation_id)
            .map_err(js_error)?;
        let array = js_sys::Array::new();
        for member in members {
            array.push(&member.into());
        }
        Ok(array)
    }
}

/// Output of `add_members` is built inline; no struct needed.
/// Identities in the group whose account is no longer a server member.
/// Returns a JS array of identity strings. The caller's own identity is omitted.
#[wasm_bindgen]
pub fn departed_members(
    self_identity: &str,
    group_members: Vec<String>,
    server_members: Vec<String>,
) -> js_sys::Array {
    let array = js_sys::Array::new();
    for identity in core::roster::departed_members(self_identity, &group_members, &server_members) {
        array.push(&identity.into());
    }
    array
}

/// Plan adds, pending approvals, and removals for one account.
///
/// Returns an array of `{ kind: "add" | "pending" | "remove", identity }`.
#[wasm_bindgen]
pub fn plan_roster(
    self_identity: &str,
    group_members: Vec<String>,
    username: &str,
    active_device_ids: Vec<String>,
    trusted: Vec<String>,
    admit_untrusted: bool,
    known_account: bool,
) -> js_sys::Array {
    let trusted: std::collections::BTreeSet<String> = trusted.into_iter().collect();
    let array = js_sys::Array::new();
    for action in core::roster::plan_roster(
        self_identity,
        &group_members,
        username,
        &active_device_ids,
        &trusted,
        admit_untrusted,
        known_account,
    ) {
        let (kind, identity) = match action {
            core::roster::RosterAction::Add(identity) => ("add", identity),
            core::roster::RosterAction::Pending(identity) => ("pending", identity),
            core::roster::RosterAction::Remove(identity) => ("remove", identity),
        };
        let entry = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&entry, &"kind".into(), &kind.into());
        let _ = js_sys::Reflect::set(&entry, &"identity".into(), &identity.into());
        array.push(&entry);
    }
    array
}

/// Generate fresh key material for a new device identity
/// (`username#device-id`).
#[wasm_bindgen]
pub fn new_device(identity: String) -> DeviceState {
    DeviceState {
        inner: DeviceCrypto::generate(identity),
    }
}

/// Reconstruct a device from [`DeviceState::toJSON`] output.
#[wasm_bindgen(js_name = deviceFromJson)]
pub fn device_from_json(json: &str) -> Result<DeviceState, JsValue> {
    DeviceCrypto::from_json(json)
        .map(|inner| DeviceState { inner })
        .map_err(js_error)
}

/// Generate a fresh per-file key and nonce as
/// `{ key_base64, nonce_base64 }`, ready to travel inside an
/// `AppMessage::Attachment`.
#[wasm_bindgen(js_name = newFileKey)]
pub fn new_file_key() -> Result<JsValue, JsValue> {
    use base64::Engine as _;
    let key = FileKey::generate();
    let engine = base64::engine::general_purpose::STANDARD;
    #[derive(Serialize)]
    struct KeyJs {
        key_base64: String,
        nonce_base64: String,
    }
    to_value(&KeyJs {
        key_base64: engine.encode(key.key),
        nonce_base64: engine.encode(key.nonce),
    })
    .map_err(js_error)
}

/// Encrypt a file for upload; returns the ciphertext with the GCM tag.
#[wasm_bindgen(js_name = encryptFile)]
pub fn encrypt_file_js(
    key_base64: &str,
    nonce_base64: &str,
    blob_id: &str,
    plaintext: Uint8Array,
) -> Result<Uint8Array, JsValue> {
    let key = parse_file_key(key_base64, nonce_base64)?;
    Ok(array(&encrypt_file(&key, blob_id, &bytes(plaintext))))
}

/// Decrypt a fetched blob; throws when the key, id or bytes do not match.
#[wasm_bindgen(js_name = decryptFile)]
pub fn decrypt_file_js(
    key_base64: &str,
    nonce_base64: &str,
    blob_id: &str,
    ciphertext: Uint8Array,
) -> Result<Uint8Array, JsValue> {
    let key = parse_file_key(key_base64, nonce_base64)?;
    decrypt_file(&key, blob_id, &bytes(ciphertext))
        .map(|plaintext| array(&plaintext))
        .ok_or_else(|| JsValue::from_str("attachment decryption failed"))
}

/// Parse base64 key and nonce strings into a [`FileKey`].
fn parse_file_key(key_base64: &str, nonce_base64: &str) -> Result<FileKey, JsValue> {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::STANDARD;
    let decode = |text: &str, expected: usize| -> Result<Vec<u8>, JsValue> {
        let decoded = engine
            .decode(text)
            .map_err(|_| JsValue::from_str("bad attachment key encoding"))?;
        if decoded.len() != expected {
            return Err(JsValue::from_str("bad attachment key length"));
        }
        Ok(decoded)
    };
    let key_bytes = decode(key_base64, core::attachment::KEY_LEN)?;
    let nonce_bytes = decode(nonce_base64, core::attachment::NONCE_LEN)?;
    let mut key = FileKey {
        key: [0u8; core::attachment::KEY_LEN],
        nonce: [0u8; core::attachment::NONCE_LEN],
    };
    key.key.copy_from_slice(&key_bytes);
    key.nonce.copy_from_slice(&nonce_bytes);
    Ok(key)
}

/// Encode an application message object (same JSON shape as the Rust enum)
/// into the payload bytes that travel inside an MLS message.
#[wasm_bindgen(js_name = encodeAppMessage)]
pub fn encode_app_message(message: JsValue) -> Result<Uint8Array, JsValue> {
    let message: AppMessage = serde_wasm_bindgen::from_value(message).map_err(js_error)?;
    Ok(array(&serde_json::to_vec(&message).map_err(js_error)?))
}

/// Decode payload bytes into an application message object.
#[wasm_bindgen(js_name = decodeAppMessage)]
pub fn decode_app_message(payload: Uint8Array) -> Result<JsValue, JsValue> {
    let message: AppMessage = serde_json::from_slice(&bytes(payload)).map_err(js_error)?;
    to_value(&message).map_err(js_error)
}

/// Encode a client frame object into the JSON string sent over WebSocket.
#[wasm_bindgen(js_name = encodeClientFrame)]
pub fn encode_client_frame(frame: JsValue) -> Result<String, JsValue> {
    let frame: core::protocol::ClientFrame =
        serde_wasm_bindgen::from_value(frame).map_err(js_error)?;
    serde_json::to_string(&frame).map_err(js_error)
}

/// Parse a WebSocket text frame into a server frame object.
#[wasm_bindgen(js_name = decodeServerFrame)]
pub fn decode_server_frame(text: &str) -> Result<JsValue, JsValue> {
    let frame: core::protocol::ServerFrame = serde_json::from_str(text).map_err(js_error)?;
    to_value(&frame).map_err(js_error)
}

/// Wrap serialized MLS bytes as the relay payload (JSON envelope string).
#[wasm_bindgen(js_name = encodeEnvelope)]
pub fn encode_envelope(kind: &str, data: Uint8Array) -> Result<Uint8Array, JsValue> {
    use core::envelope::{Envelope, encode_envelope as encode};
    let envelope = match kind {
        "mls" => Envelope::mls(&bytes(data)),
        "welcome" => Envelope::welcome(&bytes(data)),
        other => {
            return Err(JsValue::from_str(&format!(
                "unknown envelope kind {other:?}"
            )));
        }
    };
    Ok(array(&encode(&envelope)))
}

/// Parse a relay payload into `{ kind, data: Uint8Array }`, or null.
#[wasm_bindgen(js_name = decodeEnvelope)]
pub fn decode_envelope(payload: Uint8Array) -> Result<JsValue, JsValue> {
    use core::envelope::{Envelope, decode_envelope as decode};
    let decoded = match decode(&bytes(payload)) {
        Some(decoded) => decoded,
        None => return Ok(JsValue::NULL),
    };
    let (kind, data_base64) = match &decoded {
        Envelope::Mls { data } => ("mls", data),
        Envelope::Welcome { data } => ("welcome", data),
    };
    // Bad base64 inside a valid envelope is as undecodable as a bad
    // envelope, not an empty message.
    let Some(data) = decode_payload(data_base64) else {
        return Ok(JsValue::NULL);
    };
    object(&[("kind", (*kind).into()), ("data", array(&data).into())])
}

/// Encode bytes as the base64 the relay protocol expects.
#[wasm_bindgen(js_name = encodePayload)]
pub fn encode_payload_js(data: Uint8Array) -> String {
    encode_payload(&bytes(data))
}

/// Decode relay base64; throws on malformed input.
#[wasm_bindgen(js_name = decodePayload)]
pub fn decode_payload_js(encoded: &str) -> Result<Uint8Array, JsValue> {
    decode_payload(encoded)
        .map(|bytes| array(&bytes))
        .ok_or_else(|| JsValue::from_str("bad base64 payload"))
}

/// Validate and canonicalize an account name; returns the lowercase form.
#[wasm_bindgen(js_name = validateUserName)]
pub fn validate_user_name(raw: &str) -> Result<String, JsValue> {
    core::UserName::new(raw)
        .map(|name| name.as_str().to_string())
        .map_err(|_| JsValue::from_str("invalid user name"))
}

fn js_error<E: std::fmt::Display>(error: E) -> JsValue {
    JsValue::from_str(&error.to_string())
}
