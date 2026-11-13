//! Fuzz test for the server's client-frame parser.
//!
//! The server accepts text frames from any connected client and must
//! survive arbitrary input: `parse_client_frame` returns `None`, never
//! panics. Deterministic seeded inputs mirror the stable fuzz layer in
//! docs/decisions/0011; deep coverage-guided fuzzing lives under `fuzz/`.

use geist3_server::frames::parse_client_frame;

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

    fn byte(&mut self) -> u8 {
        (self.next_u64() >> 24) as u8
    }

    /// Mutate a valid frame: truncate, flip, splice, duplicate, extend, or
    /// substitute from a pool of bytes that tend to break JSON parsers.
    fn mutate_text(&mut self, text: &str) -> String {
        let mut out = text.as_bytes().to_vec();
        const INTERESTING: [u8; 8] = [0x00, 0x22, 0x5c, 0x7b, 0x7d, 0x5b, 0x5d, 0xff];
        for _ in 0..1 + (self.next_u64() % 4) as usize {
            if out.is_empty() {
                break;
            }
            match self.next_u64() % 6 {
                0 => out.truncate((self.next_u64() as usize) % out.len()),
                1 => {
                    let at = (self.next_u64() as usize) % out.len();
                    out[at] ^= self.byte().max(1);
                }
                2 => {
                    let at = (self.next_u64() as usize) % (out.len() + 1);
                    out.insert(at, self.byte());
                }
                3 => {
                    let at = (self.next_u64() as usize) % out.len();
                    out.insert(
                        at,
                        INTERESTING[(self.next_u64() as usize) % INTERESTING.len()],
                    );
                }
                4 => {
                    let n = (self.next_u64() as usize) % 16;
                    out.extend((0..n).map(|_| self.byte()));
                }
                _ => {
                    let at = (self.next_u64() as usize) % out.len();
                    out[at] = INTERESTING[(self.next_u64() as usize) % INTERESTING.len()];
                }
            }
            if out.len() > 4096 {
                out.truncate(4096);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

const ITERS: usize = 4096;

#[test]
fn fuzz_parse_client_frame_never_panics() {
    let mut rng = Rng::new(0x5E_1F_00_01);
    let auth = r#"{"type":"auth","token":"tok"}"#;
    let ack = r#"{"type":"ack","message_id":"m1"}"#;
    for _ in 0..ITERS {
        let _ = parse_client_frame(&rng.mutate_text(auth));
        let _ = parse_client_frame(&rng.mutate_text(ack));
        // Also feed frames with unknown types and shapes outright.
        let _ = parse_client_frame(&rng.mutate_text(r#"{"type":7}"#));
    }
}
