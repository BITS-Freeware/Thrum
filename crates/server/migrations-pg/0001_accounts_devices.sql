-- Accounts and devices. Mirrors migrations/0001 with Postgres types:
-- auto-incrementing ids, bytea blobs, and a case-insensitive unique
-- username expressed with an expression index (no COLLATE NOCASE).
CREATE TABLE users (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    username      TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at    BIGINT NOT NULL
);

CREATE UNIQUE INDEX users_username_ci ON users(lower(username));

CREATE TABLE devices (
    id         TEXT PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label      TEXT NOT NULL DEFAULT '',
    token_hash TEXT NOT NULL UNIQUE,
    created_at BIGINT NOT NULL,
    revoked_at BIGINT
);

CREATE TABLE key_packages (
    id         TEXT PRIMARY KEY,
    device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    content    BYTEA NOT NULL,
    created_at BIGINT NOT NULL,
    seq        BIGINT GENERATED ALWAYS AS IDENTITY
);

CREATE INDEX idx_devices_user ON devices(user_id);
CREATE INDEX idx_key_packages_device ON key_packages(device_id, created_at);
