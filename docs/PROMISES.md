# Promises

Every claim Thrum makes about its behavior, in one numbered list. Each
promise names where it is made, how it is checked today, and what kind of
fault could break it. The point is that no document claims more than the
tests show. When a promise has no test yet, it says so.

This list was taken at `v0.2.0` (`18c997d`). Line numbers refer to that
commit.

## How to read an entry

- **Source**: the file and line that make the claim.
- **Checked by**: the test that holds the claim today, by crate and name.
  "Gap" means no test checks it yet; "Partial" means a test checks part of
  it, and the entry says which part is missing.
- **Could break under**: the faults a test campaign should inject against
  this promise.

Promises are grouped by subject. A promise keeps its number for good; if it
is withdrawn, the entry stays with the reason.

## Confidentiality of content

### P1. The server never holds message plaintext

- **Source**: README.md:9; docs/self-hosting.md:3-4 and 200-203;
  docs/threat-model.md:23-25; decision 0002; decision 0006 (Consequences).
- **Checked by**: Partial. `thrum-cli` e2e tests
  `two_cli_clients_exchange_encrypted_messages`,
  `added_member_cannot_read_earlier_messages`,
  `group_names_and_invite_links_flow` and
  `edits_deletes_and_read_receipts_flow` search the server's SQLite
  database for the plaintext they sent. At `v0.2.0` this search read only
  the main database file. The server runs SQLite in WAL mode, and while the
  test runs that file holds only its 4 KiB header page; every row is in the
  `-wal` file. A probe in `two_cli_clients_exchange_encrypted_messages`
  confirmed that the main file contained neither the username `alice` nor
  the conversation id, so the search would have passed whatever the server
  stored. The search now reads the main file and the write-ahead log, and
  fails unless it finds a value the server is known to store
  (`Server::stored_bytes` in `crates/cli/tests/support`). It passes. It
  still does not cover PostgreSQL, the server's log output, or the blob
  directory (blobs are covered by P3).
- **Could break under**: a client bug that sends an application message
  outside MLS; a new payload kind added to the envelope without
  encryption; debug logging of request bodies; a crash or restart between
  the encrypt and send steps.

### P2. Edits, deletions, replies and read receipts are inside MLS

- **Source**: decision 0007 choice 4; docs/self-hosting.md:202-203.
- **Checked by**: Partial. `edits_deletes_and_read_receipts_flow` searches
  for edited and reply text in the server database (the same scan as P1).
  Nothing checks that a read receipt or a delete leaves no plaintext
  marker.
- **Could break under**: the same faults as P1.

### P3. The server never holds attachment plaintext

- **Source**: decision 0008 choices 1-2 and Consequences;
  docs/self-hosting.md:4 and 187.
- **Checked by**: `thrum-cli` e2e `attachments_flow_end_to_end` searches
  the server database and every stored blob for the original file bytes.
  `thrum-core` `attachment::tests::roundtrip`, `keys_are_fresh`.
- **Could break under**: an upload path that skips `encrypt_file`; reuse of
  a key and nonce across files; image recompression applied after
  encryption instead of before.

### P4. An attachment opens only when it is intact and has the blob id it was sent with

- **Source**: decision 0008 choice 1 (blob id bound as AAD); decision 0011
  ("received attachments are saved verbatim after the sha-256 check").
- **Checked by**: `thrum-core` `attachment::tests::wrong_key_fails`,
  `wrong_blob_id_fails`, `tampered_bytes_fail`,
  `truncated_ciphertext_fails`. The CLI's SHA-256 check after decryption is
  exercised only on the success path in `attachments_flow_end_to_end`. Gap:
  the web client's check (`web/src/session.ts`) has no test, and no test
  feeds either client a blob whose SHA-256 does not match.
- **Could break under**: a server that serves different bytes for an id;
  a sender that lies about the SHA-256 or size; truncated downloads.

## Group membership and MLS

### P5. A removed member cannot read messages sent after removal

- **Source**: docs/threat-model.md:98-100; decision 0006 choice 6.
- **Checked by**: `thrum-core` `mls::tests::removed_member_cannot_read_later_messages`;
  `thrum-cli` e2e `removed_member_stops_receiving_messages`.
