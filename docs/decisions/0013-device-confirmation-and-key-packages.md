# Decision 0013: Device confirmation and key-package limits

Date: 2026-11-14
Status: accepted

## Context

Decision 0006 let any online member add a new device of an existing member
as soon as the server listed it, and it treated key-package fetch as an
unauthenticated, unlimited consume. Both match a server that is only a
relay. They do not match a server that might be curious or compromised:
it could list an extra device and drain one-time key packages.

## Choices

1. **Trust on first sight, confirm afterwards.** The first time an account
   appears in a conversation — the user named them, or they redeemed an
   invite — that account's devices at that moment are added (trust on first
   use). A device identity that appears later is pending until the user
   trusts it (`thrum trust --device username#id`, or Trust in the web
   client). A device the server no longer lists as active, and every device
   of an account that left, is removed on the next sync by a member who is
   still in the group. The server cannot remove an MLS leaf itself.
2. **Key packages require a session.** `GET /v1/devices/{id}/key-package`
   returns 401 without a bearer token. Fetches are rate-limited per caller
   and per target device. Each device may stock a capped number of one-time
   packages (default 100, `0` unlimited). One last-resort package stays in
   the directory and is served, without being deleted, when the one-time
   stock is empty. OpenMLS marks that package so a group can still be
   formed; clients replace it on the next top-up.
3. **The threat model says this and no more.** A malicious server can still
   substitute a device at the moment the user adds an account. Key
   transparency stays out of scope. Later devices are the part that is no
   longer silent.

Points 6 and 7 of decision 0006 are superseded by this record.

## Consequences

- Both clients share one roster planner (`plan_roster` in `thrum-core`,
  exported to the web client through wasm).
- Revoking a device stops delivery immediately. The leaf leaves the group
  when another member syncs, so a later message is not decryptable by the
  revoked device.
- Two members who sync a removal at the same moment can each try to commit.
  A failed removal is logged and the next sync tries again.
