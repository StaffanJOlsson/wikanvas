-- Invite links (docs/ARCHITECTURE.md §5, §8, D148): made by the owner or an admin, single-use, expiring (7 days by
-- default, at most 30) and revocable. Only sha256(token) is stored; the token is shown once, in the invite's URL.
-- Accepting one creates a user with its role. Append-only after the first release (D3): this raises user_version to 2.
CREATE TABLE invites (
  id          TEXT PRIMARY KEY,                                      -- UUIDv7
  token_hash  BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),  -- sha256 of the whole 51-character 'qinv_' token
  role        TEXT NOT NULL REFERENCES user_roles(role) CHECK (role <> 'owner'),
  note        TEXT CHECK (length(note) BETWEEN 1 AND 200),           -- for the inviters only
  created_by  TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at  INTEGER NOT NULL,
  expires_at  INTEGER NOT NULL,
  used_at     INTEGER,                                               -- single use: set once, by the accept
  used_by     TEXT REFERENCES users(id) ON DELETE SET NULL,          -- the user the accept created
  revoked_at  INTEGER
) STRICT;
-- Pending invites (the list, the 100-pending cap) and those an inviter made (revoked when the inviter loses the right).
CREATE INDEX invites_pending ON invites(created_at DESC, id DESC) WHERE used_at IS NULL AND revoked_at IS NULL;
CREATE INDEX invites_created_by ON invites(created_by);
CREATE INDEX invites_used_by ON invites(used_by);
