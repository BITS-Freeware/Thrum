# Status

Updated after every working session.

- **Current milestone:** M4 (not started)
- **Last done:** M3 done: MLS end-to-end encryption via OpenMLS (0.9.2, ciphersuite MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519, wire policy PURE_CIPHERTEXT). Every device is an MLS member with identity `username#device-id`; conversation UUID is the MLS GroupId. Server keeps plaintext-free fan-out (memberships only); clients wrap MLS bytes in `{"kind":"welcome"|"mls","data":base64}` envelopes. CLI: create/add/remove/sync/send/recv with key-package top-up (min 4, stock 8), state dir `<state-file>.mls/` (identity.json + storage.json, 0600/0700). Fixed two persistence bugs (key-package private keys and welcome joins must be saved before exit) and added a key-package identity check so a malicious server cannot steer commits to a foreign device. 54 tests green incl. e2e proofs: removed member can't read later messages, added member can't read earlier ones, server DB holds no plaintext, second device joins via sync.
- **Next step:** M4 conversation features: group names, admin role, invite links, replies/edits/deletes/read receipts (all inside encrypted AppMessage variants), blocking.
- **Blocked on:** nothing.

## Log

Newest first. One or two lines per session: what changed, what is next.

- M3: MLS end-to-end encryption (decision 0006) — core mls.rs over OpenMLS, envelope format, members/key-package endpoints, CLI MLS flow, two persistence fixes, 21 new tests incl. security properties and DB plaintext scan.
- M2: message relay — conversations/messages/deliveries schema, WS hub with per-device channels, queue + TTL, acks, CLI client, 15 new tests incl. a two-process e2e.
- M1: accounts and devices with sessions, key package directory, rate limiting; 11 new server integration tests.
- M0: Cargo workspace (core, server, cli), CI (fmt, clippy, test), architecture and threat model, SECURITY/CONTRIBUTING, decision 0004.
