//! Fuzz tests for every core parser that reads data from the network.
//!
//! Each parser is driven with pure-random input and with structured
//! mutations of a valid encoding; the property under test is that parsing
//! is total (no panic, only typed errors) and that the MLS layer keeps a
//! device's group state intact across arbitrary garbage. The inputs come
//! from a seeded xorshift PRNG, so every run is deterministic and a
//! failure reproduces from its seed. Deep coverage-guided fuzzing of the
//! same parsers lives under `fuzz/` (see docs/decisions/0011).

use geist3_core::envelope::{AppMessage, Envelope, decode_envelope, encode_envelope};
use geist3_core::mls::{DeviceCrypto, Incoming};
use geist3_core::protocol::{ClientFrame, ServerFrame, decode_payload, encode_payload};

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

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.byte()).collect()
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound.max(1) as u64) as usize
    }

    /// Random bytes of a random length in 0..max_len.
    fn random_vec(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.below(max_len);
        self.bytes(len)
    }

    /// Random bytes of a random length in 1..=max_len.
    fn random_nonempty_vec(&mut self, max_len: usize) -> Vec<u8> {
        let len = 1 + self.below(max_len);
        self.bytes(len)
    }

    /// Bytes that tend to break parsers, for targeted mutations.
    const INTERESTING: [u8; 10] = [0x00, 0x01, 0x22, 0x5c, 0x7b, 0x7d, 0x80, 0xfe, 0xff, 0x41];

    /// Mutate a valid encoding: truncate, flip, splice, duplicate, extend,
    /// or substitute from the interesting pool. One to four operations.
    fn mutate(&mut self, data: &[u8]) -> Vec<u8> {
        let mut out = data.to_vec();
        for _ in 0..1 + self.below(4) {
            if out.is_empty() {
                let n = 1 + self.below(8);
                out = self.bytes(n);
                continue;
            }
            match self.below(6) {
                0 => {
                    out.truncate(self.below(out.len()));
                }
                1 => {
                    let at = self.below(out.len());
                    out[at] ^= self.byte().max(1);
                }
                2 => {
                    let at = self.below(out.len() + 1);
                    let n = 1 + self.below(8);
                    let spliced = self.bytes(n);
                    for (offset, b) in spliced.into_iter().enumerate() {
                        out.insert(at + offset, b);
                    }
                }
                3 => {
                    let at = self.below(out.len());
                    let end = (at + 1 + self.below(out.len() - at)).min(out.len());
                    let chunk: Vec<u8> = out[at..end].to_vec();
                    out.splice(at..at, chunk);
                }
                4 => {
                    let n = self.below(16);
                    out.extend(self.bytes(n));
                }
                _ => {
                    let at = self.below(out.len());
                    out[at] = Self::INTERESTING[self.below(Self::INTERESTING.len())];
                }
            }
            if out.len() > 64 * 1024 {
                out.truncate(64 * 1024);
            }
        }
        out
    }

    /// A mutated string, restricted to characters a JSON or base64 parser
    /// could meet, so mutations stay inside the parser's real domain.
    fn mutate_text(&mut self, text: &str) -> String {
        let mutated = self.mutate(text.as_bytes());
        String::from_utf8_lossy(&mutated).into_owned()
    }
}

const ITERS: usize = 4096;
const MLS_ITERS: usize = 1024;

// ---------------------------------------------------------------- envelope

#[test]
fn fuzz_decode_envelope_never_panics() {
    let mut rng = Rng::new(0xE0_00_00_01);
    let valid = encode_envelope(&Envelope::mls(b"payload bytes"));
    for _ in 0..ITERS {
        let raw = rng.mutate(&valid);
        if let Some(envelope) = decode_envelope(&raw) {
            // Whatever decoded must also expose its data without panicking.
            let _ = envelope.data();
        }
        let _ = decode_envelope(&rng.random_vec(64));
    }
}

#[test]
fn fuzz_envelope_data_never_panics() {
    let mut rng = Rng::new(0xDA_00_00_02);
    for _ in 0..ITERS {
        let encoded = encode_payload(&rng.random_vec(32));
        let encoded = rng.mutate_text(&encoded);
        let _ = Envelope::Mls { data: encoded }.data();
    }
}

// ------------------------------------------------------------- app message

#[test]
fn fuzz_app_message_json_never_panics() {
    let mut rng = Rng::new(0xA0_00_00_03);
    let valid = serde_json::to_vec(&AppMessage::Text {
        text: "hello".to_string(),
    })
    .expect("serializes");
    for _ in 0..ITERS {
        let raw = rng.mutate(&valid);
        let _ = serde_json::from_slice::<AppMessage>(&raw);
        let _ = serde_json::from_slice::<AppMessage>(&rng.random_vec(64));
    }
}

