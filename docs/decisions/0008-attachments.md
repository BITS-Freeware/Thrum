# 0008 - Attachments: encrypted blobs with operator-set limits

Date: 2026-11-12
Status: accepted (M5)

## Context

Attachments are files sent through the relay. The server must keep seeing
only ciphertext, storage must work for small self-hosted installs and for
object storage with free egress (media bandwidth is the largest running
cost), and every limit must be the operator's choice.

## Decisions

1. **Per-file encryption is AES-256-GCM** (RustCrypto `aes-gcm`). A fresh
   32-byte key and 12-byte nonce are generated per file on the client. The
   blob id is bound as AAD, so a blob cannot be re-registered under another
   id and pass verification. No hand-written cryptography.
2. **The per-file key travels inside MLS.** `AppMessage::Attachment` carries
   blob id, key, nonce, file name, MIME type, plaintext size and a SHA-256
   of the plaintext. The server stores and relays it like any other
   encrypted payload and learns nothing about the file.
3. **Blob ids are bearer capabilities.** `POST /v1/blobs` takes the
   ciphertext bytes and returns a random 128-bit id; `GET /v1/blobs/{id}`
   serves the bytes to any authenticated device that can name the id. The
   id appears only inside MLS ciphertext, so only group members learn it.
   The server cannot check conversation membership for a fetch without
   leaking which messages carry attachments, so it does not try.
4. **Expiry is age-based.** Each blob row records an expiry time
   (upload time + retention). A sweeper deletes expired rows and objects.
   Clients are expected to keep their own decrypted copies; consistent with
   the relay keeping no delivered history.
5. **Storage behind a `BlobStore` trait**: a local-disk implementation and
   an S3-compatible one via the `object_store` crate (covers AWS S3,
   Cloudflare R2, MinIO). Backend and credentials come from environment
   variables only.
6. **Limits, all operator-configurable by environment:**
   - `GEIST3_ATTACHMENT_MAX_BYTES` (default 20 MiB) - largest accepted
     encrypted upload; refused with `too_large`.
   - `GEIST3_DAILY_UPLOAD_BYTES` (default 100 MiB) - per-user quota counted
     over the current UTC calendar day; refused with `quota_exceeded`.
   - `GEIST3_BLOB_RETENTION_SECONDS` (default 30 days).
   - `GEIST3_BLOB_BACKEND` (`disk` default, `s3`), `GEIST3_BLOB_DIR`,
     and `GEIST3_S3_ENDPOINT`, `GEIST3_S3_BUCKET`, `GEIST3_S3_REGION`,
     `GEIST3_S3_ACCESS_KEY_ID`, `GEIST3_S3_SECRET_ACCESS_KEY`.
7. **Images are resized and recompressed on the client before encryption**,
   so large photos do not waste quota or bandwidth. The CLI does this with
   the `image` crate; the web client (M6) will use browser canvas APIs, so
   the resize logic stays out of core.
8. **New third-party dependencies** (all well-maintained, recorded per the
   security policy): `aes-gcm` 0.10 (RustCrypto AEAD), `object_store`
   (Apache Arrow project, S3 client), `image` (client-side image decode/
   resize/encode, CLI only).

## Consequences

- The server never sees attachment bytes in the clear; its database stores
  only blob ids, sizes, uploader ids and times.
- A leaked blob id allows fetching the ciphertext, which is useless without
  the per-file key; it does allow unlinking traffic shape (size, timing)
  with the id. Rate limiting and unguessable ids are the mitigation.
- Deleting a message does not delete its blob early; retention governs.
