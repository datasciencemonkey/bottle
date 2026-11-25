-- Migration 0005 (ADR 0125; owner authorization 2026-11-25 in chat): the local recording index.
-- Forward only (MEM-35a). No row is deleted (U9); `recording_delete` sets `hidden = 1` instead.
--
-- Metadata only: no transcript text, no notes text, no token and no file path. It lives in the
-- same SQLCipher file as Conversations, through the same store handle and the same key
-- (ADR 0105): one encrypted file, one key, one keychain item. No new keychain item and no
-- second connection.
--
-- This table carries no `mog_id`. A recording belongs to a Databricks profile (a workspace),
-- not a MOG: recording commands run before any MOG exists and the owner decided Bottle has one
-- MOG only (ADR 0117), so a recording is the app's own content, the same way the device-wide
-- `consent_record` rows are (ADR 0052, ADR 0071). It is not a MEM-01 memory table, so
-- `local_store::catalog::check_params` validates its statements with no `mogId` rule, the same
-- way the consent statements do; `local_store::catalog::STATEMENTS` and `mem_local_exec` never
-- see it (Rust-only, like `catalog::CONSENT`).
CREATE TABLE IF NOT EXISTS recording (
  name TEXT PRIMARY KEY NOT NULL,
  profile TEXT NOT NULL CHECK (length(profile) BETWEEN 1 AND 128),
  started_at_ms INTEGER NOT NULL,
  duration_s INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL
    CHECK (status IN ('recording','uploading','transcribing','writing_notes','done','failed')),
  error_kind TEXT,
  error_stage TEXT,
  on_device_only INTEGER NOT NULL DEFAULT 0 CHECK (on_device_only IN (0,1)),
  hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0,1)),
  updated_at_ms INTEGER NOT NULL
);

-- The hub list reads the newest rows of one profile, excluding what the user removed.
CREATE INDEX IF NOT EXISTS recording_profile_started
  ON recording (profile, started_at_ms DESC) WHERE hidden = 0;