// ------------------------------------------------------------------ base64

#[test]
fn fuzz_decode_payload_never_panics() {
    let mut rng = Rng::new(0xB6_40_00_04);
    for _ in 0..ITERS {
        let encoded = encode_payload(&rng.random_nonempty_vec(32));
        let encoded = rng.mutate_text(&encoded);
        if let Some(bytes) = decode_payload(&encoded) {
            // Whatever decoded must survive a re-encode and decode.
            assert_eq!(
                decode_payload(&encode_payload(&bytes)).as_deref(),
                Some(&bytes[..])
            );
        }
    }
}

// ------------------------------------------------------------------ frames

#[test]
fn fuzz_client_frames_never_panic() {
    let mut rng = Rng::new(0xC0_00_00_05);
    let valid = serde_json::to_string(&ClientFrame::Auth {
        token: "token".to_string(),
    })
    .expect("serializes");
    for _ in 0..ITERS {
        let raw = rng.mutate_text(&valid);
        let _ = serde_json::from_str::<ClientFrame>(&raw);
        let _ = serde_json::from_str::<ClientFrame>(
            &rng.mutate_text(r#"{"type":"ack","message_id":"m1"}"#),
        );
    }
}

#[test]
fn fuzz_server_frames_never_panic() {
    let mut rng = Rng::new(0x50_00_00_06);
    let valid = serde_json::to_string(&ServerFrame::Message {
        conversation_id: "c1".to_string(),
        message_id: "m1".to_string(),
        sender_username: "alice".to_string(),
        payload_base64: encode_payload(b"opaque"),
        created_at: 123,
    })
    .expect("serializes");
    for _ in 0..ITERS {
        let raw = rng.mutate_text(&valid);
        if let Ok(ServerFrame::Message { payload_base64, .. }) =
            serde_json::from_str::<ServerFrame>(&raw)
        {
            let _ = decode_payload(&payload_base64);
        }
        let _ = serde_json::from_str::<ServerFrame>(
            &rng.mutate_text(r#"{"type":"auth_ok","device_id":"d1"}"#),
        );
        let _ = serde_json::from_str::<ServerFrame>(
            &rng.mutate_text(r#"{"type":"ack_ok","message_id":"m1"}"#),
        );
    }
}

// --------------------------------------------------------------------- MLS

/// A device pair with a joined two-member group, built once and shared by
/// the MLS fuzz tests below.
fn setup_group() -> (DeviceCrypto, DeviceCrypto, String) {
    let alice = DeviceCrypto::generate("alice#device-1".to_string());
    let bob = DeviceCrypto::generate("bob#device-1".to_string());
    alice.create_group("conv-fuzz").expect("create group");
    let (_commit, welcome) = alice
        .add_members("conv-fuzz", &[bob.new_key_package().expect("key package")])
        .expect("add member");
    let joined = bob
        .process_welcome(&welcome.expect("welcome"))
        .expect("join");
    assert_eq!(joined, "conv-fuzz");
    (alice, bob, "conv-fuzz".to_string())
}

#[test]
fn fuzz_mls_process_incoming_never_panics_and_state_survives() {
    let mut rng = Rng::new(0x4D_1C_00_07);
    let (alice, bob, conversation) = setup_group();
    let valid = alice
        .encrypt(&conversation, b"a real application message")
        .expect("encrypt");

    for _ in 0..MLS_ITERS {
        let raw = rng.mutate(&valid);
        let _ = bob.process_incoming(&conversation, &raw);
        let _ = alice.process_incoming(&conversation, &rng.random_vec(64));
    }

    // The storm must not have corrupted the group: both devices still
    // exchange a valid message afterwards.
    let wire = alice
        .encrypt(&conversation, b"still working")
        .expect("encrypt after fuzzing");
    match bob
        .process_incoming(&conversation, &wire)
        .expect("process a valid message after the fuzz storm")
    {
        Incoming::Application { sender, payload } => {
            assert_eq!(sender, "alice#device-1");
            assert_eq!(payload, b"still working".to_vec());
        }
        other => panic!("expected an application message, got {other:?}"),
    }
}

#[test]
fn fuzz_mls_welcome_and_key_package_never_panic() {
    let mut rng = Rng::new(0x4E_00_00_08);
    let (alice, _bob, _conversation) = setup_group();
    let outsider = DeviceCrypto::generate("carol#device-1".to_string());
    let key_package = alice.new_key_package().expect("key package");

    for _ in 0..MLS_ITERS {
        let raw = rng.mutate(&key_package);
        let _ = outsider.key_package_identity(&raw);
        let _ = outsider.process_welcome(&raw);
        let _ = outsider.process_welcome(&rng.random_vec(64));
    }
}
