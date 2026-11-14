#![no_main]

use libfuzzer_sys::fuzz_target;
use thrum_core::envelope::AppMessage;

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<AppMessage>(data);
});
