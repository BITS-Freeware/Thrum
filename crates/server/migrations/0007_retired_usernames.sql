-- A deleted account's name is never registered again, so nobody can take
-- over the name, and the trust other users placed in it (decision 0015).
CREATE TABLE retired_usernames (
    username TEXT PRIMARY KEY,
    retired_at INTEGER NOT NULL
);
