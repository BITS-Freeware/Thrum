# Self-hosting Thrum

Thrum is a small server plus clients. The server relays encrypted
messages and stores encrypted blobs; it never holds decryption keys. One
binary can serve both the API and the built web client on the same
origin, so a small install needs nothing but the server and a data
directory.

## Requirements

- Linux, macOS, or any POSIX system (or Docker)
- Rust 1.91 and Node 22 when building from source
- No database server required: SQLite is the default backend
- PostgreSQL 14+ if you prefer it (see below)

## Quick start with Docker Compose

```sh
git clone https://github.com/BITS-Freeware/Thrum.git
cd Thrum
docker compose up -d --build
```

Compose publishes the server on `127.0.0.1:8080` only. On the same
machine, open `http://127.0.0.1:8080/` for the web client, or point the
CLI at `http://127.0.0.1:8080`. Data stays in the `thrum-data` volume.

The container starts as root long enough to give a new `/data` volume to
user `thrum` (uid 10001), then the server runs as that user with no
capabilities and `no_new_privs`. The Compose file also drops every
capability but the three the entrypoint needs, keeps the root filesystem
read-only, and uses `thrum-server healthcheck` as its health check. The
image has no setuid programs. If you start the binary
yourself against an existing volume, that user must be able to write it:

```sh
sudo chown -R 10001:10001 /path/to/data
```

Do not publish port 8080 on a public interface. The server speaks plain
HTTP, so a public listener sends passwords and session tokens in the
clear. Put Caddy, nginx, or another TLS proxy in front of it first; the
section below has an nginx example. The image sets `THRUM_BIND=0.0.0.0`
so the process can accept the proxy's connection from outside the
container's network namespace. A binary you start yourself listens on
`127.0.0.1` unless you set `THRUM_BIND`, and it prints a warning when
that address is not a loopback address. Inside the image, where
`0.0.0.0` is the image's own default, it prints a one-line note instead;
setting `THRUM_BIND` to another address brings the full warning back.

Without Compose, publish the port on loopback only. A plain `-p 8080:8080`
listens on every interface, and Docker's published ports bypass host
firewalls such as ufw:

```sh
docker run -d -p 127.0.0.1:8080:8080 -v thrum-data:/data <image>
```

To run on PostgreSQL instead of SQLite, uncomment `THRUM_DB` in
`docker-compose.yml` first, then start both services:

```sh
docker compose --profile postgres up -d --build
```

Starting without `THRUM_DB` set creates a SQLite database instead, and
accounts made there stay there; Thrum has no tool to move a SQLite
database to PostgreSQL.

## From source

Building needs Rust 1.91 (`rust-toolchain.toml` selects it) and Node 22
for the web client. The web client is optional; the CLI works without it.
The wasm package is built from the `thrum-wasm` crate; it is not stored
in git.

```sh
git clone https://github.com/BITS-Freeware/Thrum.git
cd Thrum
cargo build --release -p thrum-server -p thrum-cli

# optional: build the web client and serve it from the server
./scripts/build-wasm.sh
(cd web && npm ci && npm run build)

THRUM_DB=/var/lib/thrum/thrum.sqlite \
THRUM_WEB_DIR=web/dist \
./target/release/thrum-server
```

`THRUM_DB` and `THRUM_BLOB_DIR` default to `thrum.sqlite` and `blobs` in
the working directory, so a service started from another directory comes
up with an empty database. Give absolute paths in a service unit. The
server prints its effective configuration at startup (the S3 secret shown
only as set or unset) and refuses to start on a value it cannot use: a
number that does not parse, an unknown `THRUM_BLOB_BACKEND`, or a
`THRUM_TRUSTED_PROXIES` entry that is not an address or CIDR.

