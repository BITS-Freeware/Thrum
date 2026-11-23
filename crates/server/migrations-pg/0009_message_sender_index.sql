-- Mirrors migrations/0009.
CREATE INDEX idx_messages_sender ON messages(sender_device);
