# 0010: Data export and account deletion

Date: 2026-11-12
Status: accepted
Milestone: M7 (hardening and 0.1.0)

The roadmap asks for an export of a user's own data and account deletion.
This decides what both mean for a server that holds only ciphertext.

## Export

`GET /v1/account/export` answers one JSON document holding everything the
server knows about the calling user:

- `account`: username and creation time;
- `devices`: id, label, creation and revocation times;
- `conversations`: id, name, creation time, when the user joined, the
  member list and the owner, for every conversation the user belongs to;
- `messages`: every message in those conversations — id, conversation,
  sender name, time, and the payload the server stored;
- `blobs`: the user's uploaded attachments as id, size, and timestamps.

Message payloads are exported exactly as stored: ciphertext. The server
cannot decrypt them (decision 0006), and the export must not become a
side channel around that. A client that keeps its keys can decrypt its
copy; the export itself is proof of what the server sees. The document is
plain JSON, `Content-Disposition: attachment`, no pagination — a small
server's export fits in memory, and an operator who worries about size
sets the message TTL so undelivered data does not accumulate.

## Deletion

`DELETE /v1/account` takes the account password in the body. A live
session token alone is not enough: deleting an account is the one action
a stolen token must not be able to do quietly. Wrong password answers
`401 invalid_credentials` and deletes nothing.

Deletion removes, in foreign-key-safe order:

1. the user's uploaded blob objects from the blob store, then their
   `blobs` rows — so the ciphertext bytes really leave the disk;
2. invites the user minted (`invites.created_by` has no cascade);
3. messages the user's devices sent (`messages.sender_device` has no
   cascade); their delivery rows go with them;
4. the user's devices — cascading their key packages and queued
   deliveries, and killing every session at once;
5. ownership of conversations the user owned passes to the member who
   joined earliest, or to nobody when no member remains;
6. the user row itself — cascading memberships and blocks;
7. conversations left with no members at all — cascading their messages
   and invites.

Recipients keep the copies they already hold on their own devices; as
with message deletion (decision 0007), the server cannot reach into
other people's machines, and the threat model says so.

## Foreign keys on SQLite

The explicit order above assumes cascades fire, but `PRAGMA
foreign_keys` defaults to off per SQLite connection, so until now
cascades never fired on SQLite and nothing enforced the schema's
`REFERENCES` clauses there. `db.rs` now turns `foreign_keys` on for
every SQLite connection it opens, matching PostgreSQL's always-on
enforcement. Deletion works identically on both backends; existing
write paths only ever inserted rows that satisfy the constraints.

## Client surface

The CLI grows `export` (writes the JSON document to a file, default
`thrum-export.json`) and `delete-account --password ... --yes` (the
`--yes` flag is a second, local confirmation; on success the CLI removes
the saved session file because its token is dead, and leaves the local
message store in place). The web client can surface both later; the API
is what other clients build on.

## Limitations

- Export is a single JSON document; no streaming or partial export.
- Deleting the last admin's account hands ownership to the earliest
  remaining member; a group the deleted user owned alone disappears.
- The export contains member usernames of shared conversations — that
  is data the server already shows to members through the API.
