# Contributing

Geist3 is a small, trustworthy messenger. Contributions that keep it small
and trustworthy are welcome.

## Building and testing

```sh
cargo fmt --all -- --check   # formatting
cargo clippy --all-targets -- -D warnings   # lint
cargo test                   # unit and integration tests
```

CI runs exactly these on every push and pull request, plus the web build and
browser tests once they exist (M6 onward). Please run them locally before
pushing.

## Project layout

- `crates/core` — protocol types and client logic, no I/O.
- `crates/server` — the axum/tokio server.
- `crates/cli` — the command line client and end-to-end test driver.
- `web/` — the TypeScript web client (from M6).
- `docs/` — architecture, threat model, and one decision per file in
  `docs/decisions/`.

## How we work

- Milestones live in [ROADMAP.md](ROADMAP.md); current state in
  [STATUS.md](STATUS.md).
- Changes to the architecture, the stack, the scope, or a security property
  need a decision record: add a numbered file to `docs/decisions/`, or mark
  an older one replaced.
- Keep commits small and green; write commit messages about the code.
- This is a public repository: never commit secrets, tokens, or personal
  data, and keep test fixtures to obviously fake values.

## License

By contributing you agree that your contributions are licensed under the
GNU Affero General Public License v3.0, the license of the project
([LICENSE](LICENSE)).
