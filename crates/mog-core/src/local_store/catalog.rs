//! The fixed local statement catalog of R10 (#37, MEM-34a, MEM-34b, MEM-01; ADR 0054).
//!
//! The brain never sends SQL. It sends `mem_local_exec { stmtId, params }`, and the Rust core runs
//! only the statement with that ID from [`STATEMENTS`]:
//! - Each entry is one SQL statement with named parameters (`:mogId`). The parameter names are
//!   the JSON keys of `params`, and each has a type. [`validate`] refuses an unknown key, a
//!   missing required key and a value of the wrong type or size.
//! - Every entry filters by `mog_id` and passes `mog_guard::require_mog_id_filter` (MEM-01). An
//!   upsert's conflict path updates a row only if it belongs to the same MOG.
//! - [`Access::Brain`] entries are callable through `mem_local_exec`. [`Access::Core`] entries
//!   (Wipe) run only from Rust callers, for example R14 WipeService (U9: only Settings → Wipe
//!   deletes by age).
//! - A query answers `{ rows: [ {column: value} ] }`. A write answers `{ rows: [ {changes: n} ] }`.
//! - A [`Group`] is a fixed list of write entries that commit in one transaction (MEM-10a). It
//!   answers one `{changes: n}` row for each step (`mem.item.write`, #102, ADR 0068).
//!
//! The IDs follow the remote catalog of section 4.5 where the local schema supports them. The
//! feature issues own the logic that uses them: `mem.exchange.*` (#38), `mem.item.*` (#114),
//! `mem.recall.fts` (#199), `mem.forget.*` (#123), `mem.wipe.*` (#124), `mem.outbox.*` (#102).
//! The consent statements of T12 (#63, ADR 0052) are the separate entry set [`CONSENT`] (#431,
//! ADR 0071). Each has a [`Scope`]: the session statements filter by `mog_id`; the notice, the
//! device list and the purge are device-wide on purpose, each with its reason. They are
//! [`Access::Core`] and are not in [`STATEMENTS`], so `exec` and `mem_local_exec` never run them.
//! Only `mog_core::consent` runs them; the brain reaches the records through `consent_*` (#430).

use rusqlite::types::Value as SqlValue;
use serde_json::{Map, Value};

use super::mog_guard::require_mog_id_param;
use crate::error::{CoreError, CoreResult};

const ID: &str = "R10";

/// Is the statement a read or a write?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A `SELECT`: the reply holds the rows.
    Query,
    /// An `INSERT`, `UPDATE` or `DELETE`: the reply holds the number of changed rows.
    Write,
}

/// Who may run the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// The brain, through `mem_local_exec`, and Rust callers.
    Brain,
    /// Rust callers only (for example R14 WipeService). `mem_local_exec` refuses it.
    Core,
}

/// The type of one parameter. Every value binds as a SQLite parameter, never as SQL text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// A UUID string (`8-4-4-4-12` hex).
    Uuid,
    /// A string of at most `max` characters.
    Text {
        /// The most characters.
        max: usize,
    },
    /// A fixed code: 1-64 characters of `a-z`, `0-9`, `_` and `.` (an enum value, an op name, an
    /// error code). It can hold no content.
    Code,
    /// A whole number in `min..=max`.
    Int {
        /// The smallest value.
        min: i64,
        /// The largest value.
        max: i64,
    },
    /// A number.
    Real,
    /// `true` or `false`, stored as 1 or 0.
    Bool,
    /// Any JSON value, stored as JSON text of at most `max` bytes.
    Json {
        /// The most bytes of the JSON text.
        max: usize,
    },
    /// Free text for an FTS5 `MATCH`. The core keeps only its words and quotes each one, so the
    /// text can never be FTS5 syntax (MEM-34b). At least one word is required.
    FtsTerms,
    /// A list of at most `max_items` strings, each of at most `max_chars` characters, stored as
    /// JSON text. Any other shape (an object, a nested list, a number) is refused, so each tag
    /// reaches the secret check and the quarantine as a string.
    Tags {
        /// The most items.
        max_items: usize,
        /// The most characters of one item.
        max_chars: usize,
    },
    /// Set by the core, never by the caller: 1 if the text params named in `of` must be
    /// quarantined (SEC-24a, [`super::quarantine::is_quarantined`]), else 0. A caller that sends
    /// this key is refused, so no item write can skip the check (#114, ADR 0089).
    Quarantine {
        /// The names of the text params that the core checks, together.
        of: &'static [&'static str],
    },
    /// Set by the core, never by the caller (ADR 0113): if a text param named in `of` holds a
    /// token, a key or a password ([`super::secret_check::has_secret`]), the whole call is refused
    /// with [`CoreError::SecretRefused`] before any SQL runs. Otherwise it binds 0, and the SQL
    /// checks `= 0` as a second guard.
    Secret {
        /// The names of the text params that the core checks.
        of: &'static [&'static str],
    },
    /// Set by the core, never by the caller (ADR 0112): the slug of the first param in `of` that
    /// is not empty ([`super::memory_files::slugify`]). The SQL adds the clash suffix.
    Slug {
        /// The names of the text params, in order.
        of: &'static [&'static str],
    },
    /// Set by the core, never by the caller (ADR 0112): today's local calendar day
    /// `YYYY-MM-DD` on the core's own clock and device offset (or those of [`with_day_clock`]).
    /// It never reads a caller's time, so the brain cannot file an item under another day.
    Day,
}

impl ParamType {
    /// True for a param that the core computes and a caller may never send.
    pub fn is_core_computed(self) -> bool {
        matches!(
            self,
            ParamType::Quarantine { .. }
                | ParamType::Secret { .. }
                | ParamType::Slug { .. }
                | ParamType::Day
        )
    }
}

thread_local! {
    /// The `(offset east of UTC in seconds, now in epoch ms)` of [`with_day_clock`] on this
    /// thread, if any.
    static DAY_CLOCK: std::cell::Cell<Option<(i32, i64)>> = const { std::cell::Cell::new(None) };
}

/// Restores the clock of the thread when it drops, also on a panic.
struct DayClockGuard(Option<(i32, i64)>);

impl Drop for DayClockGuard {
    fn drop(&mut self) {
        DAY_CLOCK.with(|c| c.set(self.0));
    }
}

/// Run `f` with the `Day` params computed at `now_ms` and `offset_s` seconds east of UTC,
/// instead of the wall clock and the device offset. For tests; it holds for this thread only,
/// and the old clock comes back when `f` returns or panics.
pub fn with_day_clock<T>(offset_s: i32, now_ms: i64, f: impl FnOnce() -> T) -> T {
    let _guard = DayClockGuard(DAY_CLOCK.with(|c| c.replace(Some((offset_s, now_ms)))));
    f()
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The core clock of a `Day` param: `(offset east of UTC in seconds, now in epoch ms)`.
fn day_clock() -> (i32, i64) {
    use chrono::{Offset, TimeZone};
    DAY_CLOCK.with(std::cell::Cell::get).unwrap_or_else(|| {
        let now = wall_now_ms();
        let offset = chrono::Local
            .timestamp_millis_opt(now)
            .single()
            .map_or(0, |t| t.offset().fix().local_minus_utc());
        (offset, now)
    })
}

/// The offset east of UTC, in seconds, that a `Day` param uses now: for the times of day in a
/// rendered note.
pub fn current_day_offset() -> i32 {
    day_clock().0
}

/// One named parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    /// The JSON key and the SQL name (`:<name>`).
    pub name: &'static str,
    /// The type.
    pub ty: ParamType,
    /// A required parameter must be present and not `null`. An optional one binds `NULL`.
    pub required: bool,
}

/// One fixed statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stmt {
    /// The catalog ID, for example `mem.item.upsert`.
    pub id: &'static str,
    /// One SQL statement with `:name` parameters only.
    pub sql: &'static str,
    /// The parameters, one for each `:name` in `sql`.
    pub params: &'static [Param],
    /// Read or write.
    pub kind: Kind,
    /// Who may run it.
    pub access: Access,
}

const fn req(name: &'static str, ty: ParamType) -> Param {
    Param {
        name,
        ty,
        required: true,
    }
}

const fn opt(name: &'static str, ty: ParamType) -> Param {
    Param {
        name,
        ty,
        required: false,
    }
}

const UUID: ParamType = ParamType::Uuid;
const CODE: ParamType = ParamType::Code;
const REAL: ParamType = ParamType::Real;
const BOOL: ParamType = ParamType::Bool;
/// A time in epoch milliseconds, or an HLC value.
const MS: ParamType = ParamType::Int {
    min: 0,
    max: i64::MAX,
};
/// A row count for `LIMIT`.
const LIMIT: ParamType = ParamType::Int { min: 1, max: 500 };
const fn text(max: usize) -> ParamType {
    ParamType::Text { max }
}
const fn json(max: usize) -> ParamType {
    ParamType::Json { max }
}

const MOG_ID: Param = req("mogId", UUID);
const DEVICE_ID: Param = req("deviceId", text(64));
const ROW_HLC: Param = req("rowHlc", MS);

// ---- mog (the parent row every other table references) -----------------------------------

const MOG_UPSERT: Stmt = Stmt {
    id: "mem.mog.upsert",
    sql: "INSERT INTO mog (mog_id, display_name, backend_kind, row_hlc, origin_device_id) \
          VALUES (:mogId, :displayName, :backendKind, :rowHlc, :deviceId) \
          ON CONFLICT (mog_id) DO UPDATE SET display_name = excluded.display_name, \
          backend_kind = excluded.backend_kind, row_hlc = excluded.row_hlc, \
          updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000",
    params: &[
        MOG_ID,
        req("displayName", text(80)),
        req("backendKind", CODE),
        ROW_HLC,
        DEVICE_ID,
    ],
    kind: Kind::Write,
    access: Access::Brain,
};

