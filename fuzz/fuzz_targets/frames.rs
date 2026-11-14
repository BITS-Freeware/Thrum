#![no_main]

use libfuzzer_sys::fuzz_target;
use thrum_core::protocol::{ClientFrame, ServerFrame, decode_payload};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(frame) = serde_json::from_str::<ClientFrame>(&text).ok() {
        let _ = serde_json::to_string(&frame);
    }
    if let Some(ServerFrame::Message { payload_base64, .. }) =
        serde_json::from_str::<ServerFrame>(&text).ok()
    {
        let _ = decode_payload(&payload_base64);
    }
    let _ = decode_payload(&text);
});
