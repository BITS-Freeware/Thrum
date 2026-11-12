# Status

Updated after every working session.

- **Current milestone:** M6 (not started)
- **Last done:** M5 done: encrypted attachments end to end (decision 0008). Server stores opaque ciphertext blobs — disk or S3-compatible backends behind one `BlobStore` trait, client-chosen 128-bit hex ids sent via `x-geist3-blob-id`, per-id conflict check, size cap (stable `too_large`/413), per-user daily byte quota (`quota_exceeded`/429), retention TTL with a sweeper in the background loop; route body limit raised above the cap so the handler owns the error. Core binds the per-file AES-GCM key to the blob id as associated data; the key and nonce travel only inside the MLS message. CLI `attach` scales images to ≤2048 px and recompresses JPEG q85 above 1 MiB, `fetch` decrypts, verifies the SHA-256, and saves under a sanitized name (default `<state-file>.attachments/`); the store keeps full attachment metadata per message. Tests: 8 server blob-route tests (roundtrip, bad ids, cap, quota, duplicate id, sweeper), image pipeline unit tests, and a full CLI e2e — attach → encrypted notice → fetch → byte-identical file, with the server db and every stored blob verified plaintext- and filename-free. Full local gate green: fmt, clippy --workspace --all-targets -D warnings, cargo test (93 tests).
- **Next step:** M6 per ROADMAP.md — read it first; likely multi-device sync/push polish or release prep, whatever the milestone defines.
- **Blocked on:** nothing.

## Log

Newest first. One or two lines per session: what changed, what is next.

- M5: attachments (decision 0008) — core per-file AES-GCM with blob-id AAD binding; server blob endpoints (disk + S3 via object_store) with size cap, daily quota, TTL sweeper (migration 0004); CLI attach/fetch with image downscale/recompress, sha256 verify, sanitized saves, full metadata in the local store; 8 blob-route server tests + attachments CLI e2e (8 total).
- M4: conversation features (decision 0007) — group names + admin role, invite links with revoke, blocking both directions, encrypted edits/deletes/read receipts in AppMessage; new messages/deliveries columns via migration 0003; sender-side sent-message recording; groups.rs server tests + 3 new CLI e2e tests (7 total).
- M3: MLS end-to-end encryption (decision 0006) — core mls.rs over OpenMLS, envelope format, members/key-package endpoints, CLI MLS flow, two persistence fixes, 21 new tests incl. security properties and DB plaintext scan.
- M2: message relay — conversations/messages/deliveries schema, WS hub with per-device channels, queue + TTL, acks, CLI client, 15 new tests incl. a two-process e2e.
- M1: accounts and devices with sessions, key package directory, rate limiting; 11 new server integration tests.
- M0: Cargo workspace (core, server, cli), CI (fmt, clippy, test), architecture and threat model, SECURITY/CONTRIBUTING, decision 0004.
