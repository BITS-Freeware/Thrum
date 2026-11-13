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
  only enter a fetched package into a commit when its credential identity
  is the device that was asked for. *(mitigation: enforced in the add flow
  of both clients, M3; the check is unit-tested against a swapped package.)*
- Can try to guess passwords offline after a database leak: passwords are
  stored as Argon2id PHC strings with per-user salts, and unknown-name
  logins burn time on a dummy hash. *(mitigation: M1; hash format, verify
  and salting unit-tested in the accounts module.)*

### 2. Network attacker

On the path between client and server.

- Cannot read or tamper with traffic in a correct deployment: the server
  speaks plain HTTP, and the deployment guide requires a TLS-terminating
  reverse proxy for anything reachable from the internet. *(mitigation:
  documented in the self-hosting guide, M7. Limitation: TLS is an
  operational requirement, not enforced in code; a misconfigured operator
  can still serve plain HTTP on a public interface.)*
- Cannot crash clients or the server with malformed protocol input:
  envelope, frame, payload, attachment-name and MLS parsing are total —
  garbage becomes a typed error or a silent drop, and a device's MLS
  state survives arbitrary junk. *(mitigation and tests: fuzz layer of
  M7, decision 0011.)*
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
- Cannot crash other members by feeding them malformed MLS material
  (commits, welcomes, key packages): all of it is parsed behind the M7
  panic guard and fuzz layer, and a failed parse leaves the device's
  group state untouched. *(mitigation: decision 0011; tested there.)*

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
