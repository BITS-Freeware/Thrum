#![no_main]

use libfuzzer_sys::fuzz_target;
use std::path::Path;
use thrum_cli::files::{mime_from_name, sanitize_name, save_attachment};

fuzz_target!(|data: &[u8]| {
    let name = String::from_utf8_lossy(data);
    let clean = sanitize_name(&name);
    assert!(!clean.is_empty(), "empty for {name:?}");
    assert!(clean.chars().count() <= 255, "unbounded for {name:?}");
    assert!(!clean.contains('/'), "separator survived {name:?}");
    assert_eq!(
        Path::new(&clean).components().count(),
        1,
        "{clean:?} is not a single component (from {name:?})"
    );
    if let Ok(dir) = tempfile::tempdir() {
        if let Ok(path) = save_attachment(dir.path(), &name, b"fuzz") {
            assert!(
                path.starts_with(dir.path()),
                "{path:?} escaped the directory for name {name:?}"
            );
        }
    }
    let _ = mime_from_name(&name);
});
