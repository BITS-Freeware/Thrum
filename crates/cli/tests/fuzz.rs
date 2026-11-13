//! Fuzz tests for the CLI's attachment-name handling.
//!
//! Attachment names arrive from other users inside encrypted messages, so
//! they are attacker-controlled before they touch the local filesystem.
//! The properties under test: `sanitize_name` always yields a single,
//! bounded path component, and `save_attachment` never writes outside the
//! requested directory, whatever name arrives. Deterministic seeded inputs
//! mirror the stable fuzz layer in docs/decisions/0011.

use std::path::Path;

use geist3_cli::files::{mime_from_name, sanitize_name, save_attachment};

/// A tiny deterministic PRNG (xorshift64*) so fuzz runs are reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound.max(1) as u64) as usize
    }

    fn mutate_text(&mut self, text: &str) -> String {
        let mut out = text.as_bytes().to_vec();
        for _ in 0..1 + self.below(4) {
            if out.is_empty() {
                break;
            }
            match self.below(5) {
                0 => out.truncate(self.below(out.len())),
                1 => {
                    let at = self.below(out.len());
                    out[at] = (self.next_u64() >> 24) as u8;
                }
                2 => {
                    let at = self.below(out.len() + 1);
                    out.insert(at, (self.next_u64() >> 24) as u8);
                }
                3 => {
                    let n = self.below(8);
                    out.extend((0..n).map(|_| (self.next_u64() >> 24) as u8));
                }
                _ => {
                    let at = self.below(out.len());
                    out[at] = b'.';
                }
            }
            if out.len() > 600 {
                out.truncate(600);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// A random string of unicode, including separators, dots and
    /// control characters a hostile sender might try.
    fn unicode(&mut self) -> String {
        const RANGES: [(u32, u32); 5] = [
            (0x20, 0x7e),       // printable ASCII, includes / and .
            (0xa0, 0x2ff),      // latin through IPA
            (0x2000, 0x206f),   // punctuation and separators
            (0x2028, 0x2029),   // line and paragraph separator
            (0x1f300, 0x1f6ff), // emoji
        ];
        let mut out = String::new();
        for _ in 0..1 + self.below(16) {
            let (low, high) = RANGES[self.below(RANGES.len())];
            let span = high - low;
            let point = low + (self.next_u64() % u64::from(span)) as u32;
            if let Some(character) = char::from_u32(point) {
                out.push(character);
            }
        }
        out
    }

    fn name(&mut self) -> String {
        const POOL: [&str; 12] = [
            "photo.png",
            "../etc/passwd",
            "a/b/c.txt",
            "..",
            ".",
            " ",
            "~",
            "nul\u{0}byte.png",
            "emoji \u{1f980}.png",
            "report.v2.final.pdf",
            ".hidden",
            "UPPER.JPG",
        ];
        if self.below(4) == 0 {
            self.unicode()
        } else {
            let base = POOL[self.below(POOL.len())];
            self.mutate_text(base)
        }
    }
}

const ITERS: usize = 2048;

#[test]
fn fuzz_sanitize_and_save_stay_inside_the_directory() {
    let mut rng = Rng::new(0xCA_FE_00_01);
    let dir = tempfile::tempdir().expect("temp dir");
    for _ in 0..ITERS {
        let name = rng.name();
        let clean = sanitize_name(&name);
        assert!(!clean.is_empty(), "empty for {name:?}");
        assert!(clean.chars().count() <= 255, "unbounded for {name:?}");
        assert!(!clean.contains('/'), "separator survived {name:?}");
        assert_ne!(clean, "..", "parent survived {name:?}");
        // A single path component: joining it cannot leave the directory.
        assert_eq!(
            Path::new(&clean).components().count(),
            1,
            "{clean:?} is not a single component (from {name:?})"
        );
        // Platform write failures (for example a NUL byte) are fine as
        // long as they surface as an error, not a crash.
        if let Ok(path) = save_attachment(dir.path(), &name, b"fuzz") {
            assert!(
                path.starts_with(dir.path()),
                "{path:?} escaped the directory for name {name:?}"
            );
            let _ = std::fs::remove_file(&path);
        }
        let _ = mime_from_name(&name);
    }
}
