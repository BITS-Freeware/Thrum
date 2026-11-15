-- Mirrors migrations/0005. BIGINT matches the other integer columns.
ALTER TABLE key_packages ADD COLUMN last_resort BIGINT NOT NULL DEFAULT 0;

ALTER TABLE invites ADD COLUMN expires_at BIGINT;
ALTER TABLE invites ADD COLUMN max_uses BIGINT;
ALTER TABLE invites ADD COLUMN use_count BIGINT NOT NULL DEFAULT 0;
