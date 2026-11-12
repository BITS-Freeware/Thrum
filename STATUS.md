# Status

Updated after every working session.

- **Current milestone:** M7 (not started)
- **Last done:** M6 complete — web Playwright e2e "two users" green 3× in a row (registration, live DM both ways, named group + live group delivery, rename, encrypted attachment round-trip). Fixes that got it there: (1) db.ts withStore resolved `undefined` for void transactions (registration hang); (2) live.ts serializes WS frame handling — the server delivers a message both live and via the catch-up queue and clients must dedupe by id (its own docs say so), plus MLS must see welcome before commit; (3) frames that cannot be applied are left un-acked so the server re-offers them after reconnect instead of being lost (interpret used to swallow errors and ack anyway); (4) main.ts serializes screen renders — two concurrent renders used to stack two headers/lists (the "duplicate hello bob" and double-heading artifacts); (5) the group-name input existed but was appended into the members label — fixed; e2e now fills the group name, so labels are unique on both sides; (6) e2e navigates back to the chat list before the attachment step. CLI e2e parallel flake root-caused and fixed: two tests starting in the same clock tick got identical temp-dir names (pid + timestamp) and shared state files — one test could load another's session ("state file belongs to …") or a torn session/crypto pair (key-package identity mismatch); state_dir now mixes in a per-process counter, and Chat::open fails loudly when loaded keys do not match the session's device. e2e server port moved 8080→8081 (8080 is occupied locally); Playwright artifacts now gitignored; one-off diagnostic scripts removed. Workspace gate green 4× in a row.
- **Next step:** M7 — release build, final polish, tag 0.1.0 per ROADMAP.md.
- **Blocked on:** nothing.

## Log

Newest first. One or two lines per session: what changed, what is next.

- M6 wrap-up: web e2e green 3× (register, live DM both ways, named group, rename, encrypted attachment). Root causes fixed client-side: void-transaction hang in withStore; concurrent WS frame handling broke id-dedup and MLS ordering; silent swallow+ack lost messages (now un-acked frames are re-delivered); concurrent renders stacked duplicate screens; group-name input was mis-parented. Port 8081 for e2e, artifacts gitignored, diagnostics removed. CLI e2e parallel flake fixed: same-clock-tick temp-dir collisions shared state files across tests (state_dir now uses a per-process counter) and Chat::open now rejects key material that does not belong to the session's device; workspace gate green 4×.
- M6: web client — Vite+TS, IndexedDB persistence, live WS, auth/chat/group/devices/attachments screens, dark accessible UI. Wasm crate for web build. Full local gate green.
- M5: attachments (decision 0008) — core per-file AES-GCM with blob-id AAD binding; server blob endpoints (disk + S3 via object_store) with size cap, daily quota, TTL sweeper (migration 0004); CLI attach/fetch with image downscale/recompress, sha256 verify, sanitized saves, full metadata in the local store; 8 blob-route server tests + attachments CLI e2e (8 total).
- M4: conversation features (decision 0007) — group names + admin role, invite links with revoke, blocking both directions, encrypted edits/deletes/read receipts in AppMessage; new messages/deliveries columns via migration 0003; sender-side sent-message recording; groups.rs server tests + 3 new CLI e2e tests (7 total).
- M3: MLS end-to-end encryption (decision 0006) — core mls.rs over OpenMLS, envelope format, members/key-package endpoints, CLI MLS flow, two persistence fixes, 21 new tests incl. security properties and DB plaintext scan.
- M2: message relay — conversations/messages/deliveries schema, WS hub with per-device channels, queue + TTL, acks, CLI client, 15 new tests incl. a two-process e2e.
- M1: accounts and devices with sessions, key package directory, rate limiting; 11 new server integration tests.
- M0: Cargo workspace (core, server, cli), CI (fmt, clippy, test), architecture and threat model, SECURITY/CONTRIBUTING, decision 0004.
