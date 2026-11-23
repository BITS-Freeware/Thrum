-- Mirrors migrations/0008.
CREATE TABLE conversation_removals (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id         BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    removed_at      BIGINT NOT NULL,
    PRIMARY KEY (conversation_id, user_id)
);

ALTER TABLE messages ADD COLUMN sender_name TEXT;
INSERT INTO users (username, password_hash, created_at) VALUES ('~departed', '!', 0);
INSERT INTO devices (id, user_id, label, token_hash, created_at, revoked_at)
SELECT 'departed', id, '', '!departed', 0, 0 FROM users WHERE username = '~departed';
