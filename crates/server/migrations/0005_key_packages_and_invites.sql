-- One-time key packages stay consumable. A last-resort package (last_resort
-- = 1) is kept and served again once the one-time stock is empty.
ALTER TABLE key_packages ADD COLUMN last_resort INTEGER NOT NULL DEFAULT 0;

-- Invites can expire and can be limited to a number of redemptions.
-- NULL expires_at means the invite does not expire. NULL max_uses means
-- it can be redeemed without limit. Existing rows keep both unset.
ALTER TABLE invites ADD COLUMN expires_at INTEGER;
ALTER TABLE invites ADD COLUMN max_uses INTEGER;
ALTER TABLE invites ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;
