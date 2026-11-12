# Status

Updated after every working session.

- **Current milestone:** M5 (not started)
- **Last done:** M4 done: group names (create `--name`, admin-only rename with length/control-char validation), server-side admin role, invite links (`invite`/`revoke`/`join`; server-held random tokens, redeem checks blocks), blocking (`block`/`unblock`; both directions blocked from new shared conversations and invite redeems), and encrypted AppMessage Edit/Delete/ReadReceipt variants so edits, deletes and read receipts ride inside MLS. Sender devices now record their own sent messages (so `edit`/`delete` work at once) and relay echoes no longer clobber store entries/read receipts. Server tests in `tests/groups.rs`; CLI e2e grew to 7 tests covering the name/invite/rotate flows, edit/delete/read receipts end-to-end (incl. "not yours to edit"), and blocking; server DB still plaintext-free. Full local check suite green: fmt, clippy --workspace --all-targets -D warnings, cargo test.
- **Next step:** M5 attachments: per-file client-side encryption with the key sent in the MLS message, encrypted blob storage on the server (disk + S3-compatible backend), size cap, per-user daily upload quota, client-side image resize/recompress, blob expiry — each limit operator-configurable.
- **Blocked on:** nothing.

## Log

Newest first. One or two lines per session: what changed, what is next.

- M4: conversation features (decision 0007) — group names + admin role, invite links with revoke, blocking both directions, encrypted edits/deletes/read receipts in AppMessage; new messages/deliveries columns via migration 0003; sender-side sent-message recording; groups.rs server tests + 3 new CLI e2e tests (7 total).
- M3: MLS end-to-end encryption (decision 0006) — core mls.rs over OpenMLS, envelope format, members/key-package endpoints, CLI MLS flow, two persistence fixes, 21 new tests incl. security properties and DB plaintext scan.
- M2: message relay — conversations/messages/deliveries schema, WS hub with per-device channels, queue + TTL, acks, CLI client, 15 new tests incl. a two-process e2e.
- M1: accounts and devices with sessions, key package directory, rate limiting; 11 new server integration tests.
- M0: Cargo workspace (core, server, cli), CI (fmt, clippy, test), architecture and threat model, SECURITY/CONTRIBUTING, decision 0004.
