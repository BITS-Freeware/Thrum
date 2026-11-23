# Decision 0017: Resource limits for logged-in users

Date: 2026-11-23
Status: accepted

## Context

The limits a server had for logged-in users were incomplete, and several
were a count followed by an insert in a separate statement. Invite
redemption never checked the member cap, and overlapping adds passed it
together. Nothing limited how fast a device sent or how much an account
could leave queued for a device that stays offline, and catch-up and the
export loaded the whole backlog at once. Logins created devices without
limit, with labels of any size, and every device multiplies fan-out.
Parallel uploads passed the daily quota together, parallel key-package
uploads passed the stock cap and left several last-resort packages, and on
PostgreSQL concurrent fetches fell back to the last-resort package while
one-time stock remained. The per-device fetch limit was shared by every
caller, so one user could spend it and stop everyone else from adding the
device. The web client never acknowledged a message it could not decode,
so junk from any member was replayed on every reconnect for a week.

## Choices

1. **Each limit is checked where it is spent, under a lock.** The handler
   opens a transaction whose first statement writes the row the limit
   belongs to (the conversation for membership, the account for queued
   bytes, devices and uploads, the device for key packages), counts, and
   inserts before it commits. PostgreSQL holds a row lock; SQLite holds its
   write lock. Requests that overlap are counted one after another.
2. **The member cap holds on every path that adds a member**: create, add
   and invite redemption. A member who redeems an invite again changes
   nothing and spends no use.
3. **Sends are limited per device and queued bytes per account.**
   `THRUM_MESSAGES_PER_DEVICE_PER_MINUTE` (default 120) limits sends.
   `THRUM_QUEUED_BYTES_PER_ACCOUNT` (default 100 MiB) caps the payload bytes
   of the account's messages that some device has not received yet; a send
   past it answers `429 queue_full`. The cap is on the sender, not the
   recipient, so a member cannot fill someone else's queue and block
   others from writing to them.
4. **Stored messages are read a page at a time.** Catch-up on connect sends
   the queue in pages of 100, ordered as before. The export streams its
   messages in pages after the rest of the document.
5. **Devices per account are capped**, at `THRUM_MAX_DEVICES_PER_ACCOUNT`
   (default 20) signed-in devices; login at the cap answers
   `409 too_many_devices`, and logging out or revoking a device makes room.
   Labels are at most 64 bytes.
6. **A device's key packages are handed out one fetch at a time.** Uploads
   and fetches of one device run under its row lock, so the stock cap holds,
   a new last-resort package replaces the old one, and the newest is served.
7. **Key-package fetches are limited per caller and device**
   (`THRUM_KEY_PACKAGE_FETCH_PER_CALLER_DEVICE_PER_MINUTE`, default 10), as
   well as per caller. The per-device limit for everyone together no longer
   refuses: past it, a fetch gets the last-resort package and the one-time
   stock is left alone.
8. **Clients acknowledge what they can never read.** The web client acks a
   payload with no envelope, an envelope whose data is not base64, and MLS
   bytes it cannot apply in a group it is in, as the CLI already did. Only a
   message for a group whose welcome has not arrived yet stays queued.

## Consequences

- A user who loses every one of 20 signed-in devices without logging out
  cannot log in until the operator revokes one or raises the cap. Clients
  should log out when they forget a session.
- A busy group can make a sender wait for `queue_full` while members are
  offline. The default is far above ordinary use; operators can raise it or
  set `0`.
- Past the per-device fetch ceiling, groups started in that minute share
  the device's last-resort package, which MLS allows and decision 0013
  already accepts when stock runs out.
- Membership changes, uploads and sends of one account are serialized at
  the database. They are short transactions.
