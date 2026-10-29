CREATE TABLE settings (                 -- keys: workspace.name, workspace.logo_attachment_id, instance.id, instance.created_at
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL CHECK (json_valid(value))
) STRICT;

CREATE TABLE blobs (                    -- file: /data/files/<sha[0:2]>/<sha[2:4]>/<sha256>
  sha256     TEXT PRIMARY KEY CHECK (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
  size       INTEGER NOT NULL CHECK (size >= 0),
  mime       TEXT NOT NULL,             -- sniffed from magic bytes, never trusted from the client
  width      INTEGER,                   -- read from the image header only (imagesize)
  height     INTEGER,
  created_at INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

-- ---------- lookup tables: extend with INSERT, never with a table rebuild (D37) ----------
CREATE TABLE user_roles (role TEXT PRIMARY KEY) STRICT;
INSERT INTO user_roles(role) VALUES ('owner'),('admin'),('member'),('viewer');
CREATE TABLE item_kinds (kind TEXT PRIMARY KEY) STRICT;
INSERT INTO item_kinds(kind) VALUES ('doc'),('board');
CREATE TABLE content_formats (format TEXT PRIMARY KEY) STRICT;
INSERT INTO content_formats(format) VALUES ('yjs'),('inkboard');
CREATE TABLE revision_reasons (reason TEXT PRIMARY KEY) STRICT;
INSERT INTO revision_reasons(reason) VALUES ('auto'),('manual'),('pre-restore'),('restore'),('import'),('pre-api');
CREATE TABLE link_kinds (kind TEXT PRIMARY KEY) STRICT;
INSERT INTO link_kinds(kind) VALUES ('mention'),('link'),('embed');
CREATE TABLE token_scopes (scope TEXT PRIMARY KEY) STRICT;       -- hierarchy read ⊂ write ⊂ admin lives in Rust (Scope)
INSERT INTO token_scopes(scope) VALUES ('read'),('write'),('admin');

CREATE TABLE users (
  id                   TEXT PRIMARY KEY,
  email                TEXT NOT NULL UNIQUE COLLATE NOCASE,
  name                 TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
  password_hash        TEXT,            -- PHC argon2id; NULL = SSO-only (later), or cleared in an export copy
  role                 TEXT NOT NULL REFERENCES user_roles(role),
  avatar_sha256        TEXT REFERENCES blobs(sha256),
  theme                TEXT NOT NULL DEFAULT 'system' CHECK (theme IN ('light','dark','system')),
  must_change_password INTEGER NOT NULL DEFAULT 0 CHECK (must_change_password IN (0,1)),
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  disabled_at          INTEGER
) STRICT;
CREATE UNIQUE INDEX users_single_owner ON users(role) WHERE role = 'owner';

CREATE TABLE sessions (
  id           TEXT PRIMARY KEY,
  token_hash   BLOB NOT NULL UNIQUE,    -- sha256(32 random bytes)
  user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  persistent   INTEGER NOT NULL CHECK (persistent IN (0,1)),  -- "Keep me signed in"
  created_at   INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,        -- touched at most every 5 min
  expires_at   INTEGER NOT NULL,        -- window 30 d (persistent, capped at created_at + 180 d) or 24 h; renewal in §8
  user_agent   TEXT,
  ip           TEXT
) STRICT;
CREATE INDEX sessions_user    ON sessions(user_id);
CREATE INDEX sessions_expires ON sessions(expires_at);

CREATE TABLE api_tokens (                -- personal access tokens (D42, §7.3)
  id           TEXT PRIMARY KEY,         -- UUIDv7
  user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
  token_hash   BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),  -- sha256 of the whole 51-character token
  prefix       TEXT NOT NULL CHECK (length(prefix) = 12),             -- 'qpat_' + 7 characters, shown in lists
  created_via  TEXT NOT NULL CHECK (created_via IN ('web','api','admin')),
  parent_token_id TEXT REFERENCES api_tokens(id) ON DELETE SET NULL,  -- token that created it; revoked with it (D42)
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER,                  -- NULL = never
  last_used_at INTEGER,                  -- written at most once per minute
  last_used_ip TEXT,
  revoked_at   INTEGER
) STRICT;
CREATE INDEX api_tokens_user ON api_tokens(user_id, created_at DESC);
CREATE TABLE api_token_scopes (          -- expanded: a write token has rows 'read' and 'write'
  token_id TEXT NOT NULL REFERENCES api_tokens(id) ON DELETE CASCADE,
  scope    TEXT NOT NULL REFERENCES token_scopes(scope),
  PRIMARY KEY (token_id, scope)
) STRICT, WITHOUT ROWID;

CREATE TABLE collections (
  id           TEXT PRIMARY KEY,
  url_id       TEXT NOT NULL UNIQUE,
  name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
  description  TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 500),
  icon         TEXT,                    -- lucide icon name, or NULL = letter tile
  color        TEXT NOT NULL DEFAULT '#8D8D8D'
               CHECK (color GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
  sort_key     TEXT NOT NULL,
  child_order  TEXT NOT NULL DEFAULT 'manual' CHECK (child_order IN ('manual','title','updated','created')),
  tree_version INTEGER NOT NULL DEFAULT 1,  -- bumped on any structural change in the collection → ETag of /tree
  created_by   TEXT REFERENCES users(id) ON DELETE SET NULL,
  updated_by   TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  deleted_at   INTEGER,
  deleted_by   TEXT REFERENCES users(id) ON DELETE SET NULL
) STRICT;
CREATE INDEX collections_order ON collections(sort_key, id) WHERE deleted_at IS NULL;

CREATE TABLE items (
  id              TEXT PRIMARY KEY,
  url_id          TEXT NOT NULL UNIQUE,
  collection_id   TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  parent_id       TEXT REFERENCES items(id) ON DELETE CASCADE,   -- NULL = top level of the collection
  kind            TEXT NOT NULL REFERENCES item_kinds(kind),
  title           TEXT NOT NULL DEFAULT '' CHECK (length(title) <= 1000),
  icon            TEXT,                 -- reserved; NULL in M1
  sort_key        TEXT NOT NULL,
  is_template     INTEGER NOT NULL DEFAULT 0 CHECK (is_template IN (0,1)),  -- reserved
  full_width      INTEGER NOT NULL DEFAULT 0 CHECK (full_width IN (0,1)),
  created_by      TEXT REFERENCES users(id) ON DELETE SET NULL,
  updated_by      TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,     -- metadata or content change
  content_at      INTEGER,              -- last body save; NULL = never edited ("You created …")
  published_at    INTEGER,              -- set to created_at on create; NULL = draft, only via unpublish in M1 (derived visibility, D23)
  archived_at     INTEGER,
  archive_root_id TEXT,                 -- item whose archiving archived this row
  deleted_at      INTEGER,
  deleted_by      TEXT REFERENCES users(id) ON DELETE SET NULL,
  trash_root_id   TEXT,                 -- item (= id for the root) or collection whose deletion trashed this row
  meta_version    INTEGER NOT NULL DEFAULT 1,
  CHECK (parent_id IS NULL OR parent_id <> id),
  CHECK ((deleted_at IS NULL) = (trash_root_id IS NULL)),
  CHECK ((archived_at IS NULL) = (archive_root_id IS NULL))
) STRICT;
CREATE INDEX items_tree     ON items(collection_id, parent_id, sort_key, id);
CREATE INDEX items_children ON items(parent_id, sort_key, id);
CREATE INDEX items_trash    ON items(trash_root_id) WHERE deleted_at IS NOT NULL;
CREATE INDEX items_archive  ON items(archive_root_id) WHERE archived_at IS NOT NULL;
CREATE INDEX items_updated  ON items(updated_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX items_drafts   ON items(created_by, updated_at DESC) WHERE published_at IS NULL AND deleted_at IS NULL;

-- Structural guards (defence in depth; Rust checks the same rules first). Cycles are checked in the writer transaction
-- with a recursive CTE: SQLite documents CTEs inside triggers as unsupported.
CREATE TRIGGER items_ins_guard BEFORE INSERT ON items
BEGIN
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NOT EXISTS (
    SELECT 1 FROM collections c WHERE c.id = NEW.collection_id AND c.deleted_at IS NULL);
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NEW.parent_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM items p WHERE p.id = NEW.parent_id AND p.collection_id = NEW.collection_id
      AND p.deleted_at IS NULL AND p.archived_at IS NULL);
END;
-- A new parent must be live, unarchived and in the row's (new) collection.
CREATE TRIGGER items_parent_upd BEFORE UPDATE OF parent_id ON items
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id IS NOT OLD.parent_id
BEGIN
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NOT EXISTS (
    SELECT 1 FROM items p WHERE p.id = NEW.parent_id AND p.collection_id = NEW.collection_id
      AND p.deleted_at IS NULL AND p.archived_at IS NULL);
END;
-- A collection change needs a live target collection and, when the parent stays, a parent already in that collection
-- (so subtrees move top-down, §5 rules). Liveness of an unchanged parent is not checked: trashed or archived
-- descendants move with their subtree.
CREATE TRIGGER items_collection_upd BEFORE UPDATE OF collection_id ON items
WHEN NEW.collection_id IS NOT OLD.collection_id
BEGIN
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NOT EXISTS (
    SELECT 1 FROM collections c WHERE c.id = NEW.collection_id AND c.deleted_at IS NULL);
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NEW.parent_id IS NOT NULL AND NEW.parent_id IS OLD.parent_id
    AND NOT EXISTS (SELECT 1 FROM items p WHERE p.id = NEW.parent_id AND p.collection_id = NEW.collection_id);
END;

-- ---------- documents (Yjs) ----------
CREATE TABLE doc_state (
  item_id        TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
  ydoc           BLOB NOT NULL,         -- compacted Yjs v1 update of the whole doc; new doc = encodeStateAsUpdate(empty)
  yfield         TEXT NOT NULL DEFAULT 'default',
  schema_version INTEGER NOT NULL DEFAULT 1,   -- Wikanvas TipTap schema version (§6.1)
  compacted_seq  INTEGER NOT NULL DEFAULT 0,
  next_seq       INTEGER NOT NULL DEFAULT 1,
  pm_json        TEXT CHECK (pm_json IS NULL OR json_valid(pm_json)),  -- cache sent by the client; rebuildable
  pm_json_seq    INTEGER,
  last_token_id  TEXT,                  -- token of the last content write; NULL = session, seed or admin (D53)
  updated_at     INTEGER NOT NULL
) STRICT;
CREATE TABLE doc_updates (               -- append-only; the future WebSocket writes the same log
  id         INTEGER PRIMARY KEY,
  item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  seq        INTEGER NOT NULL,
  data       BLOB NOT NULL,              -- one validated Yjs v1 update
  user_id    TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (item_id, seq)
) STRICT;
CREATE TABLE doc_clients (               -- Yjs clientID → user; survives compaction (future attribution)
  item_id       TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  client_id     INTEGER NOT NULL,
  user_id       TEXT REFERENCES users(id) ON DELETE SET NULL,
  first_seen_at INTEGER NOT NULL,
  PRIMARY KEY (item_id, client_id)
) STRICT, WITHOUT ROWID;

-- ---------- boards (Inkboard) ----------
CREATE TABLE board_state (
  item_id        TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
  body           BLOB NOT NULL,          -- deflate(envelope JSON as received, every document.assets[*].data = "")
  body_sha256    BLOB NOT NULL,          -- sha256 of the uncompressed stripped envelope → skip no-op saves
  raw_len        INTEGER NOT NULL,
  format_version INTEGER NOT NULL,       -- envelope "version" (2)
  engine_version TEXT NOT NULL,          -- X-Wikanvas-Engine of the writer, e.g. "0.2.0-beta.3+embed.wikanvas.1"; "server" for seeds
  rev            INTEGER NOT NULL DEFAULT 1,   -- ETag "r{rev}"
  seq            INTEGER NOT NULL DEFAULT 0,   -- Sequencer seq (server-owned live sync, later)
  meta           TEXT NOT NULL CHECK (json_valid(meta)),  -- {"pages":[{"id","name"}],"objects":n,"background":"#rrggbb","bytes":n,"text_excerpt":"…"}
  last_token_id  TEXT,                   -- as in doc_state (D53)
  updated_at     INTEGER NOT NULL
) STRICT;
CREATE TABLE board_assets (              -- the AssetInfo manifest of inkboard-session
  item_id  TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  asset_id TEXT NOT NULL,
  sha256   TEXT NOT NULL REFERENCES blobs(sha256),
  mime     TEXT NOT NULL,
  PRIMARY KEY (item_id, asset_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX board_assets_blob ON board_assets(sha256);
CREATE TABLE board_previews (
  item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  page_index INTEGER NOT NULL,
  rev        INTEGER NOT NULL,           -- stale when <> board_state.rev
  svg        BLOB,                       -- deflate(svg); NULL = empty page
  svg_len    INTEGER NOT NULL,
  PRIMARY KEY (item_id, page_index)
) STRICT, WITHOUT ROWID;

-- ---------- history (written in M1, UI in M2) ----------
CREATE TABLE revisions (
  id             TEXT PRIMARY KEY,
  item_id        TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  format         TEXT NOT NULL REFERENCES content_formats(format),
  format_version INTEGER NOT NULL,       -- yjs: update encoding (1); inkboard: envelope version (2)
  data           BLOB NOT NULL,          -- yjs: full encoded state; inkboard: deflated stripped envelope
  preview        TEXT,                   -- yjs: pm_json at that time (display only; restore uses data)
  assets         TEXT CHECK (assets IS NULL OR json_valid(assets)),                  -- inkboard: {"asset_id":"sha256"} (blob GC roots)
  attachment_ids TEXT CHECK (attachment_ids IS NULL OR json_valid(attachment_ids)),  -- yjs: ["attachment id",…] (attachment GC roots)
  schema_version INTEGER NOT NULL DEFAULT 1,
  title          TEXT NOT NULL,
  reason         TEXT NOT NULL REFERENCES revision_reasons(reason),
  source_seq     INTEGER,                -- yjs: the seq it captures; inkboard: the rev (D53)
  label          TEXT,
  author_id      TEXT REFERENCES users(id) ON DELETE SET NULL,
  byte_size      INTEGER NOT NULL,
  created_at     INTEGER NOT NULL
) STRICT;
CREATE INDEX revisions_item ON revisions(item_id, created_at DESC);

-- ---------- attachments (doc images, workspace logo) ----------
CREATE TABLE attachments (
  id         TEXT PRIMARY KEY,           -- URL /api/v1/attachments/{id}
  item_id    TEXT REFERENCES items(id) ON DELETE SET NULL,  -- upload context only; never used for authorization (D36)
  sha256     TEXT NOT NULL REFERENCES blobs(sha256),
  filename   TEXT NOT NULL,              -- sanitized at upload (§8)
  mime       TEXT NOT NULL,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX attachments_blob ON attachments(sha256);
CREATE TABLE item_attachments (          -- rebuilt from the ytext walk in every doc save txn; copied by duplicate
  item_id       TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
  PRIMARY KEY (item_id, attachment_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX item_attachments_att ON item_attachments(attachment_id);

-- ---------- links (maintained on every doc save; backlinks UI later) ----------
CREATE TABLE item_links (
  source_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  target_id TEXT NOT NULL,               -- not an FK: a purged target renders as "Deleted item"
  kind      TEXT NOT NULL REFERENCES link_kinds(kind),
  PRIMARY KEY (source_id, target_id, kind)
) STRICT, WITHOUT ROWID;
CREATE INDEX item_links_target ON item_links(target_id);

-- ---------- per user ----------
CREATE TABLE stars (
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  sort_key   TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, item_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE item_views (
  user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  item_id         TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  view_count      INTEGER NOT NULL DEFAULT 1,
  first_viewed_at INTEGER NOT NULL,
  last_viewed_at  INTEGER NOT NULL,
  PRIMARY KEY (user_id, item_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX item_views_recent ON item_views(user_id, last_viewed_at DESC);
CREATE INDEX item_views_item   ON item_views(item_id);

-- ---------- full-text search ----------
CREATE TABLE item_text (
  text_id INTEGER PRIMARY KEY,           -- explicit, so VACUUM cannot renumber rowids
  item_id TEXT NOT NULL UNIQUE REFERENCES items(id) ON DELETE CASCADE,
  title   TEXT NOT NULL,
  body    TEXT NOT NULL                  -- doc: text derived from Y; board: page names, labels, table cells, latex
) STRICT;
CREATE VIRTUAL TABLE item_fts USING fts5(
  title, body, content = 'item_text', content_rowid = 'text_id',
  tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3');
CREATE TRIGGER item_text_ai AFTER INSERT ON item_text BEGIN
  INSERT INTO item_fts(rowid, title, body) VALUES (NEW.text_id, NEW.title, NEW.body);
END;
CREATE TRIGGER item_text_ad AFTER DELETE ON item_text BEGIN
  INSERT INTO item_fts(item_fts, rowid, title, body) VALUES ('delete', OLD.text_id, OLD.title, OLD.body);
END;
CREATE TRIGGER item_text_au AFTER UPDATE ON item_text BEGIN
  INSERT INTO item_fts(item_fts, rowid, title, body) VALUES ('delete', OLD.text_id, OLD.title, OLD.body);
  INSERT INTO item_fts(rowid, title, body) VALUES (NEW.text_id, NEW.title, NEW.body);
END;
CREATE TRIGGER items_text_ins AFTER INSERT ON items BEGIN
  INSERT INTO item_text(item_id, title, body) VALUES (NEW.id, NEW.title, '');
END;
CREATE TRIGGER items_text_title AFTER UPDATE OF title ON items WHEN NEW.title IS NOT OLD.title BEGIN
  UPDATE item_text SET title = NEW.title WHERE item_id = NEW.id;
END;
CREATE TRIGGER items_text_del BEFORE DELETE ON items BEGIN
  DELETE FROM item_text WHERE item_id = OLD.id;
END;
