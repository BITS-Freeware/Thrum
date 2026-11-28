-- Join times are whole seconds, so two members who join in the same second
-- tie, and ownership on account deletion went to the older account rather
-- than the member who joined first. Record each member's place in the join
-- order instead. Existing rows keep the order the old query gave them.
ALTER TABLE conversation_members ADD COLUMN join_seq BIGINT NOT NULL DEFAULT 0;
UPDATE conversation_members SET join_seq = (
    SELECT COUNT(*)
    FROM conversation_members AS earlier
    WHERE earlier.conversation_id = conversation_members.conversation_id
      AND (earlier.joined_at < conversation_members.joined_at
           OR (earlier.joined_at = conversation_members.joined_at
               AND earlier.user_id <= conversation_members.user_id))
);
