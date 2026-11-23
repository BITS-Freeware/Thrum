-- Sends sum what the sending account still has queued; account deletion
-- finds the messages a device sent.
CREATE INDEX idx_messages_sender ON messages(sender_device);
