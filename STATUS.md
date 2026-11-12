# Status

Updated after every working session.

- **Current milestone:** M7 (not started)
- **Last done:** M6 + fixes: web client (Vite+TS, IndexedDB, live WS with 120ms refresh coalescing, read receipts self-marked, draft clearing, auth/chat/group/devices/attachments screens). Wasm crate for web build. CLI e2e fix: Chat::open now checks state.json (M6 unified format) instead of stale identity.json, resolving 7/8 e2e failures. Full local gate green: fmt, clippy --workspace --all-targets -D warnings, cargo test (90 tests), web build.
- **Next step:** M7 — release build, final polish, tag 0.1.0 per ROADMAP.md.
- **Blocked on:** nothing.

## Log

Newest first. One or two lines per session: what changed, what is next.

- M6: web client — Vite+TS, IndexedDB persistence, live WS, auth/chat/group/devices/attachments screens, dark accessible UI. Wasm crate for web build. Full local gate green.
- M5: attachments (decision 0008) — core per-file AES-GCM with blob-id AAD binding; server blob endpoints (disk + S3 via object_store) with size cap, daily quota, TTL sweeper (migration 0004); CLI attach/fetch with image downscale/recompress, sha256 verify, sanitized saves, full metadata in the local store; 8 blob-route server tests + attachments CLI e2e (8 total).
- M4: conversation features (decision 0007) — group names + admin role, invite links with revoke, blocking both directions, encrypted edits/deletes/read receipts in AppMessage; new messages/deliveries columns via migration 0003; sender-side sent-message recording; groups.rs server tests + 3 new CLI e2e tests (7 total).
- M3: MLS end-to-end encryption (decision 0006) — core mls.rs over OpenMLS, envelope format, members/key-package endpoints, CLI MLS flow, two persistence fixes, 21 new tests incl. security properties and DB plaintext scan.
- M2: message relay — conversations/messages/deliveries schema, WS hub with per-device channels, queue + TTL, acks, CLI client, 15 new tests incl. a two-process e2e.
- M1: accounts and devices with sessions, key package directory, rate limiting; 11 new server integration tests.
- M0: Cargo workspace (core, server, cli), CI (fmt, clippy, test), architecture and threat model, SECURITY/CONTRIBUTING, decision 0004.
