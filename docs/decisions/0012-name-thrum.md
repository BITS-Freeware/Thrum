# Decision 0012: Name the Project Thrum

Date: 2026-11-14 (after 0.1.0)

## Context

The project was built under a working name. It needs a lasting
name that does not tie it to one company, since it is meant to be used and
self-hosted widely. The name had to be free of existing messengers and free
as a prefix on crates.io.

## Decision

The project is named **Thrum**. Every identifier follows: crates
`thrum-core`, `thrum-server`, `thrum-cli` and `thrum-wasm`; environment
variables `THRUM_*`; the blob header `x-thrum-blob-id`; the export format
`thrum-export`; and the web package `thrum-web`. The repository lives at
[BITS-Freeware/Thrum](https://github.com/BITS-Freeware/Thrum).

## Consequences

- Servers from release 0.1.0 used a different environment-variable prefix and must switch to `THRUM_*`.
  No compatibility aliases are kept, since 0.1.0 had no known deployments.
- Exports written by 0.1.0 use a different format string; nothing reads that field
  yet, so no migration is needed.
- Nothing in the cryptography or the database schema changes, so no data
  migration is needed.
