-- Migration 0001: the local mirror of requirements Appendix A (B2), for the MVP memory store (#35).
-- Forward-only (MEM-35a). The runner records it in schema_migrations; there is no down migration.
--
-- Mapping from the Lakebase DDL (Appendix A, B2): uuid -> TEXT, timestamptz -> INTEGER epoch ms,
-- jsonb and arrays -> JSON TEXT, boolean -> INTEGER 0/1, tsvector + GIN -> FTS5, pgvector + HNSW ->
-- sqlite-vec vec0 (int8 index) with an f16 BLOB cache on memory_item (SP-21, MEM-21a).
--
-- Binding differences from Appendix A (ADR 0033):
-- * U9: Bottle deletes no user content on its own. So the local mirror has no *_retention_days
--   columns, no expiry indexes (except consent_expiry, which drives the 13-month re-prompt and
--   deletes nothing), and no `CHECK (pinned OR expires_at IS NOT NULL)`. expires_at stays as a
--   nullable mirror column for the v1 sync; no local job reads it. Only Settings -> Wipe deletes.
-- * MEM-01: every table carries mog_id, and every explicit index starts with mog_id.
--   consent_record gets a nullable mog_id (NULL = a device-wide notice consent).
-- * genie_exchange keeps final_answer_md locally (B2, D60); the Lakebase table does not.
-- * Raw audio has no column anywhere. No table has a token, secret or credential column.
-- * Triggers only keep the derived FTS5 and vec0 tables in step with their content rows.

-- ---- mog ---------------------------------------------------------------------------------
CREATE TABLE mog (
  mog_id TEXT PRIMARY KEY NOT NULL,
  owner_principal TEXT,
  display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 80),
  backend_kind TEXT NOT NULL CHECK (backend_kind IN ('genie_one_mcp','genie_agent','custom_agent')),
  default_profile TEXT,
  default_host TEXT,
  genie_target TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(genie_target)),
  memory_enabled INTEGER NOT NULL DEFAULT 1 CHECK (memory_enabled IN (0,1)),
  transcript_sync_optin INTEGER NOT NULL DEFAULT 0 CHECK (transcript_sync_optin IN (0,1)),
  answer_freshness_hours INTEGER NOT NULL DEFAULT 24 CHECK (answer_freshness_hours BETWEEN 0 AND 720),
  created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  deleted_at INTEGER,
  row_hlc INTEGER NOT NULL,
  origin_device_id TEXT NOT NULL,
  server_seq INTEGER NOT NULL DEFAULT 0
);

-- ---- session -----------------------------------------------------------------------------
CREATE TABLE session (
  session_id TEXT PRIMARY KEY NOT NULL,
  mog_id TEXT NOT NULL REFERENCES mog(mog_id) ON DELETE CASCADE,
  owner_principal TEXT,
  kind TEXT NOT NULL CHECK (kind IN ('push_to_talk','hands_free','meeting')),
  title TEXT CHECK (length(title) <= 200),
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  end_reason TEXT CHECK (end_reason IN ('manual','panic','mic_released','calendar_end','silence','sleep','lid','lock','logout',
                                        'disk_low','soft_cap','hard_cap','device_lost','permission','policy','indicator_failed','crash_recovered')),
  attestation_kind TEXT CHECK (attestation_kind IN ('only_me','others_agreed')),
  others_track INTEGER NOT NULL DEFAULT 0 CHECK (others_track IN (0,1)),
  workspace_host TEXT,
  workspace_id INTEGER,
  profile_name TEXT,
  utterance_count INTEGER NOT NULL DEFAULT 0,
  question_count INTEGER NOT NULL DEFAULT 0,
  summary TEXT CHECK (length(summary) <= 2000),
  transcript_synced INTEGER NOT NULL DEFAULT 0 CHECK (transcript_synced IN (0,1)),
  writer_watermark_seq INTEGER NOT NULL DEFAULT 0,
  expires_at INTEGER,
  created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  deleted_at INTEGER,
  row_hlc INTEGER NOT NULL,
  origin_device_id TEXT NOT NULL,
  server_seq INTEGER NOT NULL DEFAULT 0
);