- **Could break under**: the removing commit lost, delayed or reordered;
  two members committing at once; the server withholding the commit from
  some members; the removed member's client restoring old state.

### P6. A new member cannot read messages sent before they were added

- **Source**: docs/threat-model.md:104-105; decision 0006 (Consequences).
- **Checked by**: `thrum-core` `mls::tests::added_member_cannot_read_earlier_messages`;
  `thrum-cli` e2e `added_member_cannot_read_earlier_messages`.
- **Could break under**: a server that replays old ciphertext to the new
  member; a welcome processed out of order.

### P7. No member can forge a message from another member

- **Source**: docs/threat-model.md:101-103 ("exercised by M3 tests").
- **Checked by**: Gap. No test builds a message claiming another member's
  leaf, or alters the sender field of an application message, and checks
  that receivers refuse it. The claim rests on OpenMLS. The threat model's
  "exercised by M3 tests" is stronger than the tests.
- **Could break under**: a client that shows a sender name taken from the
  decrypted payload instead of the MLS sender leaf; edit or delete
  applied by matching a username in the payload.

### P8. Edits and deletes apply only to the sender's own messages

- **Source**: decision 0007 choice 4.
- **Checked by**: `thrum-cli` `store::tests::edits_and_deletes_only_by_the_sender`;
  `read_receipts_ignore_the_reader_and_deduplicate`. Gap: the web client's
  store has no equivalent test.
- **Could break under**: the sender identity taken from the payload rather
  than MLS (see P7); username case differences.

### P9. A malformed MLS message, welcome or key package cannot crash a device or damage its group state

- **Source**: docs/threat-model.md:63-68 and 106-110; decision 0011.
- **Checked by**: `thrum-core` fuzz tests
  `fuzz_mls_process_incoming_never_panics_and_state_survives` and
  `fuzz_mls_welcome_and_key_package_never_panic`; the libFuzzer target
  `mls_process`.
- **Could break under**: a panic in a step after parsing (decision 0011
  leaves those loud by design); a new OpenMLS version that moves where it
  asserts; a well-formed but semantically hostile commit.

### P10. Every device is its own MLS member, and a group id is its conversation id

- **Source**: decision 0006 choices 1-2.
- **Checked by**: `thrum-core` `mls::tests::identity_split`,
  `roundtrip_between_two_members`; `thrum-cli` e2e
  `a_second_device_joins_existing_conversations`.
- **Could break under**: a username containing `#`; case differences
  between the username in the credential and the one the server lists.

## Devices and key packages

### P11. A device that appears after an account was first seen joins only after its owner trusts it

- **Source**: docs/threat-model.md:40-47; decision 0013 choice 1;
  CHANGELOG.md (0.2.0, Security).
- **Checked by**: `thrum-core` `roster::tests::later_devices_wait_and_revoked_devices_are_removed`,
  `a_trusted_device_is_added_on_a_later_sync`,
  `a_replacement_device_stays_pending_after_the_leaf_is_gone`;
  `thrum-cli` `device_security::tests::a_new_device_is_not_added_until_it_is_trusted`.
  Gap: the web client's path through the shared planner has no test of its
  own.
- **Could break under**: a server that lists a device at the account's
  first sight (documented limitation); a client that loses its record of
  which accounts it has seen; two members syncing at once.

### P12. Devices present at an account's first sight are trusted then

- **Source**: docs/threat-model.md:40-44; decision 0013 choice 1.
- **Checked by**: `thrum-core` `roster::tests::the_first_sight_of_an_account_admits_its_current_devices`,
  `an_explicit_add_admits_current_devices_only`.
- **Could break under**: this is the documented limitation side of P11: a
  malicious server can substitute a device at first sight. A campaign
  should confirm that this is the only window.

### P13. A revoked device cannot read messages sent after its leaf is removed

- **Source**: docs/threat-model.md:83-88; decision 0013 (Consequences);
  CHANGELOG.md (0.2.0, Security).
