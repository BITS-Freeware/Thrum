# Changelog

All notable changes to Thrum are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/).

## [Unreleased]

## [0.2.1] - 2026-11-22

This release fixes the issues described in the repository's
[security advisories](https://github.com/BITS-Freeware/Thrum/security/advisories)
for 0.2.0 and earlier. Upgrade the server and every client.

### Security

- Clients check every commit and welcome against the server's member list
  and refuse changes it does not back: adding a leaf that is not a listed
  device of a member, removing a member the server still lists, or joining a
  group from a welcome meant for another conversation. A refused change
  leaves the device as it was (decision 0014).
- The server binds a signature key to each device from its first key
  package and refuses packages with another key or another device's
  identity. Clients compare leaves with these keys.
- The sender shown for a message is the MLS credential that signed it. A
  frame whose relay label disagrees is dropped, and a message from a removed
  device is ignored.
- A failed welcome no longer uses up the one-time key package it named.
- One device runs in one place at a time. A CLI command waits while another
  command uses the same device, and the web client runs in one tab and shows
  a notice in the others (decision 0015).
- The server keeps one live socket per device: a new connection closes the
  older one, a closing connection no longer unregisters a newer one, and
  revoking a device, logging out or deleting the account closes its socket.
- The CLI replaces its state files atomically, readable by the owner only,
  and refuses to start from a damaged message store instead of silently
  forgetting trust decisions.
- Clients pin each device's signature key on first sight. A known device with
  another key waits for the user, trust on first use is per account rather
  than per conversation, and a device listed again after its removal waits
  for the user.
- A deleted account's name cannot be registered again.
- The web client applies frames it has received before sending.
- Login limits keep only canonical names (every invalid name shares one
  key), forget old failures, and cap login and registration bodies at
  16 KiB, so failed logins no longer grow server memory without bound.
- The login lockout and per-account limit are kept per client address, so a
  stranger's guesses no longer lock the owner out. A new hourly ceiling
  (`THRUM_LOGIN_FAILURES_PER_ACCOUNT_PER_HOUR`, default 100) bounds guesses
  spread across addresses. IPv6 clients are limited per /64.
- Wrong passwords on account deletion count toward the login limits, and
  password hashing runs off the async workers.
- A block between any two people in a conversation refuses creating it,
  adding to it and redeeming its invites, not only blocks involving the
  caller (decision 0016).
- A member the admin removed cannot rejoin through an invite until the admin
  adds them again.
- An account export holds only messages the account sent or was sent.
- Deleting an account keeps messages it sent that other members have not
  received yet, so members who were offline still get the commits they
  need; clients accept such a commit from an account that has since left.
- The nginx example overwrites `X-Forwarded-For`, and the self-hosting guide
  and Compose file say which `THRUM_TRUSTED_PROXIES` value a proxied install
  needs.
- The member cap holds for invites too, and for adds and redemptions that
  overlap. Redeeming an invite again no longer spends a use (decision 0017).
- Sends are limited per device (`THRUM_MESSAGES_PER_DEVICE_PER_MINUTE`,
  default 120), and what an account has sent that is still queued is capped
  (`THRUM_QUEUED_BYTES_PER_ACCOUNT`, default 100 MiB).
- Catch-up on connect and the account export load messages a page at a
  time instead of all at once.
- Signed-in devices per account are capped
  (`THRUM_MAX_DEVICES_PER_ACCOUNT`, default 20), and device labels are at
  most 64 bytes.
- Parallel uploads can no longer pass the daily upload quota together.
- Key-package uploads and fetches of one device run one after another: the
  stock cap holds, a device keeps one last-resort package (the newest), and
  concurrent fetches on PostgreSQL take one-time packages while any remain.
  `THRUM_KEY_PACKAGE_MAX_PER_DEVICE=0` now means unlimited, as documented.
- Key-package fetches are also limited per caller and device
  (`THRUM_KEY_PACKAGE_FETCH_PER_CALLER_DEVICE_PER_MINUTE`, default 10).
  Past a device's overall ceiling, fetches get its last-resort package, so
  one user can no longer stop others from adding a device or drain its
  stock.
- The web client acknowledges messages it can never read (no envelope, bad
  base64, bytes that are not MLS for a group it is in), so they are not
  replayed on every reconnect for a week.
- The CLI escapes control characters and text-direction overrides in
  everything it prints, so message text, attachment names and server
  answers cannot move the cursor, rewrite earlier lines, set the window
  title or write the clipboard.
- The CLI no longer crashes on a reply whose quoted id has a multi-byte
  character where it is shortened.
- The CLI ignores an attachment whose blob id is not 32 hex characters, and
  builds every request path from encoded segments, so an id from a peer or
  the server cannot reach another route.
- The CLI writes its session file (with the device token), saved
  attachments, `fetch --out` files and exports readable by the owner only,
  like its keys and message store. `delete-account` also removes the keys,
  the history and the attachments directory.
- The server creates its SQLite database and its blob files readable by its
  own user only, and the container entrypoint sets `umask 077` and
  tightens an existing `/data` volume.

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

[Unreleased]: https://github.com/BITS-Freeware/Thrum/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/BITS-Freeware/Thrum/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/BITS-Freeware/Thrum/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/BITS-Freeware/Thrum/releases/tag/v0.1.0