-- ---- utterance (cleaned and raw text only; raw audio has no column anywhere) ----------------
CREATE TABLE utterance (
  rid INTEGER PRIMARY KEY,
  utterance_id TEXT NOT NULL UNIQUE,
  session_id TEXT NOT NULL REFERENCES session(session_id) ON DELETE CASCADE,
  mog_id TEXT NOT NULL REFERENCES mog(mog_id) ON DELETE CASCADE,
  owner_principal TEXT,
  seq INTEGER NOT NULL,
  speaker_track TEXT NOT NULL CHECK (speaker_track IN ('you','others')),
  started_at INTEGER NOT NULL,
  duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
  text_clean TEXT NOT NULL CHECK (length(text_clean) <= 8000),
  text_raw TEXT CHECK (length(text_raw) <= 8000),
  lang TEXT,
  route TEXT NOT NULL CHECK (route IN ('question','memory_question','context','command','noise','dictation')),
  stt_provider TEXT NOT NULL,
  stt_model TEXT,
  stt_runtime TEXT,
  expires_at INTEGER,
  created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  deleted_at INTEGER,
  row_hlc INTEGER NOT NULL,
  origin_device_id TEXT NOT NULL,
  server_seq INTEGER NOT NULL DEFAULT 0
);

-- ---- genie_exchange ----------------------------------------------------------------------
CREATE TABLE genie_exchange (
  exchange_id TEXT PRIMARY KEY NOT NULL,
  request_id TEXT NOT NULL UNIQUE,
  mog_id TEXT NOT NULL REFERENCES mog(mog_id) ON DELETE CASCADE,
  session_id TEXT REFERENCES session(session_id) ON DELETE CASCADE,
  owner_principal TEXT,
  backend_kind TEXT NOT NULL CHECK (backend_kind IN ('genie_one_mcp','genie_agent','custom_agent')),
  profile_name TEXT NOT NULL,
  workspace_host TEXT NOT NULL,
  workspace_id INTEGER,
  is_home_workspace INTEGER NOT NULL CHECK (is_home_workspace IN (0,1)),
  space_id TEXT,
  conversation_id TEXT,
  response_id TEXT,
  question_user TEXT CHECK (length(question_user) <= 1000),
  question_sent_sha256 TEXT NOT NULL,
  preamble_item_ids TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(preamble_item_ids)),
  status TEXT NOT NULL CHECK (status IN ('pending','completed','incomplete','failed','cancelled','abandoned','uncertain')),
  answer_summary TEXT CHECK (length(answer_summary) <= 600),
  key_numbers TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(key_numbers)),
  final_answer_md TEXT,
  topic_label TEXT,
  deep_link TEXT,
  query_item_ids TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(query_item_ids)),
  split_group_id TEXT,
  feedback TEXT CHECK (feedback IN ('POSITIVE','NEGATIVE','NONE')),
  feedback_at INTEGER,
  error_code TEXT,
  latency_ms INTEGER,
  asked_at INTEGER NOT NULL,
  completed_at INTEGER,
  expires_at INTEGER,
  created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  deleted_at INTEGER,
  row_hlc INTEGER NOT NULL,
  origin_device_id TEXT NOT NULL,
  server_seq INTEGER NOT NULL DEFAULT 0,
  CHECK (is_home_workspace = 1 OR (answer_summary IS NULL AND key_numbers = '[]' AND question_user IS NULL))
);

