# Thrum

An open-source messenger with only the core features: accounts, one-to-one and group messaging, attachments, and end-to-end encryption by default. No cryptocurrency, mini-apps, ads, or paid tiers.

"Thrum" is a working name.

Status: early development. See [ROADMAP.md](ROADMAP.md) for the plan and [STATUS.md](STATUS.md) for where work stands.

## Design in Brief

- **End-to-end encrypted.** Every conversation, including one-to-one chats, is an MLS group ([RFC 9420](https://www.rfc-editor.org/rfc/rfc9420)) built on [OpenMLS](https://github.com/openmls/openmls). The server relays and stores ciphertext only.
- **Rust core and server.** One shared core crate holds the protocol and client logic. The web client uses it through WebAssembly, and later native apps can use the same crate.
- **Self-hostable.** One server binary with SQLite for small installs and Postgres for larger ones.
- **No phone number required.** Accounts are a username plus the devices that hold its keys.

## License

[GNU Affero General Public License v3.0](LICENSE). If you run a modified server for other people, you must offer them its source.
