-- Blob objects whose rows are gone (account deletion) but whose objects
-- the store has not confirmed deleted yet. The sweeper retries them, and
-- an id stays unavailable for new uploads until its object is gone.
CREATE TABLE blob_tombstones (
    id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL
);
