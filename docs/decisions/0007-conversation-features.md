# Decision 0007: Conversation features — admin, invites, blocking, message lifecycle

Date: 2026-11-11
Status: accepted

## Context

M4 adds the social machinery around the message relay of M2 and the MLS
encryption of M3: group names, an admin role, invite links, blocking, and
message lifecycle events (edits, deletes, read receipts). The roadmap fixes
the feature set, not how the pieces are modeled, so these choices settle the
modeling.

## Choices

1. **Exactly one admin per conversation, recorded server-side.** The creator
   becomes the admin; a conversation row carries the admin username. The
   server enforces admin-only mutations (add member, remove member, rename,
   mint and revoke invites) with `403 not_admin` because it is where
   membership is authoritative. Clients mirror the check for fast feedback.
   Admin transfer and multi-admin are future work; nothing in the schema
   assumes a single admin except the column.
2. **Invite tokens are bearer secrets, hashed at rest, shown once.** `invite`
   mints a 32-byte random hex token and returns it once; the server stores
   only its SHA-256 hash, exactly like session tokens. Redeeming exchanges
   the token for server-side membership; the cryptography still happens on
   clients, where a member's device pulls the newcomer into the MLS group on
   its next sync. Revoking deletes the row; revoking an unknown token is
   still `204` so tokens cannot be probed. Handing the token to a new member
   is out of band and is the admin's responsibility.
3. **Blocking is one-sided but checked in both directions for new state.**
   `block` stores the blocker/blocked pair. Creating a conversation that
   contains users with a block between any pair fails with `403 blocked`,
   whichever side calls; redeeming an invite from a user who blocks (or is
   blocked by) the redeemer fails the same way. Existing conversations are
   untouched — M4 blocks the start of new shared spaces, it does not police
   existing ones. Unblocking restores the ability.
4. **Edits, deletes and read receipts are MLS application messages.** They
   ride in new variants of the v1 `AppMessage` enum, so the server sees only
   opaque ciphertext and stays unable to tell an edit from a hello. A device
   applies an edit or delete only to a message it holds in its local store
   and only when the sender username matches the stored author; `edit`
   rewrites the text and `delete` strips it, both clearing that message's
   read receipts. Edit history is not kept (0.1.0 scope). Read receipts name
   message ids a device has received; receivers of the receipt mark their
   copy read.
5. **A sender records its own sent messages at send time.** Without this,
   `edit`/`delete` would only work after the sender's next receive round
   trip. Relay echoes of a device's own message must not clobber the local
   entry, because the entry may already carry read receipts from other
   devices.
6. **Group names are short text.** Trimmed, at most 255 bytes, no control
   characters; stored on the conversation row and returned by every
   conversation view.

## Consequences

The server stores no plaintext and learns only what it must (membership,
admin, invite hashes, block pairs). The MLS group remains the sole carrier of
content semantics: a server can censor or drop lifecycle events but cannot
forge or read them. Edit/delete application is best-effort per device — a
device that never received the original cannot apply an edit to it, which is
the intended behavior for an end-to-end-encrypted system.
