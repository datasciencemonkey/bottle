//! R10 content-free audit log (#140): SEC-23, RTR-30, PG-15 and the 9.2 "Main agent action log".
//!
//! The app writes one entry for each ask, context send, Page call, agent action and sync. An
//! entry holds IDs, sizes and hashes only (SEC-23). The validation rejects text: every string in
//! a stored entry is a closed-set name, a UUID (`mogId`, `opId`) or a SHA-256 hex value.
//!
//! Rotation (9.2): 30 d by default, admin range 7-90 d. The audit log is content-free security
//! data, not user content, so U9 lets it expire. [`AuditLog::rotate`] has no trigger and no
//! background job: WipeService `run_security_expiries` (#124) is the only caller (ADR 0059).
//!
//! TODO(#442): the R10 SQLCipher `audit_log` table (a migration), the `hashes` field of the
//! `audit_append` IPC arguments and the routing of `audit_append` to [`AuditLog::append`].
//! Until then [`MemoryAuditStore`] is the store.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::error::CoreResult;
use crate::ipc::AuditAppendArgs;

/// One day in milliseconds.
pub const DAY_MS: u64 = 86_400_000;

/// The tools of an `agent_action` entry (RTR-30): the 7 `bottle-mcp` tools (4.7 `BottleTool`)
/// and Genie One MCP through the loopback proxy (AT-42).
pub const AGENT_TOOLS: [&str; 8] = [
    "transcript.search",
    "memory.recall",
    "memory.write",
    "show_blurb",
    "set_mood",
    "propose_page",
    "ask_user",
    "genie_one",
];

/// The size names an entry may hold. Each value is a count.
pub const SIZE_KEYS: [&str; 11] = [
    "chars",
    "bytes",
    "items",
    "question_chars",
    "preamble_chars",
    "preamble_items",
    "answer_chars",
    "args_bytes",
    "result_bytes",
    "synonyms",
    "rows",
];

/// The hash names an entry may hold. Each value is a SHA-256 in 64 lowercase hex chars.
/// `sent` is the hash of the exact text that went to Genie (AT-08, G-35a).
pub const HASH_KEYS: [&str; 4] = ["sent", "payload", "args", "result"];

const AGENT_ACTION_PREFIX: &str = "agent_action:";

/// The kind of an entry (SEC-23). On the wire: `ask`, `context_send`, `page_call`, `sync`, or
/// `agent_action:<tool>` with a tool from [`AGENT_TOOLS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    Ask,
    ContextSend,
    PageCall,
    AgentAction { tool: &'static str },
    Sync,
}

impl AuditKind {
    /// The kind of a wire string, or `None` if it is not in the closed set.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ask" => Some(AuditKind::Ask),
            "context_send" => Some(AuditKind::ContextSend),
            "page_call" => Some(AuditKind::PageCall),
            "sync" => Some(AuditKind::Sync),
            _ => {
                let tool = s.strip_prefix(AGENT_ACTION_PREFIX)?;
                let tool = AGENT_TOOLS.iter().find(|t| **t == tool)?;
                Some(AuditKind::AgentAction { tool })
            }
        }
    }

    /// The kind name without the tool, as the `audit_log.kind` column of #442 holds it.
    pub fn name(&self) -> &'static str {
        match self {
            AuditKind::Ask => "ask",
            AuditKind::ContextSend => "context_send",
            AuditKind::PageCall => "page_call",
            AuditKind::AgentAction { .. } => "agent_action",
            AuditKind::Sync => "sync",
        }
    }

    /// The tool of an agent action.
    pub fn tool(&self) -> Option<&'static str> {
        match self {
            AuditKind::AgentAction { tool } => Some(tool),
            _ => None,
        }
    }

    /// The wire string.
    pub fn wire(&self) -> String {
        match self {
            AuditKind::AgentAction { tool } => format!("{AGENT_ACTION_PREFIX}{tool}"),
            k => k.name().to_owned(),
        }
    }
}

impl Serialize for AuditKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.wire())
    }
}

/// One content-free entry. The map keys come from [`SIZE_KEYS`] and [`HASH_KEYS`] only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// Wall-clock time in epoch milliseconds, stamped by the host.
    pub at_ms: u64,
    pub kind: AuditKind,
    pub mog_id: String,
    pub op_id: String,
    pub sizes: BTreeMap<&'static str, u64>,
    pub hashes: BTreeMap<&'static str, String>,
}

/// Why an append was refused. No variant carries the refused value (SEC-23, ARC-6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditError {
    UnknownKind,
    BadMogId,
    BadOpId,
    UnknownSizeKey,
    UnknownHashKey,
    BadHash,
    /// The store failed. The core error is content-free too.
    Store(crate::error::CoreError),
}

impl AuditError {
    /// A stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            AuditError::UnknownKind => "unknown_kind",
            AuditError::BadMogId => "bad_mog_id",
            AuditError::BadOpId => "bad_op_id",
            AuditError::UnknownSizeKey => "unknown_size_key",
            AuditError::UnknownHashKey => "unknown_hash_key",
            AuditError::BadHash => "bad_hash",
            AuditError::Store(_) => "store_failed",
        }
    }
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the audit entry was refused: {}", self.code())
    }
}

impl std::error::Error for AuditError {}

