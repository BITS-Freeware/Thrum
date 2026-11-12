# Self-hosting Thrum

Thrum is a small server plus clients. The server relays encrypted
messages and stores encrypted blobs; it never holds decryption keys. One
binary can serve both the API and the built web client on the same
origin, so a small install needs nothing but the server and a data
directory.

## Requirements

- Linux, macOS, or any POSIX system (or Docker)
- No database server required: SQLite is the default backend
- PostgreSQL 14+ if you prefer it (see below)

## Quick start with Docker Compose

```sh
git clone https://github.com/Zyniker13/Thrum.git
cd Thrum
docker compose up -d --build
```

The server listens on port 8080 and keeps everything in the
`thrum-data` volume. Open `http://your-host:8080/` for the web client,
or point the CLI at `http://your-host:8080`.

To run on PostgreSQL instead of SQLite:

```sh
docker compose --profile postgres up -d --build
# then uncomment THRUM_DB in docker-compose.yml and run again:
docker compose --profile postgres up -d
```

## From source

Building needs a Rust toolchain (2024 edition) and Node 22 for the web
client. The web client is optional; the CLI works without it.

```sh
git clone https://github.com/Zyniker13/Thrum.git
cd Thrum
cargo build --release -p thrum-server -p thrum-cli

# optional: build the web client and serve it from the server
(cd web && npm ci && npm run build)

THRUM_DB=/var/lib/thrum/thrum.sqlite \
THRUM_WEB_DIR=web/dist \
./target/release/thrum-server
```

The server creates the SQLite file if it is missing. With
`THRUM_WEB_DIR` set, `http://localhost:8080/` serves the web client;
without it, only the API is available (the web client can be hosted
anywhere static files are served, but it must share the server's origin,
because browsers only allow same-origin API calls).

## Configuration

All configuration is through environment variables; anything unset falls
back to the default shown.

| Variable | Default | Meaning |
| --- | --- | --- |
| `THRUM_DB` | `thrum.sqlite` | SQLite file path, or a `postgres://` URL |
| `THRUM_BIND` | `0.0.0.0` | Address to listen on (`127.0.0.1` for local only) |
| `THRUM_PORT` | `8080` | Port to listen on |
| `THRUM_WEB_DIR` | unset | Serve the built web client from this directory |
| `THRUM_ATTACHMENT_MAX_BYTES` | `20971520` | Largest accepted blob, bytes (20 MiB) |
| `THRUM_DAILY_UPLOAD_BYTES` | `104857600` | Per-user daily upload quota, bytes (100 MiB) |
| `THRUM_BLOB_RETENTION_SECONDS` | `2592000` | How long blobs are kept, seconds (30 days) |
| `THRUM_BLOB_BACKEND` | `disk` | `disk` or `s3` |
| `THRUM_BLOB_DIR` | `blobs` | Directory for disk-backed blobs |
| `THRUM_S3_ENDPOINT` | unset | S3-compatible endpoint (R2, MinIO, AWS) |
| `THRUM_S3_BUCKET` | unset | Bucket name |
| `THRUM_S3_REGION` | unset | Region (e.g. `auto` for R2) |
| `THRUM_S3_ACCESS_KEY_ID` | unset | Access key id |
| `THRUM_S3_SECRET_ACCESS_KEY` | unset | Secret access key |

Messages the server queues for offline devices are deleted after seven
days; clients fetch everything they missed when they reconnect. The
server keeps no delivered message history at all.

## PostgreSQL

Point `THRUM_DB` at a PostgreSQL URL; migrations run automatically on
first start:

```sh
THRUM_DB=postgres://thrum:secret@localhost:5432/thrum ./target/release/thrum-server
```

The same application runs the same schema on both backends. SQLite is
the right choice for one user or a family; PostgreSQL for many users on
one host or when you want managed backups.

## Object storage for blobs

Encrypted attachment blobs can live in any S3-compatible store (AWS S3,
Cloudflare R2, MinIO) instead of the local disk:

```sh
THRUM_BLOB_BACKEND=s3 \
THRUM_S3_ENDPOINT=https://<account>.r2.cloudflarestorage.com \
THRUM_S3_BUCKET=thrum \
THRUM_S3_REGION=auto \
THRUM_S3_ACCESS_KEY_ID=... \
THRUM_S3_SECRET_ACCESS_KEY=... \
./target/release/thrum-server
```

Existing blobs stay where they are; migration between backends is a copy
job (keys are opaque ciphertext).

## TLS and reverse proxy

The server speaks plain HTTP; put it behind a TLS-terminating proxy for
anything reachable from the internet. WebSocket connections to `/ws`
must be proxied with upgrade support. For nginx:

```nginx
location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
}
```

The same proxy can also serve the web client's static files itself
instead of `THRUM_WEB_DIR`; both work, because the client always talks
to its own origin.

## Backups

- **SQLite**: stop the server (or use `sqlite3 thrum.sqlite ".backup
  ..."`), then copy the database file and the blob directory.
- **PostgreSQL**: `pg_dump` plus the blob directory (or the object
  store).
- The database is useless without the blobs and vice versa: back up both
  together.

Blobs and messages are ciphertext; backing them up is safe but restoring
is only useful together with the users' own device key material, which
never leaves their machines.

## Upgrading

Pull the new version and restart the server. Migrations run
automatically at startup and are forward-only. Client upgrades are
independent; old clients keep working until the protocol changes (the
roadmap tracks compatibility).

## What the operator can and cannot see

The server stores usernames, device identities, conversation membership,
and opaque encrypted payloads. It cannot read message contents, group
membership changes beyond metadata, attachment contents, or message
edits and deletions beyond metadata. Limits (attachment size, quota) and
availability are the operator's responsibilities. See
[SECURITY.md](../SECURITY.md) and the threat model for details.