// ---- genie_exchange (#38, MEM-25a, MEM-26; ADR 0061) --------------------------------------
//
// The upsert keeps the answer parts once they are written: a later write for the same request
// with no answer (a late `pending` event, a resume) never clears `final_answer_md`, the summary,
// the key numbers or the deep link (MEM-26, U9), and never moves a finished status back to
// `pending` or `uncertain`. No statement sets `expires_at`: an exchange has
// no TTL. Only Forget (one exchange, a user action) and Wipe (R14, Core) delete one. The hub
// Conversations delete (`mem.exchange.delete`, ADR 0121, local only) leaves a tombstone, and the
// upsert never brings a tombstone back (it keeps `deleted_at` and the erased text).

const EXCHANGE_UPSERT: Stmt = Stmt {
    id: "mem.exchange.upsert",
    sql: "INSERT INTO genie_exchange (exchange_id, request_id, mog_id, session_id, backend_kind, \
          profile_name, workspace_host, workspace_id, is_home_workspace, space_id, \
          conversation_id, response_id, question_user, question_sent_sha256, status, \
          answer_summary, key_numbers, final_answer_md, deep_link, split_group_id, asked_at, \
          completed_at, row_hlc, origin_device_id) \
          VALUES (:exchangeId, :requestId, :mogId, :sessionId, :backendKind, :profileName, \
          :workspaceHost, :workspaceId, :homeWorkspace, :spaceId, :conversationId, :responseId, \
          :questionUser, :questionSentSha256, :status, :answerSummary, \
          coalesce(:keyNumbers, '[]'), :finalAnswerMd, :deepLink, :splitGroupId, :askedAt, \
          :completedAt, :rowHlc, :deviceId) \
          ON CONFLICT (request_id) DO UPDATE SET status = CASE \
          WHEN excluded.status IN ('pending','uncertain') \
          AND genie_exchange.status NOT IN ('pending','uncertain') \
          THEN genie_exchange.status ELSE excluded.status END, \
          conversation_id = coalesce(excluded.conversation_id, genie_exchange.conversation_id), \
          response_id = coalesce(excluded.response_id, genie_exchange.response_id), \
          answer_summary = coalesce(excluded.answer_summary, genie_exchange.answer_summary), \
          key_numbers = CASE excluded.key_numbers WHEN '[]' THEN genie_exchange.key_numbers \
          ELSE excluded.key_numbers END, \
          final_answer_md = coalesce(excluded.final_answer_md, genie_exchange.final_answer_md), \
          deep_link = coalesce(excluded.deep_link, genie_exchange.deep_link), \
          session_id = coalesce(genie_exchange.session_id, excluded.session_id), \
          completed_at = coalesce(excluded.completed_at, genie_exchange.completed_at), \
          row_hlc = max(genie_exchange.row_hlc, excluded.row_hlc), \
          updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000 \
          WHERE genie_exchange.mog_id = excluded.mog_id \
          AND genie_exchange.deleted_at IS NULL",
    params: &[
        req("exchangeId", UUID),
        req("requestId", UUID),
        MOG_ID,
        opt("sessionId", UUID),
        req("backendKind", CODE),
        req("profileName", text(128)),
        req("workspaceHost", text(255)),
        opt("workspaceId", MS),
        req("homeWorkspace", BOOL),
        opt("spaceId", text(128)),
        opt("conversationId", text(128)),
        opt("responseId", text(128)),
        opt("questionUser", text(1000)),
        req("questionSentSha256", text(64)),
        req("status", CODE),
        opt("answerSummary", text(600)),
        opt("keyNumbers", json(16_384)),
        opt("finalAnswerMd", text(200_000)),
        opt("deepLink", text(2048)),
        opt("splitGroupId", UUID),
        req("askedAt", MS),
        opt("completedAt", MS),
        ROW_HLC,
        DEVICE_ID,
    ],
    kind: Kind::Write,
    access: Access::Brain,
};

const EXCHANGE_GET: Stmt = Stmt {
    id: "mem.exchange.get",
    sql: "SELECT exchange_id AS exchangeId, request_id AS requestId, status, \
          workspace_host AS workspaceHost, workspace_id AS workspaceId, space_id AS spaceId, \
          conversation_id AS conversationId, answer_summary AS answerSummary, \
          key_numbers AS keyNumbers, final_answer_md AS finalAnswerMd, deep_link AS deepLink, \
          is_home_workspace AS homeWorkspace, asked_at AS askedAt, completed_at AS completedAt, \
          session_id AS sessionId, split_group_id AS splitGroupId, \
          question_user AS questionUser \
          FROM genie_exchange \
          WHERE mog_id = :mogId AND request_id = :requestId AND deleted_at IS NULL",
    params: &[MOG_ID, req("requestId", UUID)],
    kind: Kind::Query,
    access: Access::Brain,
};

/// The newest exchanges of one MOG, newest first, for the hub Conversations list (#103). It is a
/// read only, like `mem.exchange.get`, and it never lists a forgotten (soft-deleted) row. The
/// brain reads it and sends the rows to the hub; the hub view never runs it. `conversationId`
/// (genie-followup, #145) lets the hub group exchanges into threads and send a follow-up on the
/// same Genie conversation.
const EXCHANGE_RECENT: Stmt = Stmt {
    id: "mem.exchange.recent",
    sql: "SELECT request_id AS requestId, status, profile_name AS profileName, \
          workspace_host AS workspaceHost, question_user AS questionUser, \
          key_numbers AS keyNumbers, final_answer_md AS finalAnswerMd, \
          deep_link AS deepLink, conversation_id AS conversationId, \
          backend_kind AS backendKind, split_group_id AS splitGroupId, \
          asked_at AS askedAt, completed_at AS completedAt \
          FROM genie_exchange \
          WHERE mog_id = :mogId AND deleted_at IS NULL \
          ORDER BY asked_at DESC, exchange_id DESC LIMIT :limit",
    params: &[MOG_ID, req("limit", LIMIT)],
    kind: Kind::Query,
    access: Access::Brain,
};

/// Delete the named exchanges of one MOG, for the hub Conversations "Delete" (ADR 0121). It is a
/// tombstone, not a row delete: it sets `deleted_at`, erases the question and answer text and
/// keeps the ids, so a later Wipe or Forget with "Also delete Genie conversations" still reads
/// the `conversation_id` and removes the Genie conversation. Wipe and Forget hard-delete the row
/// later. It is a local delete only: it writes no `forget_log` row and calls no remote service.
/// The brain sends 1 to 200 request ids; `Tags` checks the list is at most 200 strings of at
/// most 36 characters. A value that is no request id matches no row, and a row that is already
/// a tombstone matches none. A citing `memory_item` keeps its row. The correlated `EXISTS`
/// repeats the `mog_id` filter, so the MEM-01 guard sees one in each `SELECT`.
const EXCHANGE_DELETE: Stmt = Stmt {
    id: "mem.exchange.delete",
    sql: "UPDATE genie_exchange SET deleted_at = :now, question_user = NULL, \
          answer_summary = NULL, final_answer_md = NULL, key_numbers = '[]', \
          updated_at = :now \
          WHERE mog_id = :mogId AND deleted_at IS NULL AND EXISTS \
          (SELECT 1 FROM json_each(:requestIds) j \
          WHERE genie_exchange.mog_id = :mogId AND j.value = genie_exchange.request_id)",
    params: &[
        MOG_ID,
        req("now", MS),
        req(
            "requestIds",
            ParamType::Tags {
                max_items: 200,
                max_chars: 36,
            },
        ),
    ],
    kind: Kind::Write,
    access: Access::Brain,
};

/// The memory items that cite one exchange. `memory_item.source_exchange_id` is `SET NULL`, so
/// Forget must find these before it deletes the exchange (MEM-32a); the `mem.forget.session`
/// group deletes them first.
const EXCHANGE_DEPENDENTS: Stmt = Stmt {
    id: "mem.exchange.dependents",
    sql: "SELECT item_id AS itemId FROM memory_item \
          WHERE mog_id = :mogId AND source_exchange_id = :exchangeId ORDER BY rid",
    params: &[MOG_ID, req("exchangeId", UUID)],
    kind: Kind::Query,
    access: Access::Brain,
};

/// The exchanges of one session, for Forget by session (MEM-32a) and "Delete session" (SEC-14a).
/// `genie_exchange.session_id` is `SET NULL`, so the caller reads these first.
const EXCHANGE_BY_SESSION: Stmt = Stmt {
    id: "mem.exchange.by_session",
    sql: "SELECT exchange_id AS exchangeId, request_id AS requestId FROM genie_exchange \
          WHERE mog_id = :mogId AND session_id = :sessionId ORDER BY asked_at, exchange_id",
    params: &[MOG_ID, req("sessionId", UUID)],
    kind: Kind::Query,
    access: Access::Brain,
};

// ---- memory_item (#114) and recall (#199) --------------------------------------------------
//
// #114 (ADR 0089): the item write keeps its provenance (MEM-31: source, workspace, the session,
// utterance and exchange links) and sets `status` at write time (SEC-24a). The status is
// `quarantined` if the core's check of `:text` (`textQuarantine`) or the brain's own check
// (`quarantined`) says so, else `active`. A later mention never lowers a quarantine, and adds its
// session and utterance links to the ones that are there.

