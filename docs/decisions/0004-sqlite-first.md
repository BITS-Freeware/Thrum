# 0004: SQLite First, Postgres Behind One Query Layer

Status: accepted

The server stores data through `sqlx` with SQLite as the default database:
it is the store for development, tests and small self-hosted installs, and
it needs no separate service. Postgres support is added in M7 for larger
installs, implemented against the same query layer so both backends share
one set of queries and one schema definition per backend.

Considered instead: Postgres only, which would make tests and small
installs depend on a running database service; or an ORM, which would hide
the queries this project needs to audit.
