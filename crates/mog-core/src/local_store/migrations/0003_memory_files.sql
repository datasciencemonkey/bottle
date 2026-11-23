-- Migration 0003 (local memory slice 1, ADR 0112, ADR 0113): memories as an OpenClaw-style
-- virtual folder. Forward only (MEM-35a). It rebuilds memory_item in place: every row, its rid,
-- its vec rows and its FTS text stay. No row is deleted (U9).
--
-- New columns: the file fields (title, body_md, tags, origin, layer, note_day, slug), the links
-- (profile, recording_id), the use and removal fields, and the user feedback (rating,
-- reviewed_at, released_at). The 'entity' kind and the 'proposed' status do not exist.
--
-- Old rows: every row goes to the daily note of the day it was first seen. An 'entity' row becomes
-- a 'fact' with the tag 'legacy-entity'; if it is not quarantined it becomes 'stale', so recall
-- does not use it until the user rates it up. No table references memory_item, so the rebuild
-- keeps every foreign key.

CREATE TABLE memory_item_new (
  rid INTEGER PRIMARY KEY,
  item_id TEXT NOT NULL UNIQUE,
  mog_id TEXT NOT NULL REFERENCES mog(mog_id) ON DELETE RESTRICT,
  owner_principal TEXT,
  item_type TEXT NOT NULL CHECK (item_type IN ('fact','preference','definition','decision','open_question','genie_answer')),
  text TEXT NOT NULL CHECK (length(text) BETWEEN 1 AND 600),
  source TEXT NOT NULL CHECK (source IN ('you','others','genie','agent','confirmed')),
  entities TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(entities)),
  entity_keys TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(entity_keys)),
  attributes TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(attributes)),
  confidence REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','stale','superseded','resolved','retracted','quarantined','removed')),
  superseded_by TEXT,
  resolved_by TEXT,
  pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0,1)),
  sensitivity TEXT NOT NULL DEFAULT 'normal' CHECK (sensitivity IN ('normal','restricted')),
  source_session_ids TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(source_session_ids)),
  source_utterance_ids TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(source_utterance_ids)),
  source_exchange_id TEXT REFERENCES genie_exchange(exchange_id) ON DELETE SET NULL,
  workspace_host TEXT,
  workspace_id INTEGER,
  space_id TEXT,
  mention_count INTEGER NOT NULL DEFAULT 1,
  first_seen_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  valid_until INTEGER,
  embedding BLOB,
  embedding_model TEXT,
  embedding_dims INTEGER CHECK (embedding_dims IN (256, 1024)),
  embedded_at INTEGER,
  expires_at INTEGER,
  created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  deleted_at INTEGER,
  row_hlc INTEGER NOT NULL,
  origin_device_id TEXT NOT NULL,
  server_seq INTEGER NOT NULL DEFAULT 0,
  -- The file fields (ADR 0112).
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 80),
  body_md TEXT NOT NULL DEFAULT '' CHECK (length(body_md) <= 4000),
  tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array' AND json_array_length(tags) <= 8),
  origin TEXT NOT NULL DEFAULT 'legacy' CHECK (origin IN ('pin','added','kept_answer','correction','dictation_word','call_notes','flush','agent','dream','merge','legacy')),
  layer TEXT NOT NULL DEFAULT 'daily' CHECK (layer IN ('user','durable','daily')),
  note_day TEXT CHECK (note_day IS NULL OR note_day GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
  slug TEXT CHECK (slug IS NULL OR (length(slug) BETWEEN 1 AND 72 AND slug NOT GLOB '*[^a-z0-9-]*')),
  profile TEXT CHECK (profile IS NULL OR length(profile) <= 128),
  recording_id TEXT,
  -- Use and removal.
  last_used_at INTEGER,
  use_count INTEGER NOT NULL DEFAULT 0 CHECK (use_count >= 0),
  stale_at INTEGER,
  removed_at INTEGER,
  status_before_removed TEXT CHECK (status_before_removed IS NULL OR status_before_removed IN ('active','stale')),
  -- User feedback (ADR 0113): 1 = rated up or edited; released_at = the user released a quarantine.
  rating INTEGER NOT NULL DEFAULT 0 CHECK (rating IN (-1,0,1)),
  reviewed_at INTEGER,
  released_at INTEGER,
  CHECK ((layer = 'daily') = (note_day IS NOT NULL)),
  CHECK ((item_type = 'preference') = (layer = 'user')),
  CHECK (item_type <> 'genie_answer' OR layer = 'daily'),
  -- A user origin is always a pin: the brain cannot label its own write as the user's.
  CHECK (origin NOT IN ('pin','added') OR pinned = 1),
  CHECK ((status = 'removed') = (removed_at IS NOT NULL))
);

