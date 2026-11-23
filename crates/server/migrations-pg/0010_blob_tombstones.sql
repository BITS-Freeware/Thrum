-- Mirrors migrations/0010.
CREATE TABLE blob_tombstones (
    id TEXT PRIMARY KEY,
    created_at BIGINT NOT NULL
);
