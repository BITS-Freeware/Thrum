# Decision 0006: MLS integration design

Date: 2026-11-11
Status: accepted

## Context

M3 makes every message payload an MLS (RFC 9420) application message. The
roadmap fixes the library (`openmls` with `openmls_rust_crypto`), so the
decision here is how Thrum uses it, not which library to use.

## Choices

1. **Every device is an MLS member.** The MLS credential identity of a device
   is the string `username#device-id` in a `BasicCredential`. A user with two
   devices has two leaves in every group they belong to. This matches the
   roadmap: multi-device is per-device group membership, not shared keys.
2. **Group id = conversation id.** The MLS `GroupId` is the server-assigned
   conversation UUID as bytes, so clients can map relay conversations to MLS
   groups without extra state.
3. **Ciphersuite** `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`, one for the
   whole system. Wire format policy `PURE_CIPHERTEXT`: handshake messages
   (commits, welcomes' group info) and application messages all travel as MLS
   ciphertext. The server already knows conversation membership because it
   enforces fan-out, so hiding membership changes buys nothing extra.
4. **Ratchet tree travels inside the Welcome.** Groups are created with the
   ratchet-tree extension enabled, so joiners need only the Welcome, which the
   creator sends as an ordinary (opaque) conversation payload.
5. **Payload envelope (client-side, inside the opaque relay payload).** The
   relay payload is JSON: `{"v":1,"kind":"welcome"|"mls","data":<base64>}`.
   `welcome` carries a TLS-serialized `Welcome`, `mls` a TLS-serialized MLS
   message (commit or application). The server cannot tell them apart.
6. **Control flow per conversation event:**
   - create conversation: server-side membership first, then one `add_members`
     over the key packages of *all current devices of all members*, welcome
     sent first (joiners need it), then the commit.
   - add member: server-side membership first, then `add_members` with the new
     devices' key packages, commit then welcome.
   - remove member: MLS `remove_members` and the commit is sent *before* the
     server-side member removal, so the removed member still receives and
     processes it (forward secrecy) before the server stops delivering to
     them.
   - new device of an existing member: decision 0013 replaces the silent
     add. Devices present when the user adds the account are added then;
     a device that shows up later stays pending until that user trusts it.
7. **Key package rotation.** Devices upload a stock of key packages at login
   and top up to the stock level whenever it runs low (`GET
   /v1/key-packages/count` for the caller's own device). Fetching consumes
   one-time packages. Decision 0013 requires a session for that fetch, caps
   the one-time stock, and keeps one last-resort package that is not deleted.
   Before each top-up a device lists its own stored packages (`GET
   /v1/key-packages`) and withdraws any it cannot open (`DELETE
   /v1/key-packages/{id}`). That happens after its state was restored from
   a copy taken before its last top-up; a Welcome built on such a package
   could never be read.
8. **Tolerant receive.** Clients drop MLS messages they cannot process (old
   epochs, messages from before they joined, welcomes meant for another
   device, duplicates). This is required because the relay fan-out and MLS
   epochs cannot be perfectly aligned.
9. **Key material at rest.** MLS key material lives in the client's state
   directory with restrictive permissions, unencrypted. Encrypting it at rest
   (device unlock passphrase) is future work; the stolen-device adversary is
   documented in the threat model as a limitation.

## Consequences

- The server never sees plaintext; it keeps relaying opaque payloads plus
  routing metadata (conversation id, member list, timestamps).
- A malicious or buggy server that replays old payloads cannot decrypt them
  for a member added later or a member that was removed; tests cover this.
- The M7 Postgres port must replace SQLite-only SQL (`rowid` ordering) with
  portable queries behind the existing query layer.