INSERT INTO memory_item_new (
  rid, item_id, mog_id, owner_principal, item_type, text, source, entities, entity_keys,
  attributes, confidence, status, superseded_by, resolved_by, pinned, sensitivity,
  source_session_ids, source_utterance_ids, source_exchange_id, workspace_host, workspace_id,
  space_id, mention_count, first_seen_at, last_seen_at, valid_until, embedding, embedding_model,
  embedding_dims, embedded_at, expires_at, created_at, updated_at, deleted_at, row_hlc,
  origin_device_id, server_seq,
  title, tags, origin, layer, note_day, slug, stale_at)
SELECT
  rid, item_id, mog_id, owner_principal,
  CASE WHEN item_type = 'entity' THEN 'fact' ELSE item_type END,
  text, source, entities, entity_keys,
  attributes, confidence,
  CASE WHEN item_type = 'entity' AND status <> 'quarantined' THEN 'stale' ELSE status END,
  superseded_by, resolved_by, pinned, sensitivity,
  source_session_ids, source_utterance_ids, source_exchange_id, workspace_host, workspace_id,
  space_id, mention_count, first_seen_at, last_seen_at, valid_until, embedding, embedding_model,
  embedding_dims, embedded_at, expires_at, created_at, updated_at, deleted_at, row_hlc,
  origin_device_id, server_seq,
  coalesce(nullif(trim(substr(trim(text), 1, 80)), ''), 'Untitled'),
  CASE WHEN item_type = 'entity' THEN '["legacy-entity"]' ELSE '[]' END,
  'legacy',
  'daily',
  strftime('%Y-%m-%d', first_seen_at / 1000, 'unixepoch', 'localtime'),
  NULL,
  CASE WHEN item_type = 'entity' AND status <> 'quarantined'
       THEN CAST(strftime('%s','now') AS INTEGER) * 1000 END
FROM memory_item;

-- The FTS table is external content on memory_item: drop it before the old table goes.
DROP TABLE memory_item_fts;
-- This also drops the 0001 indexes and triggers of memory_item.
DROP TABLE memory_item;
ALTER TABLE memory_item_new RENAME TO memory_item;

-- The 0001 indexes, verbatim, plus one address per item (ADR 0112).
CREATE INDEX memory_item_mog_recent ON memory_item (mog_id, last_seen_at DESC) WHERE deleted_at IS NULL AND status = 'active';
CREATE INDEX memory_item_mog_type ON memory_item (mog_id, item_type) WHERE deleted_at IS NULL;
CREATE INDEX memory_item_seq ON memory_item (mog_id, server_seq);
CREATE INDEX memory_item_embed_pending ON memory_item (mog_id) WHERE embedding IS NULL AND deleted_at IS NULL;
CREATE UNIQUE INDEX memory_item_addr ON memory_item (mog_id, layer, coalesce(note_day, ''), slug)
  WHERE slug IS NOT NULL AND deleted_at IS NULL;

-- FTS5 on the title, the summary and the body (MEM-08d).
CREATE VIRTUAL TABLE memory_item_fts USING fts5(
  title, text, body_md, mog_id UNINDEXED, content='memory_item', content_rowid='rid', tokenize='porter unicode61');
INSERT INTO memory_item_fts (memory_item_fts) VALUES ('rebuild');

CREATE TRIGGER memory_item_fts_ai AFTER INSERT ON memory_item BEGIN
  INSERT INTO memory_item_fts (rowid, title, text, body_md, mog_id)
    VALUES (new.rid, new.title, new.text, new.body_md, new.mog_id);
END;
CREATE TRIGGER memory_item_fts_ad AFTER DELETE ON memory_item BEGIN
  INSERT INTO memory_item_fts (memory_item_fts, rowid, title, text, body_md, mog_id)
    VALUES ('delete', old.rid, old.title, old.text, old.body_md, old.mog_id);
END;
CREATE TRIGGER memory_item_fts_au AFTER UPDATE OF title, text, body_md, mog_id ON memory_item BEGIN
  INSERT INTO memory_item_fts (memory_item_fts, rowid, title, text, body_md, mog_id)
    VALUES ('delete', old.rid, old.title, old.text, old.body_md, old.mog_id);
  INSERT INTO memory_item_fts (rowid, title, text, body_md, mog_id)
    VALUES (new.rid, new.title, new.text, new.body_md, new.mog_id);
END;

CREATE TRIGGER memory_item_vec_ad AFTER DELETE ON memory_item BEGIN
  DELETE FROM memory_item_vec_256 WHERE rowid = old.rid;
  DELETE FROM memory_item_vec_1024 WHERE rowid = old.rid;
END;
-- An edit clears the embedding (ADR 0113): the stale int8 index row goes with it.
CREATE TRIGGER memory_item_vec_au AFTER UPDATE OF embedding ON memory_item
WHEN new.embedding IS NULL AND old.embedding IS NOT NULL BEGIN
  DELETE FROM memory_item_vec_256 WHERE rowid = old.rid;
  DELETE FROM memory_item_vec_1024 WHERE rowid = old.rid;
END;
