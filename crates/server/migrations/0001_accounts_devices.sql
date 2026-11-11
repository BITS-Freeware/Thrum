CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE devices (
    id         TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label      TEXT NOT NULL DEFAULT '',
    token_hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);

CREATE TABLE key_packages (
    id         TEXT PRIMARY KEY,
    device_id  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    content    BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_devices_user ON devices(user_id);
CREATE INDEX idx_key_packages_device ON key_packages(device_id, created_at);
