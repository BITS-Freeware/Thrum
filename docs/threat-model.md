# Threat Model

What we protect, who we protect it from, and what version 0.1.0 actually
defends. Every adversary item ends in one of three ways: a test, a documented
mitigation, or a documented limitation. M7 re-checks every line against the
shipped code.

## Assets

- **Message plaintext.** The content of conversations, including replies,
  edits, deletions, read receipts and attachment contents.
- **Key material on devices.** MLS identity keys, leaf and epoch secrets,
  and the per-file keys of attachments.
- **Account credentials.** Passwords (stored only as Argon2id hashes) and
  session tokens.
- **Membership metadata.** Who talks to whom, group rosters, when.

## Adversaries

### 1. Curious server operator

Runs the server and reads everything it stores and relays.

- Cannot read message plaintext: payloads are MLS application messages and
  the server holds no group keys. *(mitigation: MLS by default, decision 0002;
  tested in M3 by scanning the server database for plaintext.)*
- Can see and correlate metadata: rosters, sizes, timing, IP addresses.
  *(documented limitation in 0.1.0; no cover traffic or padding is planned.)*
- Can deny, delay, drop or reorder messages. *(documented limitation; the
  client-visible effect is bounded by delivery acknowledgements and the
  offline queue, but a malicious server can always censor.)*
- Can serve a stale or wrong key package when a client fetches one, which
  could steer a group commit toward an attacker-controlled device. Clients
  must verify that a fetched key package's credential identity matches the
  account it intends to add. *(mitigation: enforced in the client add flow,
  M3; test added there.)*
- Can try to guess passwords offline after a database leak: passwords are
  hashed with Argon2id. *(mitigation: M1, tested there.)*

### 2. Network attacker

On the path between client and server.

- Cannot read or tamper with traffic: HTTPS is required for all non-local
  clients, and WebSocket runs over TLS. *(mitigation: deployment guide, M7;
  local plain HTTP is allowed only on loopback for development.)*
- Cannot break end-to-end encryption without breaking TLS *and* the server's
  MLS group keys, which the server does not hold.

### 3. Stolen device

Has the device's local storage, not the server.

- Sees whatever keys and message state the client keeps locally. 0.1.0 keeps
  client state on disk with standard file permissions and no extra at-rest
  encryption. *(documented limitation.)*
- Can be cut off from the future: the user (or any other device of theirs)
  revokes the device, it is removed from the MLS groups it belonged to, and
  later epochs are unreadable to it. *(mitigation: device revocation, M1 and
  M3; tested there.)*
- Sessions are bearer tokens; revoking a device revokes its token.
  *(mitigation: M1.)*

### 4. Malicious group member

A legitimate member who misbehaves.

- Reads everything sent while they are a member. Unavoidable and by design:
  a group member is a recipient.
- After being removed, cannot read later messages: MLS removes their leaf and
  later epochs are encrypted to the remaining members only. *(mitigation: MLS
  removal; tested in M3.)*
- Cannot forge messages from another member: MLS authenticates each
  application message to its sender's leaf. *(mitigation: MLS guarantees;
  exercised by M3 tests.)*
- Cannot read messages sent before they were added: earlier epochs do not
  include them. *(mitigation: MLS epoch structure; tested in M3.)*

## Out of scope in 0.1.0

- Anonymity from the server operator, or hiding who talks to whom.
- Spam resistance beyond rate limiting of registration and login.
- Post-quantum key agreement beyond what OpenMLS itself provides.
- Key transparency: verifying that a fetched key package really belongs to
  the claimed account beyond the credential check above.
- Encrypted-at-rest client storage, or secure erasure of local key material.
- Protection against a compromised endpoint reading the screen or memory.

Each out-of-scope item is a documented limitation, not an oversight; reopen
any of them through a decision record.
