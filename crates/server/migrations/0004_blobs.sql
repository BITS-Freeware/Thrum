-- Blobs table and index.
CREATE TABLE IF NOT EXISTS blobs (
    id TEXT PRIMARY KEY,
    uploader_id INTEGER NOT NULL REFERENCES users(id),
    size INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS blobs_expires_at ON blobs(expires_at);
