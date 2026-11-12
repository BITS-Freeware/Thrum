# Status

Updated after every working session.

- **Current milestone:** M3 (not started)
- **Last done:** M2 done: conversation create/list/send with membership checks, WebSocket hub (auth-first handshake, live fan-out, register-before-catch-up), offline queue with 7-day TTL sweeper, client acks (Ack/AckOk), CLI register/login/create/send/recv, e2e test with two real CLI processes both ways. 33 tests green.
- **Next step:** M3 MLS group messaging: decide MLS library vs minimal hand-rolled protocol, key package-based joining, ciphertext-only payloads.
- **Blocked on:** nothing.

## Log

Newest first. One or two lines per session: what changed, what is next.

- M2: message relay — conversations/messages/deliveries schema, WS hub with per-device channels, queue + TTL, acks, CLI client, 15 new tests incl. a two-process e2e.
- M1: accounts and devices with sessions, key package directory, rate limiting; 11 new server integration tests.
- M0: Cargo workspace (core, server, cli), CI (fmt, clippy, test), architecture and threat model, SECURITY/CONTRIBUTING, decision 0004.