/// The provenance and quarantine params of an item write.
const SOURCE_SESSION_IDS: Param = opt("sourceSessionIds", json(8_192));
const SOURCE_UTTERANCE_IDS: Param = opt("sourceUtteranceIds", json(65_536));
const QUARANTINED: Param = opt("quarantined", BOOL);
/// The three text fields of a write, as the core checks them (ADR 0113).
const WRITE_TEXTS: &[&str] = &["title", "text", "body"];
/// The tags of a memory: at most 8, each at most 64 characters (migration 0003 caps the count).
const TAGS: ParamType = ParamType::Tags {
    max_items: 8,
    max_chars: 64,
};
/// The fields that the secret check and the quarantine read: the texts and each tag string
/// (security review M2).
const CHECKED_TEXTS: &[&str] = &["title", "text", "body", "tags"];
const TEXT_QUARANTINE: Param = req(
    "textQuarantine",
    ParamType::Quarantine { of: CHECKED_TEXTS },
);
const SECRET_CHECK: Param = req("secretCheck", ParamType::Secret { of: CHECKED_TEXTS });
const SLUG: Param = req("slug", ParamType::Slug { of: WRITE_TEXTS });
const NOTE_DAY: Param = req("noteDay", ParamType::Day);

/// Rust-only (#102 note, MEM-10a): the brain writes an item only with `mem.item.write`, which runs
/// this step and the embed job in one transaction. So no brain item write skips the outbox.
///
/// Slice 1 (ADR 0112, ADR 0113): each new item goes into today's daily note of its MOG. `text`
/// (the summary) or `body` is required: the title falls back to the start of the text, the
/// summary to the start of the body. A pin is rated 1 and reviewed at once (a user action). The
/// slug gets `-<first 6 hex of the item id>` when another live item of the same note has it.
/// The conflict path never rewrites the text, the title or the body of a pinned or reviewed row:
/// the user's words win over a later automatic mention.
const ITEM_UPSERT: Stmt = Stmt {
    id: "mem.item.upsert",
    sql: "INSERT INTO memory_item (item_id, mog_id, item_type, text, source, confidence, status, \
          pinned, sensitivity, source_session_ids, source_utterance_ids, workspace_host, \
          workspace_id, space_id, source_exchange_id, first_seen_at, last_seen_at, row_hlc, \
          origin_device_id, title, body_md, tags, origin, layer, note_day, slug, rating, \
          reviewed_at, profile, updated_at) \
          SELECT :itemId, :mogId, :itemType, coalesce(:text, substr(:body, 1, 600)), :source, \
          :confidence, \
          CASE WHEN max(:textQuarantine, coalesce(:quarantined, 0)) = 1 \
          THEN 'quarantined' ELSE 'active' END, \
          coalesce(:pinned, 0), coalesce(:sensitivity, 'normal'), \
          coalesce(:sourceSessionIds, '[]'), coalesce(:sourceUtteranceIds, '[]'), \
          :workspaceHost, :workspaceId, :spaceId, :sourceExchangeId, :seenAt, :seenAt, :rowHlc, \
          :deviceId, coalesce(nullif(trim(:title), ''), substr(coalesce(:text, :body), 1, 80)), \
          coalesce(:body, ''), coalesce(:tags, '[]'), \
          coalesce(:origin, CASE WHEN :source = 'agent' THEN 'agent' ELSE 'legacy' END), \
          'daily', :noteDay, \
          CASE WHEN NOT EXISTS (SELECT 1 FROM memory_item c WHERE c.mog_id = :mogId \
          AND c.layer = 'daily' AND c.note_day = :noteDay AND c.slug = :slug \
          AND c.item_id <> :itemId AND c.deleted_at IS NULL) THEN :slug \
          WHEN NOT EXISTS (SELECT 1 FROM memory_item d WHERE d.mog_id = :mogId \
          AND d.layer = 'daily' AND d.note_day = :noteDay \
          AND d.slug = :slug || '-' || lower(substr(replace(:itemId, '-', ''), 1, 6)) \
          AND d.item_id <> :itemId AND d.deleted_at IS NULL) \
          THEN :slug || '-' || lower(substr(replace(:itemId, '-', ''), 1, 6)) \
          ELSE rtrim(substr(:slug, 1, 39), '-') || '-' || lower(replace(:itemId, '-', '')) END, \
          coalesce(:pinned, 0), CASE WHEN coalesce(:pinned, 0) = 1 THEN :seenAt END, :profile, \
          CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER) \
          FROM mog WHERE mog_id = :mogId AND :secretCheck = 0 \
          AND coalesce(:text, :body) IS NOT NULL \
          ON CONFLICT (item_id) DO UPDATE SET \
          text = CASE WHEN max(memory_item.pinned, memory_item.reviewed_at IS NOT NULL, \
          memory_item.status = 'quarantined') = 1 \
          THEN memory_item.text ELSE excluded.text END, \
          title = CASE WHEN max(memory_item.pinned, memory_item.reviewed_at IS NOT NULL, \
          memory_item.status = 'quarantined') = 1 \
          THEN memory_item.title ELSE excluded.title END, \
          body_md = CASE WHEN max(memory_item.pinned, memory_item.reviewed_at IS NOT NULL, \
          memory_item.status = 'quarantined') = 1 \
          THEN memory_item.body_md ELSE excluded.body_md END, \
          confidence = max(memory_item.confidence, excluded.confidence), \
          pinned = max(memory_item.pinned, excluded.pinned), \
          rating = max(memory_item.rating, excluded.rating), \
          reviewed_at = coalesce(memory_item.reviewed_at, excluded.reviewed_at), \
          status = CASE WHEN excluded.status = 'quarantined' AND memory_item.pinned = 0 \
          AND memory_item.reviewed_at IS NULL THEN 'quarantined' ELSE memory_item.status END, \
          source_session_ids = (SELECT json_group_array(value) FROM (SELECT value \
          FROM json_each(memory_item.source_session_ids) UNION \
          SELECT value FROM json_each(excluded.source_session_ids))), \
          source_utterance_ids = (SELECT json_group_array(value) FROM (SELECT value \
          FROM json_each(memory_item.source_utterance_ids) UNION \
          SELECT value FROM json_each(excluded.source_utterance_ids))), \
          source_exchange_id = coalesce(memory_item.source_exchange_id, \
          excluded.source_exchange_id), \
          last_seen_at = max(memory_item.last_seen_at, excluded.last_seen_at), \
          mention_count = memory_item.mention_count + 1, row_hlc = excluded.row_hlc, \
          updated_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER) \
          WHERE memory_item.mog_id = excluded.mog_id",
    params: &[
        req("itemId", UUID),
        MOG_ID,
        req("itemType", CODE),
        opt("text", text(600)),
        opt("title", text(80)),
        opt("body", text(4_000)),
        opt("tags", TAGS),
        opt("origin", CODE),
        opt("profile", text(128)),
        req("source", CODE),
        req("confidence", REAL),
        opt("pinned", BOOL),
        opt("sensitivity", CODE),
        opt("workspaceHost", text(255)),
        opt("workspaceId", MS),
        opt("spaceId", text(128)),
        opt("sourceExchangeId", UUID),
        SOURCE_SESSION_IDS,
        SOURCE_UTTERANCE_IDS,
        QUARANTINED,
        TEXT_QUARANTINE,
        SECRET_CHECK,
        SLUG,
        NOTE_DAY,
        req("seenAt", MS),
        ROW_HLC,
        DEVICE_ID,
    ],
    kind: Kind::Write,
    access: Access::Core,
};

const ITEM_GET: Stmt = Stmt {
    id: "mem.item.get",
    sql: "SELECT item_id AS itemId, item_type AS type, title, text, body_md AS bodyMd, tags, \
          origin, layer, note_day AS noteDay, source, confidence, status, pinned, rating, \
          reviewed_at AS reviewedAt, released_at AS releasedAt, \
          sensitivity, workspace_host AS workspaceHost, workspace_id AS workspaceId, \
          space_id AS spaceId, mention_count AS mentionCount, first_seen_at AS firstSeenAt, \
          last_seen_at AS lastSeenAt, source_session_ids AS sourceSessionIds, \
          source_utterance_ids AS sourceUtteranceIds, source_exchange_id AS sourceExchangeId \
          FROM memory_item WHERE mog_id = :mogId AND item_id = :itemId AND deleted_at IS NULL",
    params: &[MOG_ID, req("itemId", UUID)],
    kind: Kind::Query,
    access: Access::Brain,
};

/// The columns of a memory file row (`memory_files::MemoryRow::from_json`).
macro_rules! file_row_cols {
    () => {
        "SELECT item_id AS itemId, item_type AS kind, layer, note_day AS noteDay, title, text, \
         body_md AS bodyMd, slug, status, pinned, source, origin, tags, confidence, \
         workspace_host AS workspaceHost, workspace_id AS workspaceId, profile, \
         source_exchange_id AS exchangeId, recording_id AS recordingId, \
         first_seen_at AS createdAt, updated_at AS updatedAt, last_used_at AS lastUsedAt, \
         rating, reviewed_at AS reviewedAt, released_at AS releasedAt FROM memory_item "
    };
}

/// Rust-only: the rows behind the `mem.item.list` handler (ADR 0112). Only items in use.
const ITEM_LIST_ROWS: Stmt = Stmt {
    id: "mem.item.list_rows",
    sql: concat!(
        file_row_cols!(),
        "WHERE mog_id = :mogId AND deleted_at IS NULL \
         AND status IN ('active','stale','quarantined') \
         ORDER BY pinned DESC, coalesce(last_used_at, updated_at) DESC, item_id LIMIT :limit"
    ),
    params: &[MOG_ID, req("limit", LIMIT)],
    kind: Kind::Query,
    access: Access::Core,
};

