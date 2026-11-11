# Architecture

Thrum is a small end-to-end encrypted messenger: one Rust server that relays
ciphertext, and clients that hold the keys. Everything a client needs to speak
the protocol lives in one core crate, so the CLI client and the web client
behave identically.

Status markers below say how far the implementation has come; update them as
milestones land.

## Components

| Part | Crate or directory | Role |
| --- | --- | --- |
| Core | `crates/core` | Protocol types and client-side logic: validation, MLS group handling, message framing. No I/O of its own. |
| Server | `crates/server` | `axum` + `tokio`. Accounts, device registry, key package directory, conversation membership, message relay over WebSocket, blob storage. |
| CLI client | `crates/cli` | A command line user agent that drives the server through the same API the web client uses. Also the driver for end-to-end tests. |
| Web client | `web/` (from M6) | Vite + TypeScript UI using the core through WebAssembly. Keys stay in the browser. |

## Trust boundaries

The server is untrusted for confidentiality. Clients treat every payload they
receive from the server as attacker-controlled input, and the server never
holds a decryption key for any conversation. The TLS connection protects
transport; it does not make the server a party the users can trust with
content.

## Message flow (target, as of M3)

1. A device registers its account, then uploads MLS key packages (RFC 9420).
2. To start a conversation with someone, a client fetches one key package for
   each target device from the server's key package directory.
3. The initiating client builds the MLS group (a two-member group for a
   one-to-one chat) and sends the MLS commit and welcome messages through the
   server. The server checks membership requests against its conversation
   records but cannot read inside them.
4. Application messages are MLS application messages: opaque ciphertext plus
   routing metadata. The server forwards them to the other devices in the
   conversation over WebSocket, and queues them for devices that are offline
   until they reconnect or the queue expires.
5. Every device of every member is a member of the MLS group, so all of a
   user's devices see the same history from the epoch they joined.

## What the server can and cannot see

Can see:

- Account usernames, device identities and session activity.
- Conversation membership and membership changes (who is in which group, and
  when members are added or removed).
- Ciphertext payloads, their sizes, timing and ordering.
- IP addresses and connection times.
- Encrypted blob storage for attachments (ciphertext only).

Cannot see:

- Message plaintext, for any conversation.
- Message content metadata carried inside encrypted payloads (replies,
  edits, deletions, read receipts, attachment names and keys).
- Decryption keys of any kind. The server is never an MLS group member.

## Data stores

- SQLite for development, tests and small installs; Postgres supported from
  M7, behind the same query layer (decision 0004).
- Attachments (from M5) are opaque encrypted blobs with a size limit, stored
  on local disk with an interface that later allows S3-compatible storage.

## Testing

- `cargo test` covers unit and integration tests per crate.
- End-to-end tests start a real server and drive two CLI clients through it,
  including one client that goes offline and comes back (M2 onward).
- From M6, Playwright tests the web client in a real browser, in CI too.
