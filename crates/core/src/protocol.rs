//! Wire protocol frames shared by the server and every client.
//!
//! Frames are JSON objects tagged by their `type` field. Message payloads
//! travel as base64 strings so the frames stay text; the bytes themselves
//! are opaque to the server and will be MLS ciphertext from M3 on.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// A frame a client sends to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    /// The first frame on a WebSocket: authenticate a device session.
    Auth { token: String },
    /// Confirm receipt of a message so the server can stop queueing it.
    Ack { message_id: String },
}

/// A frame the server sends to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    /// Reply to a successful [`ClientFrame::Auth`].
    AuthOk { device_id: String },
    /// A message for one of the caller's conversations.
    Message {
        conversation_id: String,
        message_id: String,
        sender_username: String,
        payload_base64: String,
        created_at: i64,
    },
    /// Reply to an [`ClientFrame::Ack`], sent after the server recorded it.
    AckOk { message_id: String },
    /// Every message that was queued when the connection authenticated has
    /// been sent. Later frames are live. A client that needs a current
    /// group state before acting waits for this and for its frames to be
    /// applied.
    CaughtUp,
}

/// Encode an opaque payload as standard base64.
pub fn encode_payload(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// Decode a base64 payload; `None` when malformed.
pub fn decode_payload(encoded: &str) -> Option<Vec<u8>> {
    STANDARD.decode(encoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        let bytes = b"opaque bytes \xf0\x9f\x91\xbb";
        assert_eq!(
            decode_payload(&encode_payload(bytes)).as_deref(),
            Some(&bytes[..])
        );
    }

    #[test]
    fn the_end_of_the_queue_is_a_bare_tag() {
        let json = serde_json::to_string(&ServerFrame::CaughtUp).expect("serialize");
        assert_eq!(json, r#"{"type":"caught_up"}"#);
        assert_eq!(
            serde_json::from_str::<ServerFrame>(&json).expect("parse"),
            ServerFrame::CaughtUp
        );
    }

    #[test]
    fn frames_roundtrip_through_json() {
        let frame = ServerFrame::Message {
            conversation_id: "c1".to_string(),
            message_id: "m1".to_string(),
            sender_username: "alice".to_string(),
            payload_base64: encode_payload(b"hi"),
            created_at: 123,
        };
        let json = serde_json::to_string(&frame).expect("serialize");
        assert_eq!(
            serde_json::from_str::<ServerFrame>(&json).expect("parse"),
            frame
        );
        assert!(json.contains(r#""type":"message""#));
    }

    #[test]
    fn decode_rejects_malformed_base64() {
        assert!(decode_payload("!!!").is_none());
    }
}
