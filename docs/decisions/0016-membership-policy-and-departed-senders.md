# Decision 0016: Membership policy, export scope and departed senders

Date: 2026-11-23
Status: accepted

## Context

Decision 0007 says a block between any two members refuses a new group, but
the server checked only the caller against each other person, on create,
add and redeem alike. Removing a member left their multi-use invite
working, so they could walk back in. The export (decision 0010) listed
every stored message of the caller's conversations, including messages
sent before the caller joined and addressed only to others. Deleting an
account deleted every message it had sent, including commits that members
who were offline had not fetched yet, which left those members stuck in an
old epoch.

## Choices

1. **Blocks are checked across the whole group.** Creating a conversation
   fails with `403 blocked` when any two of its people block each other.
   Adding members, and redeeming an invite, fail the same way when a
   newcomer and anyone already in the group, or two newcomers, block each
   other.
2. **A removal sticks until the admin undoes it.** The server records each
   removal. Redeeming an invite to that conversation answers
   `403 removed`; the admin adding the person again lifts it.
3. **An export holds the caller's own traffic.** Messages the caller's
   devices sent, and messages queued for one of the caller's devices. Not
   what others were sent before the caller joined.
4. **Deletion keeps what others still need.** A deleted account's messages
   that some other member's device has not received yet stay until they are
   delivered or expire. They move to a placeholder sender (`~departed`,
   which nobody can log in as or add), and keep the deleted account's name
   for the frames that deliver them. Everything else the account sent goes.
5. **Clients accept a commit from an account that has left.** Decision 0014
   requires the committer to be a listed device. A commit from a leaf whose
   account is no longer a member at all (it left or was deleted after
   committing) is now accepted, because its adds, removals and updates are
   still held to the server's roster like anyone else's. A device of a
   member account must still be listed with its key.

Choice 3 replaces the export's `messages` entry in decision 0010; choice 4
replaces its deletion step 3. Choice 5 amends decision 0014 choice 1.

## Consequences

- A member who was offline while a deleted admin added someone still
  catches up. The person added then still needs a welcome from a member
  whose account exists: a welcome is checked strictly, because a leaf from
  outside the conversation is exactly what a forged group looks like.
- Removal records cascade with the conversation and with the account.
