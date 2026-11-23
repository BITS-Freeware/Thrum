-- Mirrors migrations/0007.
CREATE TABLE retired_usernames (
    username TEXT PRIMARY KEY,
    retired_at BIGINT NOT NULL
);