/// A lowercase UUID (`MogId` is a UUID v4, 4.7).
fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn known(keys: &[&'static str], k: &str) -> Option<&'static str> {
    keys.iter().find(|x| **x == k).copied()
}

impl AuditEntry {
    /// Validate the `audit_append` arguments and the hashes. Every check runs before any write.
    pub fn new(
        args: &AuditAppendArgs,
        hashes: &BTreeMap<String, String>,
        at_ms: u64,
    ) -> Result<Self, AuditError> {
        let kind = AuditKind::parse(&args.kind).ok_or(AuditError::UnknownKind)?;
        if !is_uuid(&args.mog_id) {
            return Err(AuditError::BadMogId);
        }
        // The OperationTracker issues a UUID for each operation, and the AgentBridge mints one for
        // each tool call. A looser ID rule lets joined text through (`What_was_revenue`).
        if !is_uuid(&args.op_id) {
            return Err(AuditError::BadOpId);
        }
        let sizes = args
            .sizes
            .iter()
            .map(|(k, v)| {
                known(&SIZE_KEYS, k)
                    .map(|k| (k, *v))
                    .ok_or(AuditError::UnknownSizeKey)
            })
            .collect::<Result<_, _>>()?;
        let hashes = hashes
            .iter()
            .map(|(k, v)| {
                // RTR-30: an agent action holds sizes only, so it takes no hash.
                let k = known(&HASH_KEYS, k)
                    .filter(|_| kind.tool().is_none())
                    .ok_or(AuditError::UnknownHashKey)?;
                if !is_sha256_hex(v) {
                    return Err(AuditError::BadHash);
                }
                Ok((k, v.clone()))
            })
            .collect::<Result<_, _>>()?;
        Ok(AuditEntry {
            at_ms,
            kind,
            mog_id: args.mog_id.clone(),
            op_id: args.op_id.clone(),
            sizes,
            hashes,
        })
    }
}

/// The retention is outside the 9.2 admin range (7-90 d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionOutOfRange;

/// The retention of the action log (9.2): 30 d by default, admin range 7-90 d.
/// Appendix C has no MDM key for it yet; the host passes the admin value (ADR 0059).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditRetention {
    days: u32,
}

impl AuditRetention {
    /// The 9.2 default.
    pub const DEFAULT_DAYS: u32 = 30;
    /// The admin range.
    pub const MIN_DAYS: u32 = 7;
    pub const MAX_DAYS: u32 = 90;

    /// A retention in the admin range.
    pub fn new(days: u32) -> Result<Self, RetentionOutOfRange> {
        if (Self::MIN_DAYS..=Self::MAX_DAYS).contains(&days) {
            Ok(AuditRetention { days })
        } else {
            Err(RetentionOutOfRange)
        }
    }

    pub fn days(&self) -> u32 {
        self.days
    }

    /// Entries with a time before this cutoff are expired. An entry exactly `days` old stays.
    pub fn cutoff(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(u64::from(self.days) * DAY_MS)
    }
}

impl Default for AuditRetention {
    fn default() -> Self {
        AuditRetention {
            days: Self::DEFAULT_DAYS,
        }
    }
}

/// Where entries live. The store deletes only on [`AuditStore::delete_before`].
pub trait AuditStore: Send + Sync {
    /// Keep one validated entry.
    fn append(&self, entry: AuditEntry) -> CoreResult<()>;
    /// The entries of one MOG, oldest first (MEM-01).
    fn entries(&self, mog_id: &str) -> CoreResult<Vec<AuditEntry>>;
    /// Delete the entries with a time before `cutoff_ms`, for every MOG. Returns the count.
    fn delete_before(&self, cutoff_ms: u64) -> CoreResult<usize>;
}

/// The in-memory store: the test double, and the store until the table of #442 lands.
/// TODO(#442): the SQLCipher store on the `audit_log` table.
#[derive(Debug, Default)]
pub struct MemoryAuditStore {
    entries: Mutex<Vec<AuditEntry>>,
}

impl MemoryAuditStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<AuditEntry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl AuditStore for MemoryAuditStore {
    fn append(&self, entry: AuditEntry) -> CoreResult<()> {
        self.lock().push(entry);
        Ok(())
    }

    fn entries(&self, mog_id: &str) -> CoreResult<Vec<AuditEntry>> {
        let mut v: Vec<AuditEntry> = self
            .lock()
            .iter()
            .filter(|e| e.mog_id == mog_id)
            .cloned()
            .collect();
        v.sort_by_key(|e| e.at_ms);
        Ok(v)
    }

    fn delete_before(&self, cutoff_ms: u64) -> CoreResult<usize> {
        let mut g = self.lock();
        let before = g.len();
        g.retain(|e| e.at_ms >= cutoff_ms);
        Ok(before - g.len())
    }
}

/// The audit log: validation in front of a store, and the 9.2 rotation.
#[derive(Debug)]
pub struct AuditLog<S> {
    store: S,
    retention: AuditRetention,
}

impl<S: AuditStore> AuditLog<S> {
    pub fn new(store: S, retention: AuditRetention) -> Self {
        AuditLog { store, retention }
    }

    pub fn retention(&self) -> AuditRetention {
        self.retention
    }

    /// Validate and keep one entry stamped `at_ms`. A refused call writes nothing. It never
    /// deletes: only [`AuditLog::rotate`] expires entries.
    pub fn append(
        &self,
        args: &AuditAppendArgs,
        hashes: &BTreeMap<String, String>,
        at_ms: u64,
    ) -> Result<AuditEntry, AuditError> {
        let entry = AuditEntry::new(args, hashes, at_ms)?;
        self.store
            .append(entry.clone())
            .map_err(AuditError::Store)?;
        Ok(entry)
    }

    /// The entries of one MOG, oldest first.
    pub fn entries(&self, mog_id: &str) -> CoreResult<Vec<AuditEntry>> {
        self.store.entries(mog_id)
    }

    /// The 9.2 rotation: delete entries older than the retention. It touches the audit log only,
    /// never user content (U9). WipeService `run_security_expiries` (#124) calls it.
    pub fn rotate(&self, now_ms: u64) -> CoreResult<usize> {
        self.store.delete_before(self.retention.cutoff(now_ms))
    }
}

#[cfg(test)]
mod tests;
