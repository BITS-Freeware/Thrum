-- Blobs table and index. Mirrors migrations/0004 with Postgres types.
-- `created_at` holds Unix seconds like every other timestamp.
CREATE TABLE IF NOT EXISTS blobs (
    id         TEXT PRIMARY KEY,
    uploader_id BIGINT NOT NULL REFERENCES users(id),
    size       BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS blobs_expires_at ON blobs(expires_at);
