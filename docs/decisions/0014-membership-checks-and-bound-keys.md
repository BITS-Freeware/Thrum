# Decision 0014: Membership checks and bound signature keys

Date: 2026-11-23
Status: accepted

## Context

OpenMLS checks that a commit or welcome is well formed and signed by a
member of the group. It does not know who should be in a Thrum
conversation. Before this record, every client merged any valid commit,
joined any valid welcome, and showed the sender the relay named. That left
group membership, and the name on a message, in the hands of any one member
or of the relay.

## Choices

1. **The server's member list is the authority for who may be in a group.**
   Before a client merges a commit, it checks the commit against the
   server's current roster for that conversation (`check_commit` in
   `thrum-core::membership`):
   - the committer must be a listed device, unless the commit only removes
     the committer itself (decision 0016 also accepts a committer whose
     account has left the conversation);
   - every added leaf must be a listed device of a member account and carry
     that device's bound signature key;
   - a removed leaf must be the committer, malformed, or no longer listed;
   - an update keeps its identity and its bound key;
   - no identity may appear twice afterwards.
   A refused commit leaves the device state exactly as it was. The client
   fetches the roster again and retries once, because the commit may follow
   a membership change it has not seen yet.
2. **A welcome only counts for the conversation it arrived in.** The group id
   inside it must equal that conversation id, an active local group is never
   replaced, and every other member it names must pass the same roster
   check (`check_welcome`).
3. **The server binds one signature key to each device.** The first key
   package a device uploads fixes its key; a package with another key is
   refused with 409, and a package must claim the uploader's own
   `username#device-id`. The device list returns the bound keys, so clients
   can tell a listed device from a leaf that only claims its name.
4. **The sender of a message is the MLS credential that signed it.** The
   relay's sender label is a hint. A frame whose label disagrees with the
   credential is dropped. A message sent by a leaf that has since been
   removed, or whose slot another device now holds, is ignored.
5. **Failure leaves no trace.** Processing a commit or welcome works on a
   snapshot of the device's MLS storage and restores it on any refusal or
   error. A failed welcome therefore no longer uses up the one-time key
   package it was addressed to. A commit that removes this device deletes
   the local group, so a later welcome can bring the device back.

## Consequences

- Both clients run the same checks: the CLI directly, the web client through
  the wasm bindings, which take the roster as JSON.
- A client needs a fresh roster to accept legitimate changes. The cache plus
  one refresh keeps that to one extra request in the common case.
- A member can still admit a device of another account under decision 0013's
  trust-on-first-use rule; that device must be one the server lists, with
  the key the server bound to it.
- A server that lies about its roster can still steer membership. Key
  transparency stays out of scope, as in decision 0013.
