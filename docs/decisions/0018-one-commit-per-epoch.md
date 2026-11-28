# Decision 0018: One commit per epoch

Date: 2026-11-28
Status: accepted

## Context

A device that changed a group (an add, a removal, or `sync` doing either)
merged its own commit at once and then sent it. The server relayed every
payload in arrival order and never handed a device its own message back.
When two devices committed in the same epoch, the server relayed both.
Every other device applied whichever came first and ignored the other as
an old-epoch message. The second committer had already merged its own
commit, so it sat alone in an epoch nobody else had and ignored every
later commit. A commit whose send failed after the merge stranded the
committer the same way, and an add's welcome could bring a new member
into a group state nobody else had (#46).

## Choices

1. **A client says which epoch a commit is for.** It sends a commit with
   `?commit_epoch=N`, the epoch its group was in when it built the
   commit. Other payloads are sent as before.
2. **The server takes one commit per epoch.** It records the next epoch it
   expects for each conversation. A commit for an epoch another commit
   already took is refused with `409 stale_epoch`. A commit for a later
   epoch than expected is taken, because commits sent without an epoch
   (older clients) advance the group unseen.
3. **A retry of the last commit taken is answered, not refused.** The
   server keeps the hash and id of the last commit it took; the same
   bytes sent again get that id and are not queued twice. A sender whose
   reply was lost can safely send again.
4. **A commit goes back to its sender too**, queued in order with
   everything else. A device that never heard the reply still learns its
   commit was taken before any commit built on top of it.
5. **A device merges its own commit only once the server took it.** It
   stages the commit, sends it, and merges it on the server's reply or
   when the commit comes back. If another commit took the epoch, it
   applies that one and drops its own; the next `sync` makes its change
   again. A commit still pending when a request kept failing is sent
   again before the device's next change.
6. **A welcome goes out only after its commit was taken**, so nobody joins
   a group state that lost.
7. **A device that is behind accepts a welcome for a group it holds.** The
   conversation list shows the epoch the server knows. Since the server
   removal comes first (#45), a removed member's device never gets the
   commit that removes it and keeps its old group. Added again, its group
   is behind the server's epoch, so it drops that group and joins with the
   welcome. A device that is up to date still refuses a welcome for a
   group it is in.

## Consequences

- The server learns which payloads are commits and the epoch numbers of a
  conversation. It already saw membership changes and could tell commits
  by size and timing; it still cannot read them.
- A member can stop another member's commit only by committing first,
  which any member could always do.
- The web client still sends commits without an epoch and merges them at
  once, so its own commits can still lose a race. It should follow.
