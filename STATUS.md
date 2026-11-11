# Status

Updated after every working session.

- **Current milestone:** M2 (not started)
- **Last done:** M1 done: register/login/logout, Argon2id, case-insensitive unique usernames, device sessions with hashed bearer tokens, device list/revoke, key package upload + one-time fetch, per-IP rate limits. 18 tests green.
- **Next step:** M2 message relay: conversations, membership, WebSocket delivery, offline queue, acks, CLI client send/receive, end-to-end test.
- **Blocked on:** nothing.

## Log

Newest first. One or two lines per session: what changed, what is next.

- M1: accounts and devices with sessions, key package directory, rate limiting; 11 new server integration tests.
- M0: Cargo workspace (core, server, cli), CI (fmt, clippy, test), architecture and threat model, SECURITY/CONTRIBUTING, decision 0004.
