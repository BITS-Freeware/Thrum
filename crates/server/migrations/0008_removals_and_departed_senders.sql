-- A member the admin removed cannot rejoin through an invite; the admin
-- adding them again deletes the row.
CREATE TABLE conversation_removals (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    removed_at      INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, user_id)
);

-- When an account is deleted, messages it sent that some member has not
-- received yet are kept until they are delivered or expire: they may be
-- commits the member needs. They move to this placeholder sender, which
-- no one can log in as or be added as (its name is not a valid username),
-- and keep the deleted sender's name in sender_name.
ALTER TABLE messages ADD COLUMN sender_name TEXT;
INSERT INTO users (username, password_hash, created_at) VALUES ('~departed', '!', 0);
INSERT INTO devices (id, user_id, label, token_hash, created_at, revoked_at)
SELECT 'departed', id, '', '!departed', 0, 0 FROM users WHERE username = '~departed';
