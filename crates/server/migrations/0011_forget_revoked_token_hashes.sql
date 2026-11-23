-- A signed-out or revoked device's token can never authenticate again, so
-- its hash serves no purpose. Replace it with a unique placeholder that no
-- token hashes to; revocation and logout do the same from now on.
UPDATE devices SET token_hash = 'revoked:' || id WHERE revoked_at IS NOT NULL;