/// Rust-only: the rows of one file behind the `mem.file.get` handler (ADR 0112).
const ITEM_FILE_ROWS: Stmt = Stmt {
    id: "mem.item.file_rows",
    sql: concat!(
        file_row_cols!(),
        "WHERE mog_id = :mogId AND layer = :layer \
         AND coalesce(note_day, '') = coalesce(:noteDay, '') AND deleted_at IS NULL \
         AND status IN ('active','stale','quarantined') ORDER BY first_seen_at, item_id LIMIT 2000"
    ),
    params: &[MOG_ID, req("layer", CODE), opt("noteDay", text(10))],
    kind: Kind::Query,
    access: Access::Core,
};

/// Rust-only: the display name of one MOG, for the headings of its files.
const MOG_NAME: Stmt = Stmt {
    id: "mem.mog.name",
    sql: "SELECT display_name AS name FROM mog WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Query,
    access: Access::Core,
};

/// The wall clock of the Rate ticket's mint. `exec_ticketed` sets it from the redeemed ticket;
/// a caller that sends it is refused (security review M1).
const MINTED_AT: Param = req("mintedAt", MS);

/// Rust-only, only with a hub Rate ticket (ADR 0113): a thumbs up. Rating 1, reviewed now,
/// confidence at least 0.9, and the item in use: a stale item and a quarantined item become
/// `active`; a quarantined one gets `released_at` (SEC-24c). SQLite reads every `SET`
/// expression from the old row, so `released_at` sees the old status. Only a row that no write
/// touched after the click changes (`updated_at < mintedAt`, all writers set `updated_at` in ms
/// on the SQLite clock), so the brain cannot swap the text between the click and the release.
const ITEM_RATE: Stmt = Stmt {
    id: "mem.item.rate_row",
    sql: "UPDATE memory_item SET rating = 1, reviewed_at = :now, \
          confidence = max(confidence, 0.9), \
          released_at = CASE WHEN status = 'quarantined' THEN :now ELSE released_at END, \
          stale_at = NULL, status = 'active', row_hlc = :rowHlc, \
          updated_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER) \
          WHERE mog_id = :mogId AND item_id = :itemId AND deleted_at IS NULL \
          AND status IN ('active','stale','quarantined') AND updated_at < :mintedAt",
    params: &[
        MOG_ID,
        req("itemId", UUID),
        req("now", MS),
        ROW_HLC,
        MINTED_AT,
    ],
    kind: Kind::Write,
    access: Access::Core,
};

/// The text fields of an edit.
const EDIT_TEXTS: &[&str] = &["title", "body"];

/// Rust-only, only with a hub Edit ticket (ADR 0113): the user's edit of the kind, the title and
/// the body. The summary is the start of the body. The quarantine check runs again: instruction
/// text goes back into quarantine even on a released item; clean text makes the item `active`.
/// The edit rates the item 1, clears the embedding (the vec row goes with it, 0003 trigger) and
/// marks a source other than `you` as `confirmed`. A Genie answer stays one, no item becomes one,
/// and no edit makes a preference (USER.md is slice 2).
const ITEM_EDIT: Stmt = Stmt {
    id: "mem.item.edit_row",
    sql: "UPDATE memory_item SET item_type = :itemType, title = :title, body_md = :body, \
          text = substr(:body, 1, 600), \
          slug = CASE WHEN NOT EXISTS (SELECT 1 FROM memory_item c WHERE c.mog_id = :mogId \
          AND c.layer = memory_item.layer \
          AND coalesce(c.note_day, '') = coalesce(memory_item.note_day, '') \
          AND c.slug = :slug AND c.item_id <> :itemId AND c.deleted_at IS NULL) THEN :slug \
          WHEN NOT EXISTS (SELECT 1 FROM memory_item d WHERE d.mog_id = :mogId \
          AND d.layer = memory_item.layer \
          AND coalesce(d.note_day, '') = coalesce(memory_item.note_day, '') \
          AND d.slug = :slug || '-' || lower(substr(replace(:itemId, '-', ''), 1, 6)) \
          AND d.item_id <> :itemId AND d.deleted_at IS NULL) \
          THEN :slug || '-' || lower(substr(replace(:itemId, '-', ''), 1, 6)) \
          ELSE rtrim(substr(:slug, 1, 39), '-') || '-' || lower(replace(:itemId, '-', '')) END, \
          status = CASE WHEN :textQuarantine = 1 THEN 'quarantined' ELSE 'active' END, \
          released_at = NULL, stale_at = NULL, rating = 1, reviewed_at = :now, \
          confidence = max(confidence, 0.9), \
          source = CASE WHEN source = 'you' THEN 'you' ELSE 'confirmed' END, \
          embedding = NULL, embedding_model = NULL, embedding_dims = NULL, embedded_at = NULL, \
          row_hlc = :rowHlc, updated_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER) \
          WHERE mog_id = :mogId AND item_id = :itemId AND deleted_at IS NULL \
          AND status IN ('active','stale','quarantined') AND :secretCheck = 0 \
          AND trim(:title) <> '' AND trim(:body) <> '' AND :itemType <> 'preference' \
          AND (:itemType = 'genie_answer') = (item_type = 'genie_answer')",
    params: &[
        MOG_ID,
        req("itemId", UUID),
        req("itemType", CODE),
        req("title", text(80)),
        req("body", text(4_000)),
        req("textQuarantine", ParamType::Quarantine { of: EDIT_TEXTS }),
        req("secretCheck", ParamType::Secret { of: EDIT_TEXTS }),
        req("slug", ParamType::Slug { of: &["title"] }),
        req("now", MS),
        ROW_HLC,
    ],
    kind: Kind::Write,
    access: Access::Core,
};

/// The ids that `mem_local_exec` answers with a Rust handler over [`memory_files`] (ADR 0112),
/// not with one statement. Both are reads for the brain.
///
/// [`memory_files`]: super::memory_files
pub const HANDLERS: &[&str] = &["mem.item.list", "mem.file.get"];

const RECALL_FTS: Stmt = Stmt {
    id: "mem.recall.fts",
    sql: "SELECT m.item_id AS itemId, m.item_type AS type, m.text AS text, \
          m.workspace_id AS workspaceId, m.space_id AS spaceId, m.pinned AS pinned, \
          m.last_seen_at AS lastSeenAt, bm25(memory_item_fts) AS score \
          FROM memory_item_fts JOIN memory_item m ON m.rid = memory_item_fts.rowid \
          WHERE m.mog_id = :mogId AND memory_item_fts MATCH :query \
          AND m.deleted_at IS NULL AND m.status = 'active' \
          ORDER BY score LIMIT :limit",
    params: &[
        MOG_ID,
        req("query", ParamType::FtsTerms),
        req("limit", LIMIT),
    ],
    kind: Kind::Query,
    access: Access::Brain,
};

// ---- Forget (#123): a user action through T10 (MEM-32a, U9; ADR 0081) ----------------------
//
// Forget runs as one of three atomic groups below (`mem.forget.items`, `mem.forget.session`,
// `mem.forget.mog`). The steps are `Access::Core`, so the brain cannot run a partial Forget.
// The optional pointers `genie_exchange.session_id` and `memory_item.source_exchange_id` are
// `ON DELETE SET NULL` (ADR 0033). So each group deletes the citing items first, while those
// pointers still lead to their parents, then the embed jobs that point at no item, then the
// parents. FTS5 and vec0 rows go with their rows through the 0001 triggers. The `forget_log` row
// is the last step: a failed delete writes no log row. No step has an age rule (U9). Wipe (#124)
// can reuse the steps that do not name a session or item.

/// The Genie conversations of one session, for Forget (MEM-32c). T10 reads them before the
/// delete, because the delete removes the exchange rows that hold them. A conversation that an
/// exchange outside the session also uses is left out: that exchange keeps it.
const FORGET_SESSION_CONVERSATIONS: Stmt = Stmt {
    id: "mem.forget.session_conversations",
    sql: "SELECT DISTINCT backend_kind AS backendKind, profile_name AS profile, \
          workspace_host AS host, workspace_id AS workspaceId, space_id AS spaceId, \
          conversation_id AS conversationId FROM genie_exchange \
          WHERE mog_id = :mogId AND session_id = :sessionId AND conversation_id IS NOT NULL \
          AND conversation_id NOT IN (SELECT o.conversation_id FROM genie_exchange o \
          WHERE o.mog_id = :mogId AND o.session_id IS NOT :sessionId \
          AND o.conversation_id IS NOT NULL) \
          ORDER BY conversation_id, profile_name",
    params: &[MOG_ID, req("sessionId", UUID)],
    kind: Kind::Query,
    access: Access::Brain,
};

/// The Genie conversations of one MOG, for Forget of the MOG (MEM-32c).
const FORGET_MOG_CONVERSATIONS: Stmt = Stmt {
    id: "mem.forget.mog_conversations",
    sql: "SELECT DISTINCT backend_kind AS backendKind, profile_name AS profile, \
          workspace_host AS host, workspace_id AS workspaceId, space_id AS spaceId, \
          conversation_id AS conversationId FROM genie_exchange \
          WHERE mog_id = :mogId AND conversation_id IS NOT NULL \
          ORDER BY conversation_id, profile_name",
    params: &[MOG_ID],
    kind: Kind::Query,
    access: Access::Brain,
};