- **Checked by**: `thrum-cli` `device_security::tests::a_revoked_device_cannot_read_messages_sent_afterwards`;
  `thrum-core` `roster::tests::later_devices_wait_and_revoked_devices_are_removed`.
- **Could break under**: no remaining member syncing for a long time (the
  window between revocation and removal is open until someone syncs); two
  members removing at once; a failed removal that is never retried.

### P14. A client adds a fetched key package only when its credential is the device that was asked for

- **Source**: docs/threat-model.md:31-35 ("enforced in the add flow of both
  clients").
- **Checked by**: Partial. `thrum-cli` `chat::tests::a_mismatched_key_package_is_rejected`
  covers the CLI. The web client's check (`web/src/session.ts`) has no
  test.
- **Could break under**: a server that serves another device's package; an
  identity that differs only in letter case.

### P15. Key-package fetch needs a session, is rate-limited, is capped, and keeps one last-resort package

- **Source**: docs/threat-model.md:36-39; decision 0013 choice 2;
  docs/self-hosting.md:108-110.
- **Checked by**: `thrum-server` `keypackages` tests
  `anonymous_key_package_fetch_is_rejected`,
  `key_packages_need_auth_and_respect_limits`,
  `key_package_stock_is_capped_and_last_resort_survives_drain`,
  `key_package_fetch_is_rate_limited_per_caller_and_device`,
  `revoked_device_key_packages_are_unfetchable`; `thrum-core`
  `mls::tests::last_resort_key_package_joins_a_second_group`.
- **Could break under**: concurrent fetches racing the last one-time
  package; uploads racing the cap; PostgreSQL ordering differences.

## Accounts and sessions

### P16. Passwords are stored only as Argon2id hashes with per-user salts

- **Source**: docs/threat-model.md:48-52; CHANGELOG.md (0.1.0).
- **Checked by**: `thrum-server` `accounts::tests::password_hashes_are_argon2id`.
- **Could break under**: a code path that logs the login body.

### P17. A login for an unknown name costs the same time as a wrong password

- **Source**: docs/threat-model.md:51-52 ("unknown-name logins burn time on
  a dummy hash").
- **Checked by**: Partial. `accounts::tests::dummy_hash_is_a_real_argon2id_string`
  checks that the dummy hash is valid. Nothing measures that both paths
  take similar time, or that the per-account limit and backoff do not
  reveal which names exist.
- **Could break under**: early returns before hashing; different error
  bodies or status codes for the two cases; the per-account backoff
  applied only to existing accounts.

### P18. Session tokens are stored hashed, and revoking a device kills its token at once

- **Source**: docs/threat-model.md:89-90; decision 0007 choice 2;
  `crates/server/src/auth.rs:46`.
- **Checked by**: `thrum-server` `devices::devices_list_and_revoke`,
  `accounts::register_login_logout_roundtrip`. Gap: nothing checks that an
  open WebSocket of a revoked device is closed.
- **Could break under**: a WebSocket opened before revocation that stays
  open; a revocation racing a login.

### P19. Login and registration limits follow the real client address only behind a trusted proxy, and login also has a per-account limit with backoff

- **Source**: docs/threat-model.md:70-74; docs/self-hosting.md:101-107.
- **Checked by**: `thrum-server` `ratelimit` tests and `client_ip` unit
  tests.
- **Could break under**: a proxy list that includes the client's own
  network; IPv6 and IPv4-mapped addresses; limits held in memory and lost
  on restart.

## Server authorization

### P20. Only members can send to or read from a conversation

- **Source**: decision 0006 (the server "enforces fan-out"); implied by
  docs/self-hosting.md:200-201.
- **Checked by**: Partial. `thrum-server`
  `conversations::sending_requires_membership_and_a_real_conversation`.
  Gap: no test walks every route as a second account to check that it
  cannot read, list, rename, invite to, or remove from a conversation it is
  not in, or fetch another account's export, devices or key-package count.
- **Could break under**: a route added without the membership check;
  a member removed while their WebSocket is open; catch-up delivery after
  removal.

### P21. Only the admin can add, remove, rename, and mint or revoke invites

- **Source**: decision 0007 choice 1.
- **Checked by**: `thrum-server` `groups::rename_is_admin_only`,
  `members::only_the_admin_can_add_members`. Gap: removal and invite
  revocation by a non-admin are not tested.
- **Could break under**: ownership transfer on account deletion (decision
  0010) leaving the wrong admin or none.

### P22. Invites expire and stop at their use limit, and revoking an unknown token looks the same as revoking a real one

- **Source**: decision 0007 choice 2; docs/self-hosting.md:114-116;
  CHANGELOG.md (0.2.0, Added).
- **Checked by**: `thrum-server` `groups::invite_roundtrip`,
  `invites_expire_and_honor_a_use_limit`. Gap: concurrent redemptions of a
  one-use invite are not tested.
- **Could break under**: two redemptions at the same moment; clock
  changes.

### P23. Blocking stops new shared conversations and invites in both directions

- **Source**: decision 0007 choice 3.
- **Checked by**: `thrum-server` `groups::blocks_prevent_new_shared_conversations`
  covers creating a conversation from either side of a block, and the
  blocked user redeeming the blocker's invite; `thrum-cli` e2e
  `blocking_stops_new_shared_conversations`. Gap: the blocker redeeming the
  blocked user's invite, and adding a blocked user to an existing
  conversation, are not tested.

### P24. A conversation never exceeds the member cap

- **Source**: docs/self-hosting.md:111; CHANGELOG.md (0.2.0, Added).
- **Checked by**: `thrum-server` `groups::group_member_cap_is_enforced`
  covers creation and direct adds.
- **Could break under**: every path that adds a member; two joins at the
  same moment.

### P25. A blob can be fetched by any logged-in device that knows its id, and by nobody else

- **Source**: decision 0008 choice 3.
- **Checked by**: `thrum-server` `blobs::rejects_unauthenticated`,
  `upload_and_fetch_roundtrip`, `rejects_bad_ids`,
  `fetch_unknown_blob_is_not_found`.
- **Could break under**: predictable ids; ids exposed in logs or error
  bodies.

## Delivery

### P26. No acknowledged message is lost, and each device gets each message once

- **Source**: CHANGELOG.md (0.1.0: "an offline queue that replays
  undelivered messages, and delivery acknowledgements");
  docs/threat-model.md:28-30.
- **Checked by**: Partial. `thrum-server` `ws::live_delivery_and_ack_stop_queueing`,
  `catch_up_replays_undelivered_messages`,
  `sender_is_not_delivered_own_message`. Gap: no test drops, duplicates or
  reorders frames, or restarts the server between send and ack.
- **Could break under**: a crash between storing and fan-out; an ack for a
  message the device never processed; reconnect during catch-up.

### P27. Undelivered messages are deleted after seven days, and delivered messages are not kept

- **Source**: docs/self-hosting.md:122-124; `crates/server/src/messages.rs`
  (`cleanup_expired`).
- **Checked by**: Partial. `thrum-server` `ws::expired_undelivered_messages_are_dropped`.
  Gap: nothing checks that a delivered message's row is gone after the next
  sweep.

### P28. Clients drop MLS messages they cannot process instead of failing

- **Source**: decision 0006 choice 8.
- **Checked by**: Partial, through the fuzz tests in P9. Gap: no test
  delivers a valid message from an old epoch or a duplicate.

## Storage and operations

### P29. SQLite and PostgreSQL behave the same

- **Source**: README.md:11; docs/self-hosting.md:135; decision 0009.
- **Checked by**: the whole `thrum-server` suite runs on both in CI. Gap:
  the CLI e2e tests run only on SQLite.
- **Could break under**: ordering ties; case-insensitive usernames;
  transaction isolation differences under concurrency.

### P30. Migrations run automatically, are forward-only, and upgrade earlier databases

- **Source**: docs/self-hosting.md:193-194; decision 0009 (Consequences).
- **Checked by**: Gap. No test opens a 0.1.0 database with 0.2.0.
- **Could break under**: an interrupted migration; a migration that rewrites
  data.

### P31. Export holds everything the server knows about the account, and payloads stay ciphertext

- **Source**: decision 0010 (Export).
- **Checked by**: Partial. `thrum-server` `account::export_empty_account`,
  `export_requires_auth`. Gap: no test exports an account with devices,
  conversations, messages and blobs and compares it with the database.
- **Could break under**: a table added later and missed by the export.

### P32. Account deletion removes everything listed in decision 0010 and needs the password

- **Source**: decision 0010 (Deletion).
- **Checked by**: `thrum-server` `account::delete_account_wrong_password`,
  `delete_account_removes_everything`,
  `delete_account_transfers_ownership`.
- **Could break under**: deletion racing a send or an upload; blob store
  errors part way through; PostgreSQL cascade differences.

### P33. Blobs expire after the retention period, and uploads respect size and daily quota

- **Source**: decision 0008 choices 4 and 6; docs/self-hosting.md:91-93.
- **Checked by**: `thrum-server` `blobs::sweeper_removes_expired_blobs`,
  `rejects_oversized_and_empty_blobs`, `enforces_daily_quota`,
  `upload_reuses_are_conflicts`.
- **Could break under**: concurrent uploads at the quota boundary; the day
  rolling over during an upload.

## Deployment and web client

### P34. Out of the box the server listens on loopback only and warns otherwise

- **Source**: docs/self-hosting.md:36-43 and 88; CHANGELOG.md (0.2.0,
  Security).
- **Checked by**: Gap. The warning in `crates/server/src/main.rs` has no
  test.

### P35. The container runs as an unprivileged user and Compose publishes on loopback only

- **Source**: docs/self-hosting.md:24-34.
- **Checked by**: Gap. Not tested in CI.

### P36. The web client and API send a strict Content-Security-Policy and framing, sniffing and referrer protection

- **Source**: docs/self-hosting.md:118-120; CHANGELOG.md (0.2.0, Security).
- **Checked by**: `thrum-server` `health::healthz_answers_ok` checks the API
  response headers. Gap: no test checks the headers on the static files, or
  runs the web client in a browser and treats a CSP violation as a failure.

### P37. The web client offers its source

- **Source**: README.md (License); docs/self-hosting.md:112 and 120;
  decision 0003.
- **Checked by**: the Playwright e2e test (`web/tests/e2e.spec.ts`) checks
  the source link.

### P38. Malformed frames, envelopes and attachment names cannot crash a client or the server, and a received name cannot escape the download directory

- **Source**: docs/threat-model.md:63-68; decision 0011.
- **Checked by**: `thrum-core` and `thrum-server` fuzz tests;
  `thrum-cli` `fuzz_sanitize_and_save_stay_inside_the_directory`;
  libFuzzer targets in `fuzz/`.
- **Could break under**: HTTP and WebSocket request parsing outside the
  listed parsers (decision 0011 leaves them to axum and serde); the web
  client rendering a name or message as HTML.

## Documented limitations

These are claims about what Thrum does not protect. A test campaign checks
that each limit is no wider than stated.

- **L1.** The server sees metadata: rosters, sizes, timing and addresses
  (docs/threat-model.md:26-27).
- **L2.** The server can drop, delay, reorder and censor messages
  (docs/threat-model.md:28-30).
- **L3.** A malicious server can substitute a device at an account's first
  sight; key transparency is out of scope (docs/threat-model.md:44-47).
- **L4.** TLS is an operator's job and not enforced in code
  (docs/threat-model.md:57-62).
- **L5.** Client state is not encrypted at rest (docs/threat-model.md:80-82;
  decision 0006 choice 9).
- **L6.** Any user can add any other user to a conversation without asking
  (#17).
- **L7.** A leaked blob id lets anyone logged in fetch that ciphertext
  (decision 0008, Consequences).

## Documentation corrections found while writing this list

- docs/self-hosting.md:162 says WebSocket connections go to `/ws`; the
  route is `/v1/ws`. The nginx example proxies `/`, so it works either way.
- docs/threat-model.md:101-103 says forgery is "exercised by M3 tests";
  see P7.
- docs/threat-model.md:23-25 says plaintext is "tested in M3 by scanning
  the server database". Until this list was written, that scan read a file
  that held no rows; see P1.
