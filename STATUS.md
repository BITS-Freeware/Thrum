# Status

Thrum 0.2.1 is the current release.
It is a security release on top of 0.2.0; see `CHANGELOG.md` and the
repository's security advisories.

The server, CLI, and web client speak the same protocol. Self-hosting
defaults bind to loopback, the container runs as a dedicated user, and the
web client's WebAssembly is built in CI and in the image rather than stored
in git.

See [CHANGELOG.md](CHANGELOG.md) for what changed and
[docs/self-hosting.md](docs/self-hosting.md) for how to run a server.