/// The items named in `itemIds` (a JSON array of item ids).
const FORGET_ITEM_ROWS: Stmt = Stmt {
    id: "mem.forget.item_rows",
    sql: "DELETE FROM memory_item WHERE mog_id = :mogId AND item_id IN \
          (SELECT m.item_id FROM memory_item m, json_each(:itemIds) j \
          WHERE m.mog_id = :mogId AND m.item_id = j.value)",
    params: &[MOG_ID, req("itemIds", json(65_536))],
    kind: Kind::Write,
    access: Access::Core,
};

/// The items that cite the session in `source_session_ids`.
const FORGET_SESSION_ITEMS_BY_SESSION: Stmt = Stmt {
    id: "mem.forget.session_items_by_session",
    sql: "DELETE FROM memory_item WHERE mog_id = :mogId AND item_id IN \
          (SELECT m.item_id FROM memory_item m, json_each(m.source_session_ids) j \
          WHERE m.mog_id = :mogId AND j.value = :sessionId)",
    params: &[MOG_ID, req("sessionId", UUID)],
    kind: Kind::Write,
    access: Access::Core,
};

/// The items that cite an utterance of the session in `source_utterance_ids`.
const FORGET_SESSION_ITEMS_BY_UTTERANCE: Stmt = Stmt {
    id: "mem.forget.session_items_by_utterance",
    sql: "DELETE FROM memory_item WHERE mog_id = :mogId AND item_id IN \
          (SELECT m.item_id FROM memory_item m, json_each(m.source_utterance_ids) j, utterance u \
          WHERE m.mog_id = :mogId AND u.mog_id = :mogId AND u.session_id = :sessionId \
          AND u.utterance_id = j.value)",
    params: &[MOG_ID, req("sessionId", UUID)],
    kind: Kind::Write,
    access: Access::Core,
};

/// The items that cite an exchange of the session in `source_exchange_id`. It runs before the
/// exchange delete, which would set the pointer to NULL.
const FORGET_SESSION_ITEMS_BY_EXCHANGE: Stmt = Stmt {
    id: "mem.forget.session_items_by_exchange",
    sql: "DELETE FROM memory_item WHERE mog_id = :mogId AND source_exchange_id IN \
          (SELECT exchange_id FROM genie_exchange \
          WHERE mog_id = :mogId AND session_id = :sessionId)",
    params: &[MOG_ID, req("sessionId", UUID)],
    kind: Kind::Write,
    access: Access::Core,
};

