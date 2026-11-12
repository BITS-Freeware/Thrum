-- Conversations and messages. Mirrors migrations/0002 with Postgres types.
-- `seq` gives a stable insertion order for catch-up; SQLite uses its
-- implicit rowid for the same purpose.
CREATE TABLE conversations (
    id         TEXT PRIMARY KEY,
    created_at BIGINT NOT NULL
);

CREATE TABLE conversation_members (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id         BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at       BIGINT NOT NULL,
    PRIMARY KEY (conversation_id, user_id)
);

CREATE TABLE messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sender_device   TEXT NOT NULL REFERENCES devices(id),
    payload         BYTEA NOT NULL,
    created_at      BIGINT NOT NULL,
    seq             BIGINT GENERATED ALWAYS AS IDENTITY
);

CREATE TABLE deliveries (
    message_id   TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    device_id    TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    queued_at    BIGINT NOT NULL,
    delivered_at BIGINT,
    PRIMARY KEY (message_id, device_id)
);

CREATE INDEX idx_conversation_members_user ON conversation_members(user_id);
CREATE INDEX idx_deliveries_device ON deliveries(device_id, delivered_at);
