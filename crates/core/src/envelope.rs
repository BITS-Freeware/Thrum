//! The client-side envelope around MLS material sent through the relay.
//!
//! The server stores and forwards payloads as opaque bytes. Every payload a
//! client sends is a small JSON envelope: a MLS message (commit or
//! application message) or a Welcome that lets a new device join a group.
//! Everything inside is MLS ciphertext except the envelope kind.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// A payload as it travels through the relay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Envelope {
    /// A TLS-serialized MLS `Welcome`; lets one device join a group.
    Welcome { data: String },
    /// A TLS-serialized MLS message: commit or application message.
    Mls { data: String },
}

impl Envelope {
    /// Wrap serialized MLS bytes as an `mls` envelope.
    pub fn mls(bytes: &[u8]) -> Self {
        Envelope::Mls {
            data: STANDARD.encode(bytes),
        }
    }

    /// Wrap a serialized `Welcome`.
    pub fn welcome(bytes: &[u8]) -> Self {
        Envelope::Welcome {
            data: STANDARD.encode(bytes),
        }
    }

    /// The wrapped bytes, or `None` when the envelope is malformed.
    pub fn data(&self) -> Option<Vec<u8>> {
        match self {
            Envelope::Mls { data } | Envelope::Welcome { data } => STANDARD.decode(data).ok(),
        }
    }
}

/// Serialize an envelope to the bytes that travel through the relay.
pub fn encode_envelope(envelope: &Envelope) -> Vec<u8> {
    serde_json::to_vec(envelope).expect("envelope serializes")
}

/// Parse relay payload bytes into an envelope; `None` when malformed.
pub fn decode_envelope(payload: &[u8]) -> Option<Envelope> {
    serde_json::from_slice(payload).ok()
}

/// What clients exchange inside MLS application messages. Everything here is
/// encrypted like an ordinary message; the server only ever sees ciphertext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppMessage {
    /// A plain chat text.
    Text { text: String },
    /// A chat text that quotes an earlier message by its relay id.
    Reply { reply_to: String, text: String },
    /// Replace the text of an earlier message the sender wrote.
    Edit { message_id: String, text: String },
    /// Retract an earlier message the sender wrote.
    Delete { message_id: String },
    /// Tell the group these messages have been read (by their relay ids).
    Read { message_ids: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let envelope = Envelope::mls(b"ciphertext bytes");
        let bytes = encode_envelope(&envelope);
        let decoded = decode_envelope(&bytes).expect("parses");
        assert_eq!(decoded, envelope);
        assert_eq!(decoded.data().expect("data"), b"ciphertext bytes");
    }

    #[test]
    fn envelope_kinds_are_distinct_and_opaque() {
        let welcome = encode_envelope(&Envelope::welcome(b"w"));
        let mls = encode_envelope(&Envelope::mls(b"m"));
        let welcome = String::from_utf8(welcome).expect("utf8");
        assert!(welcome.contains(r#""kind":"welcome""#));
        let mls = String::from_utf8(mls).expect("utf8");
        assert!(mls.contains(r#""kind":"mls""#));
        assert!(decode_envelope(b"not json").is_none());
    }

    #[test]
    fn app_message_roundtrip() {
        let message = AppMessage::Text {
            text: "hello".to_string(),
        };
        let bytes = serde_json::to_vec(&message).expect("serializes");
        assert_eq!(
            serde_json::from_slice::<AppMessage>(&bytes).expect("parses"),
            message
        );
        let bytes = String::from_utf8(bytes).expect("utf8");
        assert!(bytes.contains(r#""type":"text""#));
    }
}
