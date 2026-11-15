# Status

Thrum 0.2.0 is the current version on `main` once this release is tagged.
0.1.0 remains the previous tagged release.

The server, CLI, and web client speak the same protocol. Self-hosting
defaults bind to loopback, the container runs as a dedicated user, and the
web client's WebAssembly is built in CI and in the image rather than stored
in git.

See [CHANGELOG.md](CHANGELOG.md) for what changed and
[docs/self-hosting.md](docs/self-hosting.md) for how to run a server.
