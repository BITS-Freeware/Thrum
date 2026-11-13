#![no_main]

use thrum_core::envelope::AppMessage;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<AppMessage>(data);
});
