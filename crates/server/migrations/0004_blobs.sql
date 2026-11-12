-- Blobs table and index.
-- `created_at` holds Unix seconds like every other timestamp; the insert
-- always supplies it explicitly.
CREATE TABLE IF NOT EXISTS blobs (
    id         TEXT PRIMARY KEY,
    uploader_id INTEGER NOT NULL REFERENCES users(id),
    size       INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS blobs_expires_at ON blobs(expires_at);
