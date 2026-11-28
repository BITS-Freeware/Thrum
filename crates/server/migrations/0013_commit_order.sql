-- Two devices that commit in the same MLS epoch used to have both commits
-- relayed; every other device applied the first and the second committer
-- was left on its own copy of the group. A client now says which epoch a
-- commit is for, and the server takes one commit per epoch: the next
-- epoch it expects, recorded here, and the last commit it took, so that a
-- retry of that same commit is answered as before instead of refused.
ALTER TABLE conversations ADD COLUMN commit_epoch INTEGER;
ALTER TABLE conversations ADD COLUMN last_commit_hash TEXT;
ALTER TABLE conversations ADD COLUMN last_commit_id TEXT;
