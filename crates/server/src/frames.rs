//! Wire frames for the message relay.

use geist3_core::protocol::{ClientFrame, ServerFrame};

/// Re-exported so handlers and tests share one import path.
pub use geist3_core::protocol::{decode_payload, encode_payload};

/// A frame waiting to go out to one device.
pub type OutboundFrame = ServerFrame;

/// Parse a client text frame; `None` when it is not a valid frame.
pub fn parse_client_frame(text: &str) -> Option<ClientFrame> {
    serde_json::from_str(text).ok()
}

/// Serialize a server frame to JSON text.
pub fn render_server_frame(frame: &ServerFrame) -> String {
    serde_json::to_string(frame).expect("server frames serialize")
}