-- ---- memory_item -------------------------------------------------------------------------
CREATE TABLE memory_item (
  rid INTEGER PRIMARY KEY,
  item_id TEXT NOT NULL UNIQUE,
  mog_id TEXT NOT NULL REFERENCES mog(mog_id) ON DELETE CASCADE,
  owner_principal TEXT,
  item_type TEXT NOT NULL CHECK (item_type IN ('decision','fact','entity','open_question','genie_answer')),
  text TEXT NOT NULL CHECK (length(text) BETWEEN 1 AND 600),
  source TEXT NOT NULL CHECK (source IN ('you','others','genie','agent','confirmed')),
  entities TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(entities)),
  entity_keys TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(entity_keys)),
  attributes TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(attributes)),
  confidence REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','superseded','resolved','retracted','quarantined')),
  superseded_by TEXT,
  resolved_by TEXT,
  pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0,1)),
  sensitivity TEXT NOT NULL DEFAULT 'normal' CHECK (sensitivity IN ('normal','restricted')),
  source_session_ids TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(source_session_ids)),
  source_utterance_ids TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(source_utterance_ids)),
  source_exchange_id TEXT REFERENCES genie_exchange(exchange_id) ON DELETE CASCADE,
  workspace_host TEXT,
  workspace_id INTEGER,
  space_id TEXT,
  mention_count INTEGER NOT NULL DEFAULT 1,
  first_seen_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  valid_until INTEGER,
  -- f16 vector cache for the exact re-rank (SP-21). The int8 index lives in memory_item_vec_*.
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
  server_seq INTEGER NOT NULL DEFAULT 0
);

-- ---- consent_record (content-free; mog_id NULL = device-wide notice consent) ----------------
CREATE TABLE consent_record (
  consent_id TEXT PRIMARY KEY NOT NULL,
  mog_id TEXT,
  owner_principal TEXT,
  device_id TEXT NOT NULL,
  notice_version TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('notice','session')),
  session_id TEXT,
  attestation_kind TEXT,
  disclosure_copied INTEGER CHECK (disclosure_copied IN (0,1)),
  recorded_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  server_seq INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000)
);

-- ---- sync_state --------------------------------------------------------------------------
CREATE TABLE sync_state (
  device_id TEXT NOT NULL,
  mog_id TEXT NOT NULL REFERENCES mog(mog_id) ON DELETE CASCADE,
  owner_principal TEXT,
  device_name TEXT,
  app_version TEXT NOT NULL,
  last_pull_seq INTEGER NOT NULL DEFAULT 0,
  last_push_at INTEGER,
  last_retention_run_at INTEGER,
  local_schema_version INTEGER NOT NULL,
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  PRIMARY KEY (device_id, mog_id)
);

-- ---- forget_log --------------------------------------------------------------------------
CREATE TABLE forget_log (
  forget_id TEXT PRIMARY KEY NOT NULL,
  mog_id TEXT NOT NULL,
  owner_principal TEXT,
  scope TEXT NOT NULL CHECK (scope IN ('item','session','exchange','mog','all')),
  target_ids TEXT NOT NULL CHECK (json_valid(target_ids)),
  requested_at INTEGER NOT NULL,
  device_id TEXT NOT NULL,
  server_seq INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000)
);

-- ---- outbox: embedding requests and v1 sync operations (MEM-08f) ---------------------------
CREATE TABLE outbox (
  oid INTEGER PRIMARY KEY,
  mog_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('embed','sync')),
  op TEXT NOT NULL,
  ref_id TEXT NOT NULL,
  payload_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(payload_json)),
  created_at INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER NOT NULL,
  last_error_code TEXT
);

-- ---- kv: small per-MOG settings and state. A key that names a secret is refused. ------------
CREATE TABLE kv (
  mog_id TEXT NOT NULL,
  k TEXT NOT NULL CHECK (
    lower(k) NOT GLOB '*token*' AND lower(k) NOT GLOB '*secret*' AND lower(k) NOT GLOB '*password*'
    AND lower(k) NOT GLOB '*credential*' AND lower(k) NOT GLOB '*cookie*'),
  v TEXT NOT NULL,
  updated_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER) * 1000),
  PRIMARY KEY (mog_id, k)
);

