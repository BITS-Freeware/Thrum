#![no_main]

use thrum_core::envelope::{decode_envelope, Envelope};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(envelope) = decode_envelope(data) {
        let _ = envelope.data();
        // Re-encode must succeed for anything that decoded.
        let _ = serde_json::to_vec(&envelope);
    }
    let _ = Envelope::Mls {
        data: String::from_utf8_lossy(data).into_owned(),
    }
    .data();
});
