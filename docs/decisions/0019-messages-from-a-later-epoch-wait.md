# Decision 0019: A message from a later epoch waits for the commits before it

Date: 2026-11-28
Status: accepted

## Context

A device processes each queued payload as it arrives and acknowledges it
unless processing failed for a reason that a retry could fix (#40). A
commit or message from a later epoch than the device's group fails MLS
validation, so it came back as `Incoming::Ignored`, was acknowledged, and
was gone. When frames arrive out of order on one connection, a device
that gets the commit for epoch N+1 before the commit for epoch N ignores
the later one, applies the earlier one, and then stays one epoch behind
the others for good: it cannot read anything sent after, and with
decision 0018 every commit it builds is refused as stale (#49).

Frames can arrive out of order. The threat model lets the server
reorder, and an honest server could too: the send route waited on a
database sweep between storing a message and pushing it to live
connections, so a later message could be pushed first, even one sent by
a device that had already read the first through catch-up.

## Choices

1. **The core reports an early message.** `process_incoming` returns
   `Incoming::Early` for a message of the group's own id whose epoch is
   later than the group's. Nothing is merged or changed.
2. **The CLI leaves a live early message queued.** For a frame that came
   after the server's `caught_up` marker, `interpret` returns an error that
   starts with `chat::EARLY`, so the message is not acknowledged and the
   connection stops, as for any message that cannot be applied yet.
   `catch_up` and `recv` connect again; catch-up sends the queue in the
   server's order, so the commit the message needs comes first.
3. **In catch-up an early message is dropped at once, and live ones after
   a few tries.** Catch-up comes in the order the server stored the
   messages, so a message still early there needs a commit that is never
   coming: one the device refused, or one sent while it was removed (a
   member added back gets the add's commit before its welcome). One
   process leaves the same live message queued at most `EARLY_TRIES` (3)
   times, so a server that keeps reordering cannot stop every connection
   at the same place.
4. **The web client leaves it queued too.** It does not acknowledge an
   early message, as it already does for a message that comes before its
   group's welcome; the next connection offers it again.
5. **The server pushes live frames right after storing.** The send route
   broadcasts before its opportunistic sweep of expired messages, with no
   wait between the stored row and the push, so live frames go out in the
   order messages were stored.

## Consequences

- A reordered commit or message costs a reconnect instead of the
  device's place in the group.
- A device that is behind for good, because it refused a commit, still
  drops later messages. Recovering such a device is the membership
  question in #45.
- `Incoming` has a new variant; code that matches it exhaustively must
  handle `Early`.
