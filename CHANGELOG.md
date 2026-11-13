# Changelog

All notable changes to Geist3 are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/).

## [0.1.0] - 2026-11-13

First release: a small, end-to-end-encrypted messenger with a self-hostable
server, a CLI client and a web client.

### Added

- **Accounts and devices.** Registration, login, multiple devices per user
  with per-device session tokens, device listing and revocation, and MLS key
  packages uploaded and served per device. Passwords are stored as Argon2id
  hashes with per-user salts.
- **Message relay.** Conversations, delivery of MLS ciphertext over
  WebSocket, an offline queue that replays undelivered messages, and
  delivery acknowledgements. The server never holds group keys and relays
  only opaque payloads.
- **End-to-end encryption (MLS).** Every conversation is an MLS group
  (via OpenMLS, DHKEMX25519/Ed25519 ciphersuite). Members join through
  welcomes, removed members cannot read later messages, newly added members
  cannot read earlier ones, and every application message is authenticated
  to its sender's leaf.
- **Conversation features.** Group names, an admin role, invite links,
  blocking, replies, edits, deletions and read receipts — all carried
  inside the encrypted payload.
- **Attachments.** Encrypted before upload with a per-file key bound to the
  blob id; the server stores only ciphertext, with per-user upload quotas,
  retention expiry, a disk or S3-compatible backend, and integrity checks
  on download. Large images are recompressed client-side before sending.
- **Web client.** A TypeScript client served by the server itself: login,
  conversation list, chat, group management, devices, and attachments, with
  live updates over WebSocket.
- **CLI client.** A terminal client with the same feature set, including
  an end-to-end test mode used by the integration tests.
- **Deployment.** SQLite (default) and PostgreSQL support, configuration
  through environment variables, a Dockerfile, a Docker Compose file, and
  a self-hosting guide covering storage, TLS termination and updates.
- **Data export and account deletion.** A user can export their own data
  and delete their account and devices.
- **Hardening.** Fuzz tests for every parser that reads network data (plus
  libFuzzer targets under `fuzz/`), rate limiting of registration and
  login, and per-device key-package validation in the clients.

### Security

- The threat model (`docs/threat-model.md`) maps every adversary capability
  to a test, a mitigation, or a documented limitation, and was re-checked
  against the shipped code for this release.
- Fuzzing found and fixed a reachable crash: malformed MLS input could
  panic inside the `tls_codec` dependency; all OpenMLS parse entry points
  now guard against it and return typed errors (decision 0011).

### Known limitations

Documented in the threat model's "Out of scope in 0.1.0": no metadata
protection from the server operator, no at-rest encryption of client state,
TLS termination is an operational requirement rather than code-enforced,
and other items listed there.

[0.1.0]: https://github.com/Zyniker13/Geist3/releases/tag/v0.1.0