-- ---- Indexes: Appendix A, with mog_id first (MEM-01). No purge-by-age index (U9). ----------
CREATE INDEX memory_item_mog_recent ON memory_item (mog_id, last_seen_at DESC) WHERE deleted_at IS NULL AND status = 'active';
CREATE INDEX memory_item_mog_type ON memory_item (mog_id, item_type) WHERE deleted_at IS NULL;
CREATE INDEX memory_item_seq ON memory_item (mog_id, server_seq);
CREATE INDEX memory_item_embed_pending ON memory_item (mog_id) WHERE embedding IS NULL AND deleted_at IS NULL;
CREATE INDEX session_mog ON session (mog_id, started_at DESC);
CREATE INDEX session_seq ON session (mog_id, server_seq);
CREATE UNIQUE INDEX utterance_session_seq ON utterance (mog_id, session_id, seq);
CREATE INDEX utterance_seq ON utterance (mog_id, server_seq);
CREATE INDEX ge_mog_time ON genie_exchange (mog_id, asked_at DESC);
CREATE INDEX ge_conv ON genie_exchange (mog_id, conversation_id);
CREATE INDEX ge_seq ON genie_exchange (mog_id, server_seq);
CREATE INDEX forget_seq ON forget_log (mog_id, server_seq);
CREATE INDEX consent_expiry ON consent_record (mog_id, expires_at);
CREATE INDEX outbox_due ON outbox (mog_id, next_attempt_at);

-- ---- FTS5 keyword search (MEM-08d): external content, so the text is stored once. ----------
CREATE VIRTUAL TABLE memory_item_fts USING fts5(
  text, mog_id UNINDEXED, content='memory_item', content_rowid='rid', tokenize='unicode61');
CREATE VIRTUAL TABLE utterance_fts USING fts5(
  text_clean, mog_id UNINDEXED, content='utterance', content_rowid='rid', tokenize='unicode61');

CREATE TRIGGER memory_item_fts_ai AFTER INSERT ON memory_item BEGIN
  INSERT INTO memory_item_fts (rowid, text, mog_id) VALUES (new.rid, new.text, new.mog_id);
END;
CREATE TRIGGER memory_item_fts_ad AFTER DELETE ON memory_item BEGIN
  INSERT INTO memory_item_fts (memory_item_fts, rowid, text, mog_id) VALUES ('delete', old.rid, old.text, old.mog_id);
END;
CREATE TRIGGER memory_item_fts_au AFTER UPDATE OF text, mog_id ON memory_item BEGIN
  INSERT INTO memory_item_fts (memory_item_fts, rowid, text, mog_id) VALUES ('delete', old.rid, old.text, old.mog_id);
  INSERT INTO memory_item_fts (rowid, text, mog_id) VALUES (new.rid, new.text, new.mog_id);
END;
CREATE TRIGGER utterance_fts_ai AFTER INSERT ON utterance BEGIN
  INSERT INTO utterance_fts (rowid, text_clean, mog_id) VALUES (new.rid, new.text_clean, new.mog_id);
END;
CREATE TRIGGER utterance_fts_ad AFTER DELETE ON utterance BEGIN
  INSERT INTO utterance_fts (utterance_fts, rowid, text_clean, mog_id) VALUES ('delete', old.rid, old.text_clean, old.mog_id);
END;
CREATE TRIGGER utterance_fts_au AFTER UPDATE OF text_clean, mog_id ON utterance BEGIN
  INSERT INTO utterance_fts (utterance_fts, rowid, text_clean, mog_id) VALUES ('delete', old.rid, old.text_clean, old.mog_id);
  INSERT INTO utterance_fts (rowid, text_clean, mog_id) VALUES (new.rid, new.text_clean, new.mog_id);
END;

-- ---- sqlite-vec int8 index (MEM-08e, MEM-21a, SP-21) -----------------------------------------
-- rowid = memory_item.rid. Partitioned by MOG and embedding model, so a KNN never crosses a MOG.
-- Values come from vec_codec::quantize_int8, which clamps to [-63, 63]: the NEON build of
-- sqlite-vec 0.1.9 returns a NULL L2 distance when a component difference leaves the i8 range.
CREATE VIRTUAL TABLE memory_item_vec_256 USING vec0(
  mog_id text partition key, embedding_model text partition key, embedding int8[256] distance_metric=l2);
CREATE VIRTUAL TABLE memory_item_vec_1024 USING vec0(
  mog_id text partition key, embedding_model text partition key, embedding int8[1024] distance_metric=l2);

CREATE TRIGGER memory_item_vec_ad AFTER DELETE ON memory_item BEGIN
  DELETE FROM memory_item_vec_256 WHERE rowid = old.rid;
  DELETE FROM memory_item_vec_1024 WHERE rowid = old.rid;
END;
