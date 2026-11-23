# Decision 0015: One writer per device, and pinned device keys

Date: 2026-11-23
Status: accepted

## Context

A device's MLS state is a ratchet: each copy that processes or sends moves
it on. Both clients keep that state in local storage and write it back
whole. Nothing stopped two copies of one device from running at once (two
CLI commands, two browser tabs), and the server kept one socket per device
but forgot the newer one when the older one closed. Copies drifted apart,
and a copy that missed a removal went on encrypting to the removed member.

Device trust was also keyed by the `username#device-id` string alone, and
trust on first use was per conversation. The server could hand out a known
device under a key of its own whenever a new conversation started, or when a
revoked device was listed again. A deleted account's name could be
registered at once by someone else.

## Choices

1. **One process or tab runs a device.** The CLI takes an exclusive lock on
   `<state-file>.lock` for the life of each command; a second command on the
   same device waits for it, for up to two minutes. The web client holds a
   Web Lock (`thrum-device`) for the life of the tab; another tab shows a
   notice and takes over when the first closes.
2. **State files are replaced, not rewritten.** The CLI writes the MLS state
   and the message store to a temporary file, syncs it, and renames it over
   the old one, readable by the owner only. A store that does not parse is
   an error, never an empty start, because the store holds trust decisions.
3. **The server keeps one live socket per device and closes the rest.** A
   new connection replaces and closes the previous one; a connection that
   ends only unregisters itself. Revoking a device, logging out and deleting
   the account close its socket.
4. **Clients pin each device's signature key.** The first key a client sees
   for a device identity (in a key package it adds, a welcome it accepts or
   a commit it merges) is pinned. Rosters are checked against the pinned key,
   whatever the server reports. A known identity with another key is not
   added and waits for the user, like a new device; `thrum trust --device`
   (or Trust in the web client) accepts the new key.
5. **Trust on first use is per account.** An account introduced in any
   conversation is known everywhere, so its later devices wait for the user
   in a new conversation too. A device removed because the server stopped
   listing it is no longer trusted; if it is listed again, it waits.
6. **Deleted names are retired.** Registration refuses a name that belonged
   to a deleted account, with the same answer as a name in use.
7. **The web client applies what it has received before it sends**, so it
   does not encrypt in an epoch that a queued removal has already ended.

## Consequences

- `thrum send` while `thrum recv` runs on the same device waits until `recv`
  ends. Running both at once was never safe.
- Each login is a new device id, so an honest device never changes its key
  under the same identity. A changed key is always worth a prompt.
- Retired names are never freed. An operator who needs to release one can
  delete its row from `retired_usernames`.
- The window decision 0013 leaves open is now the first sight of an
  account, not of each conversation.
