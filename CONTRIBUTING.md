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

## Fuzzing

Every parser that reads data from the network is fuzz-tested; the inventory
and the reasoning live in `docs/decisions/0011-fuzz-testing.md`. Two layers:

1. **In CI** — `cargo test` includes `crates/{core,server,cli}/tests/fuzz.rs`:
   thousands of deterministic random and mutated inputs per parser, checking
   that parsing is total (no panics) and that MLS group state survives
   garbage. These run everywhere, no extra setup.
2. **Coverage-guided, locally** — libFuzzer targets under `fuzz/` (a separate
   crate, nightly toolchain):

   ```sh
   cargo install cargo-fuzz
   cd fuzz
   cargo +nightly fuzz run envelope       # also: app_message, frames,
   cargo +nightly fuzz run mls_process    #       names
   ```

   Runs are not timed in CI; run them for a few minutes before touching a
   parser. Any crash input lands in `fuzz/artifacts/` and should become a
   regression test in the stable layer.

When you add a network-facing parser, add a target in both layers.

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