/// The embed jobs that point at no item of the MOG. A job holds no text (ADR 0068); this removes
/// the pointers that a Forget left behind.
const FORGET_EMBED_ORPHANS: Stmt = Stmt {
    id: "mem.forget.embed_orphans",
    sql: "DELETE FROM outbox WHERE mog_id = :mogId AND kind = 'embed' AND ref_id NOT IN \
          (SELECT item_id FROM memory_item WHERE mog_id = :mogId)",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_SESSION_EXCHANGES: Stmt = Stmt {
    id: "mem.forget.session_exchanges",
    sql: "DELETE FROM genie_exchange WHERE mog_id = :mogId AND session_id = :sessionId",
    params: &[MOG_ID, req("sessionId", UUID)],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_SESSION_UTTERANCES: Stmt = Stmt {
    id: "mem.forget.session_utterances",
    sql: "DELETE FROM utterance WHERE mog_id = :mogId AND session_id = :sessionId",
    params: &[MOG_ID, req("sessionId", UUID)],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_SESSION_ROW: Stmt = Stmt {
    id: "mem.forget.session_row",
    sql: "DELETE FROM session WHERE mog_id = :mogId AND session_id = :sessionId",
    params: &[MOG_ID, req("sessionId", UUID)],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_MOG_ITEMS: Stmt = Stmt {
    id: "mem.forget.mog_items",
    sql: "DELETE FROM memory_item WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

/// Every outbox row of the MOG: its embed jobs and any v1 `sync` row. They point only at rows of
/// the forgotten MOG. The v1 forget itself travels as the `forget_log` row (MEM-12b).
const FORGET_MOG_OUTBOX: Stmt = Stmt {
    id: "mem.forget.mog_outbox",
    sql: "DELETE FROM outbox WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_MOG_EXCHANGES: Stmt = Stmt {
    id: "mem.forget.mog_exchanges",
    sql: "DELETE FROM genie_exchange WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_MOG_UTTERANCES: Stmt = Stmt {
    id: "mem.forget.mog_utterances",
    sql: "DELETE FROM utterance WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_MOG_SESSIONS: Stmt = Stmt {
    id: "mem.forget.mog_sessions",
    sql: "DELETE FROM session WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_MOG_KV: Stmt = Stmt {
    id: "mem.forget.mog_kv",
    sql: "DELETE FROM kv WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_MOG_SYNC_STATE: Stmt = Stmt {
    id: "mem.forget.mog_sync_state",
    sql: "DELETE FROM sync_state WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

/// The MOG row itself. The caller registers it again (`mem.mog.upsert`) before a new write.
const FORGET_MOG_ROW: Stmt = Stmt {
    id: "mem.forget.mog_row",
    sql: "DELETE FROM mog WHERE mog_id = :mogId",
    params: &[MOG_ID],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_LOG_ITEMS: Stmt = Stmt {
    id: "mem.forget.log_items",
    sql: "INSERT INTO forget_log (forget_id, mog_id, scope, target_ids, requested_at, device_id) \
          VALUES (:forgetId, :mogId, 'item', :itemIds, :requestedAt, :deviceId)",
    params: &[
        req("forgetId", UUID),
        MOG_ID,
        req("itemIds", json(65_536)),
        req("requestedAt", MS),
        DEVICE_ID,
    ],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_LOG_SESSION: Stmt = Stmt {
    id: "mem.forget.log_session",
    sql: "INSERT INTO forget_log (forget_id, mog_id, scope, target_ids, requested_at, device_id) \
          VALUES (:forgetId, :mogId, 'session', json_array(:sessionId), :requestedAt, :deviceId)",
    params: &[
        req("forgetId", UUID),
        MOG_ID,
        req("sessionId", UUID),
        req("requestedAt", MS),
        DEVICE_ID,
    ],
    kind: Kind::Write,
    access: Access::Core,
};

const FORGET_LOG_MOG: Stmt = Stmt {
    id: "mem.forget.log_mog",
    sql: "INSERT INTO forget_log (forget_id, mog_id, scope, target_ids, requested_at, device_id) \
          VALUES (:forgetId, :mogId, 'mog', json_array(:mogId), :requestedAt, :deviceId)",
    params: &[
        req("forgetId", UUID),
        MOG_ID,
        req("requestedAt", MS),
        DEVICE_ID,
    ],
    kind: Kind::Write,
    access: Access::Core,
};

// ---- Wipe (#124): R14 WipeService only (U9, MEM-33a; ADR 0087) -----------------------------
//
// Only the user's Settings -> Wipe runs these, through R14 in Rust. Every statement and group is
// `Access::Core`, so `mem_local_exec` refuses them. Each one filters by `mog_id` (MEM-01); R14
// runs them for every MOG of the device in one transaction. There is no cascade (ADR 0033): each
// table goes by its own age column, citing rows before their parents, and a recent row stays
// when an old row that it points at goes. The age is the app-clock time of the row: an item's
// `first_seen_at`, an exchange's `asked_at` (ADR 0061) and an utterance's `started_at` (the
// transcripts and the dictation history). "Older than" is strict: a row at the cutoff stays.

const BEFORE: Param = req("before", MS);

const WIPE_ITEMS: Stmt = Stmt {
    id: "mem.wipe.items_before",
    sql: "DELETE FROM memory_item WHERE mog_id = :mogId AND first_seen_at < :before",
    params: &[MOG_ID, BEFORE],
    kind: Kind::Write,
    access: Access::Core,
};

const WIPE_EXCHANGES: Stmt = Stmt {
    id: "mem.wipe.exchanges_before",
    // The age of an exchange is the time the user asked (`asked_at`, the T10 clock, indexed by
    // `ge_mog_time`), not the local insert time (ADR 0061).
    sql: "DELETE FROM genie_exchange WHERE mog_id = :mogId AND asked_at < :before",
    params: &[MOG_ID, BEFORE],
    kind: Kind::Write,
    access: Access::Core,
};

/// Transcripts (raw and cleaned) and the dictation history (`route = 'dictation'`) by the time
/// the speech started. FTS5 rows go through the 0001 trigger.
const WIPE_UTTERANCES: Stmt = Stmt {
    id: "mem.wipe.utterances_before",
    sql: "DELETE FROM utterance WHERE mog_id = :mogId AND started_at < :before",
    params: &[MOG_ID, BEFORE],
    kind: Kind::Write,
    access: Access::Core,
};

/// An old session goes only when none of its utterances and none of its exchanges is left
/// (ADR 0033). So a kept exchange never loses its session pointer, and the RESTRICT link from
/// `utterance` never fails.
const WIPE_SESSIONS: Stmt = Stmt {
    id: "mem.wipe.sessions_before",
    sql: "DELETE FROM session WHERE mog_id = :mogId AND started_at < :before \
          AND NOT EXISTS (SELECT 1 FROM utterance u WHERE u.mog_id = :mogId \
          AND u.session_id = session.session_id) \
          AND NOT EXISTS (SELECT 1 FROM genie_exchange e WHERE e.mog_id = :mogId \
          AND e.session_id = session.session_id)",
    params: &[MOG_ID, BEFORE],
    kind: Kind::Write,
    access: Access::Core,
};

/// The Genie conversations of the exchanges that an age Wipe deletes, read before the delete
/// (9.2 "Also delete Genie conversations", G-49). A conversation that a kept exchange of the MOG
/// still uses is left out. Ids only: no question, summary or answer.
const WIPE_CONVERSATIONS: Stmt = Stmt {
    id: "mem.wipe.conversations_before",
    sql: "SELECT DISTINCT backend_kind AS backendKind, profile_name AS profile, \
          workspace_host AS host, workspace_id AS workspaceId, space_id AS spaceId, \
          conversation_id AS conversationId FROM genie_exchange \
          WHERE mog_id = :mogId AND asked_at < :before AND conversation_id IS NOT NULL \
          AND conversation_id NOT IN (SELECT o.conversation_id FROM genie_exchange o \
          WHERE o.mog_id = :mogId AND o.asked_at >= :before \
          AND o.conversation_id IS NOT NULL) \
          ORDER BY conversation_id, profile_name",
    params: &[MOG_ID, BEFORE],
    kind: Kind::Query,
    access: Access::Core,
};

// ---- Outbox: embedding requests and v1 sync operations (MEM-08f, #102) ----------------------

const OUTBOX_ENQUEUE: Stmt = Stmt {
    id: "mem.outbox.enqueue",
    sql: "INSERT INTO outbox (mog_id, kind, op, ref_id, payload_json, created_at, \
          next_attempt_at) \
          VALUES (:mogId, :kind, :op, :refId, coalesce(:payload, '{}'), :now, :now)",
    params: &[
        MOG_ID,
        req("kind", CODE),
        req("op", CODE),
        req("refId", text(128)),
        opt("payload", json(65_536)),
        req("now", MS),
    ],
    kind: Kind::Write,
    access: Access::Brain,
};

/// The due jobs of one queue kind, oldest first (FIFO). The kind is required, so the embed drain
/// never reads v1 `sync` rows and the other way round.
const OUTBOX_DUE: Stmt = Stmt {
    id: "mem.outbox.due",
    sql: "SELECT oid, kind, op, ref_id AS refId, payload_json AS payload, attempts, \
          next_attempt_at AS nextAttemptAt \
          FROM outbox WHERE mog_id = :mogId AND kind = :kind AND next_attempt_at <= :now \
          ORDER BY oid LIMIT :limit",
    params: &[
        MOG_ID,
        req("kind", CODE),
        req("now", MS),
        req("limit", LIMIT),
    ],
    kind: Kind::Query,
    access: Access::Brain,
};

const OUTBOX_RETRY: Stmt = Stmt {
    id: "mem.outbox.retry",
    sql: "UPDATE outbox SET attempts = attempts + 1, next_attempt_at = :nextAttemptAt, \
          last_error_code = :errorCode WHERE mog_id = :mogId AND oid = :oid",
    params: &[
        MOG_ID,
        req("oid", MS),
        req("nextAttemptAt", MS),
        opt("errorCode", CODE),
    ],
    kind: Kind::Write,
    access: Access::Brain,
};

const OUTBOX_DONE: Stmt = Stmt {
    id: "mem.outbox.done",
    sql: "DELETE FROM outbox WHERE mog_id = :mogId AND oid = :oid",
    params: &[MOG_ID, req("oid", MS)],
    kind: Kind::Write,
    access: Access::Brain,
};

/// The embed job of one item, as step 2 of `mem.item.write` (MEM-10a, MEM-19d). It adds a job only
/// if the item is in this MOG (MEM-01), is `active` (a quarantined item is never embedded, SEC-24a,
/// ADR 0089), has no embedding yet, and has no embed job waiting: so a
/// second mention adds no duplicate (SP-21). The job is a pointer (`ref_id` = the item id) and
/// holds no text. Rust only: the brain adds an embed job only with its item write.
/// TODO(#198): when the text of an embedded item changes, #198 clears its embedding and its
/// `vec0` row, so this step queues a new job. Until then the old vector stays.
const OUTBOX_ENQUEUE_ITEM_EMBED: Stmt = Stmt {
    id: "mem.outbox.enqueue_item_embed",
    sql: "INSERT INTO outbox (mog_id, kind, op, ref_id, created_at, next_attempt_at) \
          SELECT m.mog_id, 'embed', 'item.embed', m.item_id, :now, :now FROM memory_item m \
          WHERE m.mog_id = :mogId AND m.item_id = :itemId AND m.embedding IS NULL \
          AND m.deleted_at IS NULL AND m.status = 'active' AND NOT EXISTS (SELECT 1 FROM outbox o \
          WHERE o.mog_id = :mogId AND o.kind = 'embed' AND o.ref_id = :itemId)",
    params: &[MOG_ID, req("itemId", UUID), req("now", MS)],
    kind: Kind::Write,
    access: Access::Core,
};

// ---- Atomic groups: fixed statements that commit in one transaction (MEM-10a, ADR 0068) ------

/// A fixed list of catalog write statements that run in one SQLite transaction. The caller sends
/// one ID and one params object, as for a [`Stmt`]; each step binds the params it names. If any
/// step fails, or the process dies before the commit, no step is kept. The answer has one
/// `{changes: n}` row for each step, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    /// The catalog ID, for example `mem.item.write`.
    pub id: &'static str,
    /// The steps, in order. Each one is a [`Kind::Write`] entry of [`STATEMENTS`].
    pub steps: &'static [&'static Stmt],
    /// The params: the union of the step params, each one once.
    pub params: &'static [Param],
    /// Who may run it.
    pub access: Access,
}

/// Write a memory item and queue its embedding in one transaction (#102, MEM-10a, MEM-19d). The
/// params are those of `mem.item.upsert` plus `now`, the time of the outbox row.
const ITEM_WRITE: Group = Group {
    id: "mem.item.write",
    steps: &[&ITEM_UPSERT, &OUTBOX_ENQUEUE_ITEM_EMBED],
    params: &[
        req("itemId", UUID),
        MOG_ID,
        req("itemType", CODE),
        opt("text", text(600)),
        opt("title", text(80)),
        opt("body", text(4_000)),
        opt("tags", TAGS),
        opt("origin", CODE),
        opt("profile", text(128)),
        req("source", CODE),
        req("confidence", REAL),
        opt("pinned", BOOL),
        opt("sensitivity", CODE),
        opt("workspaceHost", text(255)),
        opt("workspaceId", MS),
        opt("spaceId", text(128)),
        opt("sourceExchangeId", UUID),
        SOURCE_SESSION_IDS,
        SOURCE_UTTERANCE_IDS,
        QUARANTINED,
        TEXT_QUARANTINE,
        SECRET_CHECK,
        SLUG,
        NOTE_DAY,
        req("seenAt", MS),
        ROW_HLC,
        DEVICE_ID,
        req("now", MS),
    ],
    access: Access::Brain,
};

/// A thumbs up and its embed job (a released or revived item is `active` now). Rust-only: the
/// host runs it only after it redeems a hub Rate ticket (`local_store::exec_ticketed`).
const ITEM_RATE_GROUP: Group = Group {
    id: "mem.item.rate",
    steps: &[&ITEM_RATE, &OUTBOX_ENQUEUE_ITEM_EMBED],
    params: &[
        MOG_ID,
        req("itemId", UUID),
        req("now", MS),
        ROW_HLC,
        MINTED_AT,
    ],
    access: Access::Core,
};

/// An edit and the new embed job of an `active` result. Rust-only: the host runs it only after
/// it redeems a hub Edit ticket.
const ITEM_EDIT_GROUP: Group = Group {
    id: "mem.item.edit",
    steps: &[&ITEM_EDIT, &OUTBOX_ENQUEUE_ITEM_EMBED],
    params: &[
        MOG_ID,
        req("itemId", UUID),
        req("itemType", CODE),
        req("title", text(80)),
        req("body", text(4_000)),
        req("textQuarantine", ParamType::Quarantine { of: EDIT_TEXTS }),
        req("secretCheck", ParamType::Secret { of: EDIT_TEXTS }),
        req("slug", ParamType::Slug { of: &["title"] }),
        req("now", MS),
        ROW_HLC,
    ],
    access: Access::Core,
};

/// The params of every Forget group: the log row's id, time and device.
const FORGET_ID: Param = req("forgetId", UUID);
const REQUESTED_AT: Param = req("requestedAt", MS);

/// Forget the items in `itemIds` with their embed jobs, vectors and FTS rows (MEM-32a).
const FORGET_ITEMS: Group = Group {
    id: "mem.forget.items",
    steps: &[&FORGET_ITEM_ROWS, &FORGET_EMBED_ORPHANS, &FORGET_LOG_ITEMS],
    params: &[
        MOG_ID,
        req("itemIds", json(65_536)),
        FORGET_ID,
        REQUESTED_AT,
        DEVICE_ID,
    ],
    access: Access::Brain,
};

/// Forget one session: the items that cite it, its exchanges, its utterances and the session
/// row (MEM-32a, AT-16). The citing items go first (ADR 0081).
const FORGET_SESSION: Group = Group {
    id: "mem.forget.session",
    steps: &[
        &FORGET_SESSION_ITEMS_BY_SESSION,
        &FORGET_SESSION_ITEMS_BY_UTTERANCE,
        &FORGET_SESSION_ITEMS_BY_EXCHANGE,
        &FORGET_EMBED_ORPHANS,
        &FORGET_SESSION_EXCHANGES,
        &FORGET_SESSION_UTTERANCES,
        &FORGET_SESSION_ROW,
        &FORGET_LOG_SESSION,
    ],
    params: &[
        MOG_ID,
        req("sessionId", UUID),
        FORGET_ID,
        REQUESTED_AT,
        DEVICE_ID,
    ],
    access: Access::Brain,
};

/// Forget one MOG: every row of the MOG except its consent records (content-free, SEC-15b
/// expiry) and its `forget_log` rows (MEM-32a, AT-16).
const FORGET_MOG: Group = Group {
    id: "mem.forget.mog",
    steps: &[
        &FORGET_MOG_ITEMS,
        &FORGET_MOG_OUTBOX,
        &FORGET_MOG_EXCHANGES,
        &FORGET_MOG_UTTERANCES,
        &FORGET_MOG_SESSIONS,
        &FORGET_MOG_KV,
        &FORGET_MOG_SYNC_STATE,
        &FORGET_MOG_ROW,
        &FORGET_LOG_MOG,
    ],
    params: &[MOG_ID, FORGET_ID, REQUESTED_AT, DEVICE_ID],
    access: Access::Brain,
};

/// Wipe the rows of one MOG that are older than `before` (MEM-33a, U9; ADR 0087). The items go
/// first, then the embed jobs that point at no item, then the exchanges, the utterances and the
/// sessions that have no row left. R14 runs it for each MOG in one transaction.
const WIPE_BEFORE: Group = Group {
    id: "mem.wipe.before",
    steps: &[
        &WIPE_ITEMS,
        &FORGET_EMBED_ORPHANS,
        &WIPE_EXCHANGES,
        &WIPE_UTTERANCES,
        &WIPE_SESSIONS,
    ],
    params: &[MOG_ID, BEFORE],
    access: Access::Core,
};

/// Wipe every row of user content of one MOG (MEM-33a "everything"). It reuses the Forget steps
/// of `mem.forget.mog` (ADR 0081) for the items, the outbox, the exchanges, the utterances and
/// the sessions. It keeps the `mog` row (the parent that writers need; no content), `kv`,
/// `sync_state`, the consent records (SEC-15b) and `forget_log` (ADR 0087).
const WIPE_EVERYTHING: Group = Group {
    id: "mem.wipe.everything",
    steps: &[
        &FORGET_MOG_ITEMS,
        &FORGET_MOG_OUTBOX,
        &FORGET_MOG_EXCHANGES,
        &FORGET_MOG_UTTERANCES,
        &FORGET_MOG_SESSIONS,
    ],
    params: &[MOG_ID],
    access: Access::Core,
};

/// The atomic groups. Their IDs never equal a statement ID.
pub const GROUPS: &[Group] = &[
    ITEM_WRITE,
    ITEM_RATE_GROUP,
    ITEM_EDIT_GROUP,
    FORGET_ITEMS,
    FORGET_SESSION,
    FORGET_MOG,
    WIPE_BEFORE,
    WIPE_EVERYTHING,
];

/// A Rust-only statement that crosses MOGs on purpose, with its reason (MEM-01 exemption, as the
/// consent purge of #63). It is not in [`STATEMENTS`], so `exec` and `mem_local_exec` never run
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceStmt {
    /// The statement. Its access is always [`Access::Core`].
    pub stmt: Stmt,
    /// Why it is device-wide.
    pub why: &'static str,
}

/// The MOG ids of every table that holds user content (R14, #124). Settings -> Wipe is for the
/// whole device, so R14 reads this list and then runs a MOG-scoped group for each id. The
/// outbox has no foreign key to `mog`, so its ids are read too. Ids only.
pub const WIPE_MOG_LIST: DeviceStmt = DeviceStmt {
    stmt: Stmt {
        id: "wipe.mog_list",
        sql: "SELECT mog_id FROM mog UNION SELECT mog_id FROM memory_item \
              UNION SELECT mog_id FROM genie_exchange UNION SELECT mog_id FROM utterance \
              UNION SELECT mog_id FROM session UNION SELECT mog_id FROM outbox \
              ORDER BY mog_id",
        params: &[],
        kind: Kind::Query,
        access: Access::Core,
    },
    why: "Settings -> Wipe is for the whole device (U9, MEM-33a); it reads MOG ids only, and \
          each delete is MOG-scoped",
};

/// The MOG ids and names of the device, for unique MOG folder slugs (ADR 0112). Names and ids
/// only; the handlers that read it run MOG-scoped statements for the rows.
pub const MOG_NAMES: DeviceStmt = DeviceStmt {
    stmt: Stmt {
        id: "mem.mog.names",
        sql: "SELECT mog_id AS mogId, display_name AS name FROM mog ORDER BY mog_id",
        params: &[],
        kind: Kind::Query,
        access: Access::Core,
    },
    why: "a MOG folder name must be unique on the device (ADR 0112); it reads MOG ids and \
          display names only",
};

/// The device-wide statements that [`super::LocalStore::device_rows`] may run.
pub const DEVICE_STMTS: &[&DeviceStmt] = &[&WIPE_MOG_LIST, &MOG_NAMES];

/// The group with this exact ID.
pub fn find_group(id: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.id == id)
}

/// Who may run the statement or group with this ID, or `None` for an unknown ID.
pub fn access_of(id: &str) -> Option<Access> {
    find(id)
        .map(|s| s.access)
        .or_else(|| find_group(id).map(|g| g.access))
        .or_else(|| HANDLERS.contains(&id).then_some(Access::Brain))
}

/// The whole local catalog. `mem_local_exec` and `LocalStore::exec` run nothing else.
pub const STATEMENTS: &[Stmt] = &[
    MOG_UPSERT,
    EXCHANGE_UPSERT,
    EXCHANGE_GET,
    EXCHANGE_RECENT,
    EXCHANGE_DEPENDENTS,
    EXCHANGE_BY_SESSION,
    EXCHANGE_DELETE,
    ITEM_UPSERT,
    ITEM_GET,
    ITEM_LIST_ROWS,
    ITEM_FILE_ROWS,
    MOG_NAME,
    ITEM_RATE,
    ITEM_EDIT,
    RECALL_FTS,
    FORGET_SESSION_CONVERSATIONS,
    FORGET_MOG_CONVERSATIONS,
    FORGET_ITEM_ROWS,
    FORGET_SESSION_ITEMS_BY_SESSION,
    FORGET_SESSION_ITEMS_BY_UTTERANCE,
    FORGET_SESSION_ITEMS_BY_EXCHANGE,
    FORGET_EMBED_ORPHANS,
    FORGET_SESSION_EXCHANGES,
    FORGET_SESSION_UTTERANCES,
    FORGET_SESSION_ROW,
    FORGET_MOG_ITEMS,
    FORGET_MOG_OUTBOX,
    FORGET_MOG_EXCHANGES,
    FORGET_MOG_UTTERANCES,
    FORGET_MOG_SESSIONS,
    FORGET_MOG_KV,
    FORGET_MOG_SYNC_STATE,
    FORGET_MOG_ROW,
    FORGET_LOG_ITEMS,
    FORGET_LOG_SESSION,
    FORGET_LOG_MOG,
    WIPE_ITEMS,
    WIPE_EXCHANGES,
    WIPE_UTTERANCES,
    WIPE_SESSIONS,
    WIPE_CONVERSATIONS,
    OUTBOX_ENQUEUE,
    OUTBOX_DUE,
    OUTBOX_RETRY,
    OUTBOX_DONE,
    OUTBOX_ENQUEUE_ITEM_EMBED,
];

// ---- consent_record (T12, #63, ADR 0052; folded in by #431, ADR 0071) -------------------------

/// How a consent statement keeps MEM-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// It filters by `mog_id = :mogId` and passes `require_mog_id_filter`.
    Mog,
    /// It is device-wide on purpose, for the reason given. It touches `consent_record` only.
    DeviceWide(&'static str),
}

/// One consent statement and its MEM-01 scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentStmt {
    /// The statement. Its access is always [`Access::Core`].
    pub stmt: Stmt,
    /// How it keeps MEM-01.
    pub scope: Scope,
}

/// The notice version is stored as decimal text: the column is `TEXT` (ADR 0052, decision 2).
const NOTICE_VERSION_TEXT: ParamType = text(10);
/// SEC-32a `user`: the signed-in principal, if known.
const PRINCIPAL: ParamType = text(320);
/// Any Unix ms time, also one before 1970 (a cutoff on a clock near the epoch).
const ANY_MS: ParamType = ParamType::Int {
    min: i64::MIN,
    max: i64::MAX,
};

/// The record of one session attestation (9.3 step 7).
pub const CONSENT_SESSION_INSERT: ConsentStmt = ConsentStmt {
    stmt: Stmt {
        id: "consent.session.insert",
        sql: "INSERT INTO consent_record (consent_id, mog_id, owner_principal, device_id, \
              notice_version, kind, session_id, attestation_kind, disclosure_copied, recorded_at, \
              expires_at, updated_at) \
              VALUES (:consentId, :mogId, :user, :deviceId, :noticeVersion, 'session', \
              :sessionId, :attestationKind, :disclosureCopied, :now, :expiresAt, :now)",
        params: &[
            req("consentId", UUID),
            MOG_ID,
            opt("user", PRINCIPAL),
            req("deviceId", UUID),
            req("noticeVersion", NOTICE_VERSION_TEXT),
            req("sessionId", UUID),
            req("attestationKind", CODE),
            req("disclosureCopied", BOOL),
            req("now", ANY_MS),
            req("expiresAt", ANY_MS),
        ],
        kind: Kind::Write,
        access: Access::Core,
    },
    scope: Scope::Mog,
};

/// The latest record of one session in one MOG.
pub const CONSENT_SESSION_GET: ConsentStmt = ConsentStmt {
    stmt: Stmt {
        id: "consent.session.get",
        sql: "SELECT consent_id, mog_id, owner_principal, device_id, notice_version, kind, \
              session_id, attestation_kind, disclosure_copied, recorded_at, expires_at \
              FROM consent_record WHERE mog_id = :mogId AND session_id = :sessionId \
              AND kind = 'session' ORDER BY recorded_at DESC, consent_id LIMIT 1",
        params: &[MOG_ID, req("sessionId", UUID)],
        kind: Kind::Query,
        access: Access::Core,
    },
    scope: Scope::Mog,
};

/// The record of one notice acceptance (9.3 step 5). It has no MOG.
pub const CONSENT_NOTICE_INSERT: ConsentStmt = ConsentStmt {
    stmt: Stmt {
        id: "consent.notice.insert",
        sql: "INSERT INTO consent_record (consent_id, mog_id, owner_principal, device_id, \
              notice_version, kind, recorded_at, expires_at, updated_at) \
              VALUES (:consentId, NULL, :user, :deviceId, :noticeVersion, 'notice', :now, \
              :expiresAt, :now)",
        params: &[
            req("consentId", UUID),
            opt("user", PRINCIPAL),
            req("deviceId", UUID),
            req("noticeVersion", NOTICE_VERSION_TEXT),
            req("now", ANY_MS),
            req("expiresAt", ANY_MS),
        ],
        kind: Kind::Write,
        access: Access::Core,
    },
    scope: Scope::DeviceWide("a notice acceptance is device-wide: it has no MOG (mog_id NULL)"),
};

/// The highest accepted notice version (MB-08a, MB-08b).
pub const CONSENT_NOTICE_LATEST: ConsentStmt = ConsentStmt {
    stmt: Stmt {
        id: "consent.notice.latest",
        sql: "SELECT max(CAST(notice_version AS INTEGER)) FROM consent_record \
              WHERE kind = 'notice' AND mog_id IS NULL",
        params: &[],
        kind: Kind::Query,
        access: Access::Core,
    },
    scope: Scope::DeviceWide("the notice gate (MB-08a) is device-wide"),
};

/// Every consent record of the device, oldest first.
pub const CONSENT_LIST: ConsentStmt = ConsentStmt {
    stmt: Stmt {
        id: "consent.list",
        sql: "SELECT consent_id, mog_id, owner_principal, device_id, notice_version, kind, \
              session_id, attestation_kind, disclosure_copied, recorded_at, expires_at \
              FROM consent_record ORDER BY recorded_at, consent_id",
        params: &[],
        kind: Kind::Query,
        access: Access::Core,
    },
    scope: Scope::DeviceWide("the device's consent list; the records hold no content"),
};

/// SEC-15b: delete the records at or before the cutoff.
pub const CONSENT_PURGE: ConsentStmt = ConsentStmt {
    stmt: Stmt {
        id: "consent.purge",
        sql: "DELETE FROM consent_record WHERE recorded_at <= :cutoff",
        params: &[req("cutoff", ANY_MS)],
        kind: Kind::Write,
        access: Access::Core,
    },
    scope: Scope::DeviceWide("the SEC-15b security expiry runs for the whole device (U9)"),
};

/// The consent statements. Rust only: `exec`, `find` and `mem_local_exec` never see them.
pub const CONSENT: &[ConsentStmt] = &[
    CONSENT_SESSION_INSERT,
    CONSENT_SESSION_GET,
    CONSENT_NOTICE_INSERT,
    CONSENT_NOTICE_LATEST,
    CONSENT_LIST,
    CONSENT_PURGE,
];

/// The consent statement with this exact ID.
pub fn find_consent(id: &str) -> Option<&'static ConsentStmt> {
    CONSENT.iter().find(|c| c.stmt.id == id)
}

/// The statement with this exact ID.
pub fn find(id: &str) -> Option<&'static Stmt> {
    STATEMENTS.iter().find(|s| s.id == id)
}

fn bad() -> CoreError {
    CoreError::InvalidStatementParams { component: ID }
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn is_code(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'.')
}

/// The FTS5 query for free text: each word in double quotes, joined by `OR`. A word is a run of
/// letters and digits, so no quote, operator, column filter or `*` survives. At most 32 words.
fn fts_terms(s: &str) -> Option<String> {
    let words: Vec<String> = s
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(32)
        .map(|w| format!("\"{w}\""))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

fn bind_value(ty: ParamType, v: &Value) -> CoreResult<SqlValue> {
    let text = |v: &Value| v.as_str().map(str::to_owned).ok_or_else(bad);
    Ok(match ty {
        ParamType::Uuid => {
            let s = text(v)?;
            if !is_uuid(&s) {
                return Err(bad());
            }
            SqlValue::Text(s)
        }
        ParamType::Text { max } => {
            let s = text(v)?;
            if s.chars().count() > max {
                return Err(bad());
            }
            SqlValue::Text(s)
        }
        ParamType::Code => {
            let s = text(v)?;
            if !is_code(&s) {
                return Err(bad());
            }
            SqlValue::Text(s)
        }
        ParamType::Int { min, max } => {
            let n = v.as_i64().ok_or_else(bad)?;
            if n < min || n > max {
                return Err(bad());
            }
            SqlValue::Integer(n)
        }
        ParamType::Real => SqlValue::Real(v.as_f64().ok_or_else(bad)?),
        ParamType::Bool => SqlValue::Integer(i64::from(v.as_bool().ok_or_else(bad)?)),
        ParamType::Json { max } => {
            let s = serde_json::to_string(v).map_err(|_| bad())?;
            if s.len() > max {
                return Err(bad());
            }
            SqlValue::Text(s)
        }
        ParamType::FtsTerms => SqlValue::Text(fts_terms(&text(v)?).ok_or_else(bad)?),
        ParamType::Tags {
            max_items,
            max_chars,
        } => {
            let items = v.as_array().ok_or_else(bad)?;
            if items.len() > max_items
                || !items
                    .iter()
                    .all(|t| t.as_str().is_some_and(|t| t.chars().count() <= max_chars))
            {
                return Err(bad());
            }
            SqlValue::Text(serde_json::to_string(v).map_err(|_| bad())?)
        }
        // Never bound from a caller value: `check_params` computes it.
        ParamType::Quarantine { .. }
        | ParamType::Secret { .. }
        | ParamType::Slug { .. }
        | ParamType::Day => return Err(bad()),
    })
}

/// Check `params` against `stmt` and return the value for each parameter, in the order of
/// `stmt.params`. `mogId` is checked first (MEM-01).
pub fn validate(stmt: &Stmt, params: &Value) -> CoreResult<Vec<(&'static str, SqlValue)>> {
    validate_params(stmt.params, params)
}

/// Check `params` against the declared `decl` and return the value for each one, in order.
/// `mogId` is checked first (MEM-01). [`validate`] and the atomic groups use it.
pub fn validate_params(
    decl: &[Param],
    params: &Value,
) -> CoreResult<Vec<(&'static str, SqlValue)>> {
    require_mog_id_param(params)?;
    check_params(decl, params)
}

/// The string values of the params named in `of`, in order. A missing or non-string one is left
/// out (its own type check refuses a non-string value).
fn texts<'a>(obj: &'a Map<String, Value>, of: &'a [&'a str]) -> impl Iterator<Item = &'a str> {
    // A list param (`tags`) gives each of its strings.
    of.iter().filter_map(|k| obj.get(*k)).flat_map(|v| match v {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    })
}

