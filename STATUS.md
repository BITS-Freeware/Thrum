# Status

Updated after every working session.

- **Current milestone:** M7 (hardening + 0.1.0)
- **Last done:** M7.1 PostgreSQL support (decision 0009) — full server integration suite green on both SQLite and PostgreSQL (PostgreSQL 17.11 verified locally; CI runs both via a postgres:17 service). One query set with `$N` placeholders through sqlx's any driver, `migrations-pg/` mirroring `migrations/`, backend picked by the `GEIST3_DB` connection string, `seq` identity columns for stable catch-up order on Postgres, case-insensitive usernames via `lower()` on both backends, `GEIST3_TEST_DB_URL` redirects the whole server suite to Postgres. M6 closed with CI green (run 599fee4: rust + web jobs, incl. first CI-run Playwright e2e). Earlier fixes: CLI e2e parallel flake root-caused (same-clock-tick temp-dir collisions shared state files; state_dir now uses a per-process counter; Chat::open rejects foreign-identity key material) and the CI web job never installed node_modules (added npm ci).
- **Next step:** M7.2 self-hosting (env-var config completion, Docker Compose, guide), then data export + account deletion, fuzz tests, threat-model pass, CHANGELOG + v0.1.0 tag.
- **Blocked on:** nothing.

## Log

Newest first. One or two lines per session: what changed, what is next.

- M7.1: PostgreSQL support (decision 0009) — one portable query set (`$N` placeholders, sqlx any driver), migrations-pg mirror, GEIST3_DB selects the backend by connection string, seq identity columns for catch-up order, lower() username lookups, GEIST3_TEST_DB_URL runs the whole server suite against Postgres; CI gained a postgres:17 service job. Suite green on both backends.
- M6 wrap-up: web e2e green 3× (register, live DM both ways, named group, rename, encrypted attachment). Root causes fixed client-side: void-transaction hang in withStore; concurrent WS frame handling broke id-dedup and MLS ordering; silent swallow+ack lost messages (now un-acked frames are re-delivered); concurrent renders stacked duplicate screens; group-name input was mis-parented. Port 8081 for e2e, artifacts gitignored, diagnostics removed. CLI e2e parallel flake fixed: same-clock-tick temp-dir collisions shared state files across tests (state_dir now uses a per-process counter) and Chat::open now rejects key material that does not belong to the session's device; workspace gate green 4×. CI web job was missing npm ci — fixed; full CI green on 599fee4.
- M6: web client — Vite+TS, IndexedDB persistence, live WS, auth/chat/group/devices/attachments screens, dark accessible UI. Wasm crate for web build. Full local gate green.
- M5: attachments (decision 0008) — core per-file AES-GCM with blob-id AAD binding; server blob endpoints (disk + S3 via object_store) with size cap, daily quota, TTL sweeper (migration 0004); CLI attach/fetch with image downscale/recompress, sha256 verify, sanitized saves, full metadata in the local store; 8 blob-route server tests + attachments CLI e2e (8 total).
- M4: conversation features (decision 0007) — group names + admin role, invite links with revoke, blocking both directions, encrypted edits/deletes/read receipts in AppMessage; new messages/deliveries columns via migration 0003; sender-side sent-message recording; groups.rs server tests + 3 new CLI e2e tests (7 total).
- M3: MLS end-to-end encryption (decision 0006) — core mls.rs over OpenMLS, envelope format, members/key-package endpoints, CLI MLS flow, two persistence fixes, 21 new tests incl. security properties and DB plaintext scan.
- M2: message relay — conversations/messages/deliveries schema, WS hub with per-device channels, queue + TTL, acks, CLI client, 15 new tests incl. a two-process e2e.
- M1: accounts and devices with sessions, key package directory, rate limiting; 11 new server integration tests.
- M0: Cargo workspace (core, server, cli), CI (fmt, clippy, test), architecture and threat model, SECURITY/CONTRIBUTING, decision 0004.
