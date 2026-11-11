# Roadmap

The goal is a small, trustworthy messenger: accounts, one-to-one and group chat, attachments, and end-to-end encryption, with a web client and a self-hostable server. Version 0.1.0 is done when every milestone below meets its "Done when" line.

Work runs milestone by milestone, in order. Each milestone ends with its checks passing in CI and an entry in [STATUS.md](STATUS.md). Iterating is expected: a later milestone may reopen an earlier one, and that change is recorded in `docs/decisions/`.

## Not in Scope

These stay out of the project, now and later: cryptocurrency, wallets or payments, mini-apps or a bot platform, ads, paid tiers, stories, and public channels. Phone numbers are never required.

Later, after 0.1.0, and not before: native mobile and desktop apps, push notifications, voice and video calls, and federation between servers.

## Stack

| Part | Choice |
| --- | --- |
| Language | Rust (stable) for the core and the server; TypeScript for the web client |
| Encryption | MLS (RFC 9420) via `openmls`, with `openmls_rust_crypto`; no hand-written cryptography |
| Server | `axum` and `tokio`; WebSocket for live delivery, HTTPS JSON for everything else |
| Storage | `sqlx` with SQLite for development, tests and small installs; Postgres supported by M7 |
| Passwords | Argon2id (`argon2` crate) |
| Web client | Vite and TypeScript, with the core compiled through `wasm-bindgen` |
| Tests | `cargo test` for unit and integration tests; a CLI client drives end-to-end tests |
| CI | GitHub Actions on GitHub-hosted runners: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and the web build once it exists |

Layout: a Cargo workspace with `crates/core`, `crates/server` and `crates/cli`, and `web/` for the web client.

## M0: Foundations

- Cargo workspace with the three crates, compiling and testing clean.
- CI workflow running format, lint and tests on every push and pull request.
- `docs/architecture.md`: components, the message flow, and what the server can and cannot see.
- `docs/threat-model.md`: assets, adversaries (curious server operator, network attacker, stolen device, malicious group member), and what is defended against in 0.1.0.
- `SECURITY.md` (how to report a vulnerability, using GitHub private vulnerability reporting), `CONTRIBUTING.md`, and the first decision records.

**Done when:** CI is green on `main` with at least one real test per crate, and the three documents exist.

## M1: Accounts and Devices

- Register, log in and log out with a username and password. Usernames are unique and case-insensitive; passwords are hashed with Argon2id.
- Each login creates a device with its own session token. A user can list and revoke their devices.
- Each device uploads MLS key packages, and others can fetch one to add that device to a conversation.
- Basic rate limiting on registration and login.

**Done when:** integration tests cover register, login, wrong password, duplicate username, device list and revoke, and key package upload and fetch.

## M2: Message Relay

- Conversations and membership on the server.
- Sending and receiving opaque message payloads over WebSocket, with an offline queue so a device gets what it missed when it reconnects.
- Delivery acknowledgements, and a cap on how long the server keeps undelivered messages.
- A CLI client (`crates/cli`) that can register, log in, create a conversation, send and receive.

**Done when:** an end-to-end test starts a server, runs two CLI clients, exchanges messages both ways, including one sent while the receiver is offline, and checks delivery.

## M3: End-to-End Encryption

- All message payloads are MLS application messages. The server sees only ciphertext, group IDs, and routing metadata.
- One-to-one chats are two-member MLS groups. Groups support adding and removing members.
- Multi-device: every device of every member is a member of the MLS group.
- Key package rotation and replacement of used key packages.

**Done when:** tests show that a member removed from a group can't read later messages, a newly added member can't read earlier ones, and the server database contains no plaintext. The threat model is updated to match.

## M4: Conversation Features

- Group names and an admin role that can add and remove members and rename the group.
- Invite links to join a group, which an admin can revoke.
- Replies, editing and deleting your own messages, and read receipts, all carried inside encrypted messages.
- Blocking another user.

**Done when:** each feature has tests through the CLI client, and edits, deletes and receipts are encrypted like ordinary messages.

## M5: Attachments

- Files are encrypted on the client with a per-file key, which is sent inside the MLS message. The server stores only the encrypted blob.
- A size limit, local disk storage on the server, and an S3-compatible storage backend. Object storage with free egress (such as Cloudflare R2) is the recommended production choice, since media bandwidth is the largest running cost.
- Expiry for blobs nobody can still fetch.

**Done when:** a test sends an image between two CLI clients, the server holds only ciphertext, and an oversized upload is refused.

## M6: Web Client

- The core compiled to WebAssembly and used from the TypeScript client.
- Screens: register and log in, chat list, conversation view, new chat, group details and members, attachments, and devices.
- Keys stay in the browser (IndexedDB) and never go to the server.
- Accessible: keyboard use, labels, and enough contrast.

**Done when:** a browser test (Playwright) has two users chat, create a group, and send an attachment, and CI runs it.

## M7: Hardening and 0.1.0

- Postgres support, alongside SQLite.
- Fuzz tests for every parser that reads data from the network.
- A pass over the threat model. Every item gets a test, a documented mitigation, or a documented limitation.
- Export of a user's own data, and account deletion.
- Self-hosting guide with a Docker Compose file, and configuration through environment variables.
- `CHANGELOG.md` and a `v0.1.0` tag.

**Done when:** all checks are green, the self-hosting guide works from a clean checkout, and the tag exists.