/// Check `params` against `decl` with no `mogId` rule: for the device-wide [`CONSENT`]
/// statements, which run only from `mog_core::consent`. [`validate_params`] adds the MEM-01 rule.
pub fn check_params(decl: &[Param], params: &Value) -> CoreResult<Vec<(&'static str, SqlValue)>> {
    let obj: &Map<String, Value> = params.as_object().ok_or_else(bad)?;
    if obj.keys().any(|k| !decl.iter().any(|p| p.name == k)) {
        return Err(bad());
    }
    decl.iter()
        .map(|p| match (p.ty, obj.get(p.name)) {
            // SEC-24a, ADR 0112, ADR 0113: the core sets these; a caller that sends one is
            // refused.
            (ty, Some(_)) if ty.is_core_computed() => Err(bad()),
            (ParamType::Quarantine { of }, None) => {
                let t = texts(obj, of).collect::<Vec<_>>().join("\n");
                Ok((
                    p.name,
                    SqlValue::Integer(i64::from(super::quarantine::is_quarantined(&t))),
                ))
            }
            (ParamType::Secret { of }, None) => {
                // Each field, and the fields run together: a token split across two fields is
                // found too (security review L1).
                let joined: String = texts(obj, of).collect();
                if super::secret_check::any_secret(texts(obj, of))
                    || super::secret_check::has_secret(&joined)
                {
                    Err(CoreError::SecretRefused { component: ID })
                } else {
                    Ok((p.name, SqlValue::Integer(0)))
                }
            }
            (ParamType::Slug { of }, None) => {
                let first = texts(obj, of).find(|t| !t.trim().is_empty()).unwrap_or("");
                Ok((p.name, SqlValue::Text(super::memory_files::slugify(first))))
            }
            (ParamType::Day, None) => {
                let (offset, now) = day_clock();
                let day = super::memory_files::day_of(now, offset).ok_or_else(bad)?;
                Ok((p.name, SqlValue::Text(day)))
            }
            (_, None | Some(Value::Null)) if p.required => Err(bad()),
            (_, None | Some(Value::Null)) => Ok((p.name, SqlValue::Null)),
            (_, Some(v)) => Ok((p.name, bind_value(p.ty, v)?)),
        })
        .collect()
}
