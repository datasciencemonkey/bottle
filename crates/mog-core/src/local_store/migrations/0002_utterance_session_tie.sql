-- Migration 0002 (#410, MEM-01, ADR 0051): an utterance belongs to the MOG of its session.
-- Forward only (MEM-35a). These triggers only refuse a write; they never delete a row (U9).
--
-- Appendix A asks for UNIQUE (session_id, seq). The 0001 index `utterance_session_seq` is
-- UNIQUE (mog_id, session_id, seq): every index starts with mog_id (MEM-01). With the tie below,
-- every utterance of a session has the session's mog_id, so that index makes (session_id, seq)
-- unique too. No new index is needed.

CREATE TRIGGER utterance_session_mog_bi BEFORE INSERT ON utterance
WHEN NOT EXISTS (
  SELECT 1 FROM session WHERE session_id = NEW.session_id AND mog_id = NEW.mog_id
)
BEGIN
  SELECT RAISE(ABORT, 'utterance_session_mog_mismatch');
END;

CREATE TRIGGER utterance_session_mog_bu BEFORE UPDATE OF session_id, mog_id ON utterance
WHEN NOT EXISTS (
  SELECT 1 FROM session WHERE session_id = NEW.session_id AND mog_id = NEW.mog_id
)
BEGIN
  SELECT RAISE(ABORT, 'utterance_session_mog_mismatch');
END;

CREATE TRIGGER session_mog_with_utterances_bu BEFORE UPDATE OF mog_id ON session
WHEN NEW.mog_id IS NOT OLD.mog_id
  AND EXISTS (SELECT 1 FROM utterance WHERE session_id = OLD.session_id)
BEGIN
  SELECT RAISE(ABORT, 'session_mog_has_utterances');
END;

-- `INSERT OR REPLACE` of a session deletes and inserts the row, so no UPDATE trigger runs. With
-- foreign keys off or deferred, it could move a session that has utterances to another MOG.
CREATE TRIGGER session_mog_with_utterances_bi BEFORE INSERT ON session
WHEN EXISTS (
  SELECT 1 FROM utterance WHERE session_id = NEW.session_id AND mog_id IS NOT NEW.mog_id
)
BEGIN
  SELECT RAISE(ABORT, 'session_mog_has_utterances');
END;
