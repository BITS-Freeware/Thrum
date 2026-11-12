# 0009 - PostgreSQL support alongside SQLite

Date: 2026-11-12
Status: accepted (M7)

## Context

The roadmap requires PostgreSQL support next to SQLite. The server was
built SQLite-first (decision 0004): every query is runtime-checked SQL
(no compile-time macros), all timestamps are Unix seconds in INTEGER
columns, and ids are TEXT. Two realistic implementation paths exist: a
storage abstraction with one implementation per backend, or one query
set that both backends execute.

## Decisions

1. **One query set, two backends, via sqlx's `any` driver.** All runtime
   SQL uses `$1`-style placeholders, which PostgreSQL requires and SQLite
   accepts (named parameters bind positionally in sqlx), and binding is
   positional, so every query runs unchanged on either backend. This
   avoids a trait layer over ~25 call sites and keeps both backends
   exercised by the same integration suite.
2. **Separate migration sets.** Column types differ (`INTEGER PRIMARY
   KEY` autoincrement vs `BIGINT GENERATED ALWAYS AS IDENTITY`, `BLOB` vs
   `BYTEA`, case-insensitive usernames via `COLLATE NOCASE` vs a
   `lower(username)` unique index), so SQLite keeps `migrations/` and
   PostgreSQL gets `migrations-pg/` with identical versions and order.
   The backend is chosen by the connection string: a `postgres://` URL in
   `THRUM_DB` selects PostgreSQL, anything else is a SQLite path.
3. **Insertion-order tiebreaks are explicit.** Catch-up delivery and key
   package selection order by `created_at` plus an insertion-order
   tiebreak; SQLite uses its implicit `rowid`, PostgreSQL adds `seq`
   identity columns to `messages` and `key_packages`. The one-line
   fragment is chosen from `AppState.backend`.
4. **Case-insensitive usernames behave identically.** Lookups use
   `lower(username) = lower($1)` on both backends; PostgreSQL enforces
   uniqueness with an expression index, SQLite with `COLLATE NOCASE`
   (plus an expression index for seeking).
5. **The integration suite runs against both backends.**
   `THRUM_TEST_DB_URL` redirects the whole server test suite to a
   PostgreSQL URL (each test gets its own throwaway database), and CI
   runs it against a postgres:17 service container next to the SQLite
   run. Aggregate expressions are written to return BIGINT on both
   backends (`CAST(COALESCE(SUM(...), 0) AS BIGINT)`) because the any
   driver cannot decode PostgreSQL `numeric`.

## Consequences

- Operators choose SQLite (zero setup) or PostgreSQL (concurrent access,
  managed offerings) through one environment variable.
- Every runtime query must stay portable; the PostgreSQL CI job fails
  when a change introduces SQLite-only SQL.
- SQLite migrations may only be edited while no deployed database exists
  (checksum validation); the 0.1.0 release is the cut-off.
