# Decision 0011: Fuzz testing for network-facing parsers

Date: 2026-11-13 (M7)

## Context

M7 requires fuzz tests for every parser that reads data from the network.
The parsers must be enumerated honestly, and the fuzzing must fit the
project's constraints: CI runs on GitHub-hosted runners with the stable
toolchain, and the local gate must stay identical to CI.

## Decision

Fuzzing happens in two layers.

**Layer 1 — fuzz tests in `cargo test` (stable, runs in CI).** Each parser
is driven with thousands of deterministic inputs per run: pure-random
bytes/strings from a seeded xorshift PRNG, plus structured mutations of a
valid encoding (truncate, flip, splice, duplicate, extend, byte-pool
substitution). The property under test is that parsing is total: no input
panics; every failure is a typed `Err` or `None`. For the MLS layer the
property extends to state: a device that has processed arbitrary garbage
must still decrypt a valid later message. Determinism comes from fixed
seeds, so failures reproduce from the seed alone.

**Layer 2 — libFuzzer targets under `fuzz/` (nightly, not in CI).** A
separate workspace crate, excluded from the main workspace, with one
`cargo fuzz` target per parser for deep coverage-guided runs. CI does not
run it (nightly setup would slow and destabilize CI for little gain over
the stable layer); developers run it locally, see CONTRIBUTING.md.

## The parser inventory

Every component that parses bytes, strings or JSON whose content crossed
the network:

- `core::envelope::decode_envelope` and `Envelope::data` — the relay
  payload (JSON envelope + base64), used by both clients through the
  `wasm` bindings and the CLI.
- `core::envelope::AppMessage` — JSON inside a decrypted MLS application
  message; written by the sender, parsed by every receiver.
- `core::protocol::{ClientFrame, ServerFrame}` — the WebSocket frame JSON;
  the server parses `ClientFrame`, both clients parse `ServerFrame`
  (server seam: `server::frames::parse_client_frame`).
- `core::protocol::decode_payload` — base64 of `payload_base64`.
- The MLS parsers behind `core::mls::DeviceCrypto`:
  `process_incoming`, `process_welcome` and `key_package_identity` /
  `add_members` hand attacker-controllable bytes (the server relays
  payloads and serves key packages) to OpenMLS's TLS-codec parsers. The
  fuzz property is that the Thrum wrapper never panics and leaves the
  device's group state intact and usable.
- `cli::files::{sanitize_name, mime_from_name, split_name}` — attachment
  names arrive from other users inside encrypted messages and are used to
  build local file paths.

## Reviewed and deliberately out of scope

- HTTP request and response bodies on both sides go through axum/reqwest
  JSON extractors and serde-derived types; malformed input is rejected
  with a 4xx or mapped error, never parsed by hand.
- `image::load_from_memory` (attachment recompression) only ever sees
  files the local user chose to send. Received attachments are saved
  verbatim after the sha-256 check; rendering happens in the browser's
  sandboxed image decoder. No Thrum code decodes received image bytes.
- `DeviceCrypto::from_json` and the CLI session store parse files the
  client itself wrote to local storage, not network data.
- The blob upload path stores raw bytes opaquely, without parsing.

## What the fuzz layer found immediately

The first run of the MLS fuzz tests crashed inside the dependency stack,
not in Thrum code: `tls_codec` 0.5.0 asserts (rather than returning an
error) when a vector length prefix announces more bytes than the input
holds, reachable through `process_welcome` and `key_package_identity`
with attacker-controlled bytes. The mitigation lives at the Thrum
boundary: `core::mls::guard_panics` wraps the four OpenMLS parse entry
points (`key_package_identity`, `add_members` package validation,
`process_incoming` deserialization, `process_welcome` join) in
`std::panic::catch_unwind` and turns the panic into
`MlsError::Other("malformed MLS input: …")`. The guards wrap only
parsing and validation, which happen before any group state is touched,
so a caught panic leaves the device intact; panics from later,
already-validated steps stay loud. A malformed payload may still print
the dependency's panic location to stderr before it is caught; that is
diagnostic noise, not a crash.

## Consequences

- `crates/{core,server,cli}/tests/fuzz.rs` exist and run in every `cargo
  test`, locally and in CI.
- `fuzz/` is a standalone crate; it must keep its own `[workspace]` table
  so the root workspace's explicit member list stays authoritative.
- New network-facing parsers must add a target in both layers.