The server creates the SQLite file if it is missing, readable by its own
user only, and the blob directory and every blob the same way. A database
or blob directory from an older version keeps its mode; tighten it with
`chmod 600 thrum.sqlite* && chmod -R go-rwx blobs`. The container image
does this for `/data` at every start. With
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
| `THRUM_BIND` | `127.0.0.1` | Address to listen on. The container image sets `0.0.0.0` |
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
| `THRUM_REGISTER_PER_MINUTE` | `20` | Registrations per client address per minute |
| `THRUM_LOGIN_PER_MINUTE` | `60` | Login attempts per client address per minute |
| `THRUM_LOGIN_PER_ACCOUNT_PER_MINUTE` | `10` | Login attempts per account per client address per minute |
| `THRUM_LOGIN_FAILURES_BEFORE_BACKOFF` | `5` | Failed passwords from one address before that address's delay for the account starts |
| `THRUM_LOGIN_BACKOFF_BASE_SECONDS` | `30` | First delay; it doubles after each further failure |
| `THRUM_LOGIN_BACKOFF_CAP_SECONDS` | `900` | Longest delay |
| `THRUM_LOGIN_FAILURES_PER_ACCOUNT_PER_HOUR` | `100` | Failed passwords against one account per hour, from any address, before every address is refused for the rest of the hour (`0` is no ceiling). Wrong passwords on account deletion count too |
| `THRUM_TRUSTED_PROXIES` | empty | Comma-separated CIDRs. `X-Forwarded-For` and `Forwarded` are read only when the immediate peer is in this list; the client address is the rightmost address that is not |
| `THRUM_KEY_PACKAGE_FETCH_PER_CALLER_PER_MINUTE` | `120` | Key-package fetches per logged-in user per minute |
| `THRUM_KEY_PACKAGE_FETCH_PER_CALLER_DEVICE_PER_MINUTE` | `10` | Key-package fetches of one device by one user per minute |
| `THRUM_KEY_PACKAGE_FETCH_PER_DEVICE_PER_MINUTE` | `60` | Key-package fetches of one device per minute, from everyone together. Past this, fetches get the device's last-resort package and its one-time stock is left alone |
| `THRUM_KEY_PACKAGE_MAX_PER_DEVICE` | `100` | One-time key packages stored per device (`0` is unlimited). One last-resort package is extra and is not deleted |
| `THRUM_MAX_GROUP_MEMBERS` | `50` | Members per conversation, including the creator (`0` is unlimited). Holds for creating, adding and invites |
| `THRUM_MAX_DEVICES_PER_ACCOUNT` | `20` | Signed-in devices per account (`0` is unlimited). Login answers `409 too_many_devices` at the cap; logging out or revoking a device makes room |
| `THRUM_MESSAGES_PER_DEVICE_PER_MINUTE` | `120` | Messages one device may send per minute |
| `THRUM_QUEUED_BYTES_PER_ACCOUNT` | `104857600` | Bytes of an account's sent messages that some device has not received yet (100 MiB, `0` is unlimited). A send past it answers `429 queue_full` until devices catch up or the queue expires |
| `THRUM_SOURCE_URL` | the Thrum repository | Link the web client shows for this server's source (AGPL) |

New invites expire after 7 days and can be redeemed once. A create
request can set `ttl_seconds` or `max_uses` to `0` for no expiry or
unlimited redemptions. Invites created before 0.2.0 stay unlimited.

The web client and the API send `Content-Security-Policy`,
`X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and
`X-Frame-Options: DENY`. `GET /v1/meta` returns the configured source URL.

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

The server speaks plain HTTP. Anything reachable from another machine
needs a TLS-terminating proxy in front of it. Serving port 8080 directly
on a public address exposes passwords and session tokens. WebSocket
connections to `/v1/ws` must be proxied with upgrade support. For nginx:

```nginx
server {
    listen 443 ssl;
    http2 on;
    server_name chat.example.org;
    ssl_certificate     /etc/letsencrypt/live/chat.example.org/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/chat.example.org/privkey.pem;
    add_header Strict-Transport-Security "max-age=31536000" always;

    # Above THRUM_ATTACHMENT_MAX_BYTES (20 MiB by default); nginx's own
    # default of 1 MiB refuses most attachments.
    client_max_body_size 21m;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        # Replace, never append: a client must not choose its own address.
        proxy_set_header X-Forwarded-For $remote_addr;
        # Live connections stay open for hours; the server pings every
        # 30 seconds, well inside this.
        proxy_read_timeout 1h;
    }
}

server {
    listen 80;
    server_name chat.example.org;
    return 301 https://$host$request_uri;
}
```

If you raise `THRUM_ATTACHMENT_MAX_BYTES`, raise `client_max_body_size`
with it.

Rate limits follow the client address, so the server must know it. Set
`THRUM_TRUSTED_PROXIES` to the address the proxy connects from, and make
sure the proxy overwrites `X-Forwarded-For` as above (Caddy's
`reverse_proxy` does this by itself). Without it, every client shares the
proxy's address and one client's failed logins lock out everyone. With the
Compose file, the server sees the Docker network's gateway, not
`127.0.0.1`: use that network's subnet, which `docker network inspect`
shows (often inside `172.16.0.0/12`).

The same proxy can also serve the web client's static files itself
instead of `THRUM_WEB_DIR`; both work, because the client always talks
to its own origin.

## Backups

- **SQLite**: stop the server, then copy the database file and the blob
  directory. The server stops cleanly on SIGTERM (`docker compose stop`,
  `systemctl stop`) and writes its write-ahead log back into the database
  file, so after a clean stop `thrum.sqlite` alone holds everything. While
  it runs, the latest rows can still be in `thrum.sqlite-wal`: either copy
  `thrum.sqlite`, `thrum.sqlite-wal` and `thrum.sqlite-shm` together, or
  make a consistent copy with `sqlite3 thrum.sqlite ".backup backup.sqlite"`
  (the `sqlite3` tool is not in the container image; run it on the host
  against the volume's files).
- With Compose: `docker compose stop thrum`, copy the `thrum-data` volume
  (for example `docker run --rm -v thrum_thrum-data:/data -v "$PWD":/out
  debian:bookworm-slim tar -C /data -czf /out/thrum-data.tgz .`), then
  `docker compose start thrum`.
- **PostgreSQL**: `pg_dump` plus the blob directory (or the object
  store).
- The database is useless without the blobs and vice versa: back up both
  together.
- Deletion and backups: SQLite overwrites deleted rows in place
  (`secure_delete`), but a backup taken before an account was deleted still
  holds it until you rotate it. PostgreSQL keeps deleted rows on disk until
  `VACUUM` reclaims them; autovacuum does this over time.

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
