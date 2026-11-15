# Changelog

All notable changes to Thrum are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-11-14

### Security

- Key-package fetch requires a session, is rate-limited per caller and per
  target device, and stops at a per-device cap. One last-resort package is
  kept and is not deleted, so a group can still be formed after the one-time
  stock is empty (decision 0013).
- A device that appears after its account was added is not joined to existing
  conversations until the user trusts it. Both clients show that device and
  can accept it. Devices present at the moment the user adds the account are
  still trusted then.
- Revoking a device removes its MLS leaf on the next sync by a remaining
  member, so a message sent after that removal is unreadable to the revoked
  device. The threat model states that test.
- Rate limits use `X-Forwarded-For` or `Forwarded` only when the immediate
  peer is in `THRUM_TRUSTED_PROXIES`. Login has a separate per-account limit
  and an exponential backoff after repeated failures.
- Responses send a Content-Security-Policy (including
  `script-src 'self' 'wasm-unsafe-eval'`), `nosniff`, `no-referrer`, and
  `frame-ancestors 'none'` / `X-Frame-Options: DENY`.
- The default listen address is `127.0.0.1`. Binding any other address prints
  a warning that passwords and tokens are in the clear without TLS. Compose
  publishes `127.0.0.1:8080:8080`. The container image still sets
  `THRUM_BIND=0.0.0.0` so a proxy on the host can reach it, and it runs as
  user `thrum` (uid 10001) after the entrypoint gives that user `/data`.

### Added

- Invite expiry and a maximum number of uses. A request that omits them gets
  7 days and one use. `0` means no expiry or unlimited uses.
- A member cap, `THRUM_MAX_GROUP_MEMBERS` (default 50, `0` unlimited).
- `GET /v1/meta` and a source link in the web client, for the AGPL offer of
  source. `THRUM_SOURCE_URL` overrides the default repository URL.
- `rust-toolchain.toml` pins Rust 1.91.0, which is the minimum the workspace
  declares.

### Changed

- **Renamed to Thrum.** Crates are `thrum-core`, `thrum-server`, `thrum-cli`
  and `thrum-wasm`. Release 0.1.0 used a different environment-variable prefix;
  server settings use `THRUM_*`. Exports report `"format": "thrum-export"`.
  The repository is [BITS-Freeware/Thrum](https://github.com/BITS-Freeware/Thrum).
  See decision 0012.
- The web client's wasm package is built from source in CI and in the Docker
  image with wasm-bindgen-cli 0.2.130. Generated wasm files are no longer
  committed.
- Workspace and web client versions are 0.2.0.

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

[0.2.0]: https://github.com/BITS-Freeware/Thrum/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/BITS-Freeware/Thrum/releases/tag/v0.1.0
