-- Access per collection (docs/ARCHITECTURE.md §5, §8, D152). Every member and viewer gets a collection's
-- default_access ('none', 'read' or 'write'); a membership grants a person more ('read', 'write' or 'manage'), or any
-- access to a 'none' collection; the role caps both (a viewer reads at most), and the owner and admins manage every
-- collection. Existing collections get 'write', which is what every member and viewer had before (members edit,
-- viewers read), and their creators a 'manage' membership, as a new collection's creator gets. Groups later get a
-- table of their own beside this one. Append-only after the first release (D3): this raises user_version to 3.
ALTER TABLE collections ADD COLUMN default_access TEXT NOT NULL DEFAULT 'write'
  CHECK (default_access IN ('none','read','write'));

CREATE TABLE collection_members (
  collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  access        TEXT NOT NULL CHECK (access IN ('read','write','manage')),
  created_by    TEXT REFERENCES users(id) ON DELETE SET NULL,  -- who added the person; NULL for the migration
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  PRIMARY KEY (collection_id, user_id)
) STRICT, WITHOUT ROWID;
-- A user's memberships (the visibility filter, the epoch, a deleted user's rows).
CREATE INDEX collection_members_user ON collection_members(user_id, collection_id);

INSERT INTO collection_members (collection_id, user_id, access, created_by, created_at, updated_at)
  SELECT id, created_by, 'manage', NULL, created_at, created_at FROM collections WHERE created_by IS NOT NULL;

-- The per-viewer permission epoch of the tree ETag (D21): bumped whenever the user's access may have changed (a
-- membership, the role, disabling or enabling the account), so a cached tree is never revalidated across it.
ALTER TABLE users ADD COLUMN access_epoch INTEGER NOT NULL DEFAULT 0;
