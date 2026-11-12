ALTER TABLE conversations ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE conversations ADD COLUMN owner_user_id INTEGER REFERENCES users(id);

CREATE TABLE invites (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    token_hash      TEXT NOT NULL UNIQUE,
    created_by      INTEGER NOT NULL REFERENCES users(id),
    created_at      INTEGER NOT NULL,
    revoked_at      INTEGER
);

CREATE INDEX idx_invites_conversation ON invites(conversation_id);

CREATE TABLE blocks (
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked_user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (user_id, blocked_user_id)
);
