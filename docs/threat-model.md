# Threat Model

What we protect, who we protect it from, and what version 0.2.1 actually
defends. Every adversary item ends in one of three ways: a test, a documented
mitigation, or a documented limitation.

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
  the CLI end-to-end tests search the SQLite database and its write-ahead
  log for the plaintext they sent.)*
- Can read the keys and messages of web-client users by serving a modified
  web client. The web client is served from the server's own origin, so
  the browser runs whatever the operator sends, and the Content-Security-
  Policy cannot help against that origin. The command-line client does not
  depend on it. *(documented limitation. CI records the SHA-256 of the
  built web client for every commit and release, so a build can be
  compared with the published one, but browsers do not check it. A client
  delivered from elsewhere would close this.)*
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
  Fetching a package requires a logged-in caller, is rate-limited per caller
  and per caller and target device, and cannot drain the directory dry:
  one-time stock is capped, one last-resort package remains, and past a
  device's overall ceiling fetches get the last-resort package.
  *(mitigation: decisions 0013 and 0017; tested in the key-package and
  resource-limit suites.)* The last-resort package's init key is reused for
  every group joined through it until the device tops up again, so it
  protects several Welcomes. That is the trade-off MLS accepts for
  last-resort packages. *(documented limitation.)*
- Cannot silently add a device that appears after a client has already seen
  that account in any conversation. Trust on first use applies to the
  devices present the first time the account appears: the user named them,
  or they redeemed an invite. A later device stays pending until the user
  trusts it, on both clients. Clients pin each device's signature key when
  they first see it, so a known device offered under another key, in a new
  conversation or after it was revoked and listed again, also waits for the
  user. A malicious server can still substitute a device at an account's
  first sight, including by inserting an account the user has not seen yet.
  *(mitigation: decisions 0013 and 0015; tested by the CLI device-security
  tests. Key transparency remains out of scope.)*
- Cannot hand a deleted account's name to someone else: deleted names are
  retired. *(mitigation: decision 0015; tested in the account tests.)*
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
- Registration and login limits follow the real client address only when the
  immediate peer is listed in `THRUM_TRUSTED_PROXIES`; otherwise
  `X-Forwarded-For` and `Forwarded` are ignored. IPv6 addresses are limited
  per /64. Login also has a per-account limit and an exponential backoff
  after repeated failures, both kept per client address, so a stranger's
  guesses lock out the stranger rather than the owner. An hourly ceiling on
  failures per account, from any address, bounds guessing spread over many
  addresses; reaching it does refuse the owner too, for the rest of that
  hour. Wrong passwords on account deletion count toward the same limits,
  and login bodies are capped at 16 KiB, so failed logins cannot grow
  server memory. *(mitigation: tested in the rate-limit suite.)*
- Whether a username exists is public to anyone: registration answers
  `username_taken`, and any signed-in user can list a name's devices.
  People add each other by name, so this is by design. The dummy hash on
  unknown-name logins keeps login timing from revealing more, nothing
  else. *(documented limitation.)*
- Administrative routes answer a non-member as they would for a
  conversation that does not exist, so an id learned elsewhere cannot be
  confirmed. *(mitigation: tested in the member suite.)*
- A logged-in user's load on the server is bounded by operator settings:
  members per conversation on every path that adds one, sends per device
  per minute, queued bytes per account, signed-in devices per account,
  uploaded bytes per day, and key-package stock per device. Each check
  holds when requests overlap. Catch-up and export read stored messages a
  page at a time. *(mitigation: decision 0017; tested in the
  resource-limit suite on both databases.)*

### 3. Stolen device

Has the device's local storage, not the server.

- Sees whatever keys and message state the client keeps locally. The CLI
  writes its session, keys, history, attachments and exports readable by
  the owner only, so other accounts on a shared machine cannot read them,
  and deleting the account removes them; there is no extra at-rest
  encryption. *(mitigation for other local accounts: tested in the CLI
  file-mode test; at-rest encryption is a documented limitation.)*
- Can be cut off from the future: revoking the device kills its session, and
  the next sync by a remaining member removes its MLS leaf, so a message sent
  after that removal is unreadable to the revoked device. The server itself
  cannot remove the leaf. *(mitigation: decision 0013; tested by the CLI
  device-security test, which loads the revoked device's keys and checks that
  a later ciphertext is not an application message.)*
- Sessions are bearer tokens; revoking a device revokes its token, and
  its open WebSocket is closed. Tokens do not expire: one lives until
  logout, revocation or account deletion. The server keeps only a hash of
  a live token and drops it when the session ends. Invite tokens travel in
  request bodies, not paths, so proxy access logs do not record them.
  *(mitigation: M1, tested in the account and WebSocket suites; no expiry
  is a documented limitation.)*

### 4. Malicious group member

A legitimate member who misbehaves.

- Reads everything sent while they are a member. Unavoidable and by design:
  a group member is a recipient.
- After being removed, cannot read later messages: MLS removes their leaf and
  later epochs are encrypted to the remaining members only. *(mitigation: MLS
  removal; tested in M3.)*
- Cannot forge messages from another member: MLS authenticates each
  application message to its sender's leaf, and clients show the sender from
  that leaf's credential, not from the relay's label. A message from a leaf
  that has since been removed is ignored. *(mitigation: MLS guarantees and
  decision 0014. The CLI membership test relabels one member's ciphertext
  as another's and checks that it is dropped; forging another leaf's
  signature is left to OpenMLS and not tested here.)*
- Cannot change who is in a group beyond what the server lists: clients merge
  a commit, or join from a welcome, only when every leaf it adds, keeps or
  names is a device the server lists for a member account, with the
  signature key the server bound to it, and a welcome only for the
  conversation it arrived in. A refused commit or welcome leaves the device
  unchanged. *(mitigation: decision 0014; tested in the core and CLI
  membership tests.)*
- Cannot read messages sent before they were added: earlier epochs do not
  include them. *(mitigation: MLS epoch structure; tested in M3.)*
- Cannot crash other members by feeding them malformed MLS material
  (commits, welcomes, key packages): all of it is parsed behind the M7
  panic guard and fuzz layer, and a failed parse leaves the device's
  group state untouched. *(mitigation: decision 0011; tested there.)*
- Cannot control another member's terminal or requests through what they
  write: the CLI escapes control characters and direction overrides in
  everything it prints, ignores attachments whose blob id is malformed,
  and puts every id into a request path as one encoded segment.
  *(mitigation: tested in the CLI peer-text tests.)*

## Out of scope in 0.2.1

- Anonymity from the server operator, or hiding who talks to whom.
- Spam resistance beyond rate limiting of registration and login.
- Post-quantum key agreement beyond what OpenMLS itself provides.
- Key transparency: verifying that a fetched key package really belongs to
  the claimed account beyond the credential check above.
- Encrypted-at-rest client storage, or secure erasure of local key material.
- Protection against a compromised endpoint reading the screen or memory.

Each out-of-scope item is a documented limitation, not an oversight; reopen
any of them through a decision record.
