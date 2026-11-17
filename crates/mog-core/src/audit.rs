//! R10 content-free audit log (#140): SEC-23, RTR-30, PG-15, 9.2 "Main agent action log".
//!
//! Red skeleton: the API of ADR 0059 with no behaviour yet.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::ipc::AuditAppendArgs;

const ID: &str = "R10";

/// One day in milliseconds.
pub const DAY_MS: u64 = 86_400_000;

/// The tools of an `agent_action` entry.
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

/// The kind of an audit entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    Ask,
    ContextSend,
    PageCall,
    AgentAction { tool: &'static str },
    Sync,
}

impl Serialize for AuditKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("")
    }
}

/// One entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub at_ms: u64,
    pub kind: AuditKind,
    pub mog_id: String,
    pub op_id: String,
    pub sizes: BTreeMap<&'static str, u64>,
    pub hashes: BTreeMap<&'static str, String>,
}

/// Why an append was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditError {
    UnknownKind,
    BadMogId,
    BadOpId,
    UnknownSizeKey,
    UnknownHashKey,
    BadHash,
    Store(CoreError),
}

impl AuditError {
    pub fn code(&self) -> &'static str {
        ""
    }
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "")
    }
}

/// The retention is outside the admin range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionOutOfRange;

/// The retention of the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditRetention {
    days: u32,
}

impl AuditRetention {
    pub fn new(_days: u32) -> Result<Self, RetentionOutOfRange> {
        Err(RetentionOutOfRange)
    }
    pub fn days(&self) -> u32 {
        self.days
    }
}

impl Default for AuditRetention {
    fn default() -> Self {
        AuditRetention { days: 0 }
    }
}

/// Where entries live.
pub trait AuditStore: Send + Sync {
    fn append(&self, entry: AuditEntry) -> CoreResult<()>;
    fn entries(&self, mog_id: &str) -> CoreResult<Vec<AuditEntry>>;
    fn delete_before(&self, cutoff_ms: u64) -> CoreResult<usize>;
}

/// An in-memory store.
#[derive(Debug, Default)]
pub struct MemoryAuditStore {
    entries: Mutex<Vec<AuditEntry>>,
}

impl AuditStore for MemoryAuditStore {
    fn append(&self, _entry: AuditEntry) -> CoreResult<()> {
        let _ = &self.entries;
        Err(CoreError::NotImplemented { component: ID })
    }
    fn entries(&self, _mog_id: &str) -> CoreResult<Vec<AuditEntry>> {
        Ok(Vec::new())
    }
    fn delete_before(&self, _cutoff_ms: u64) -> CoreResult<usize> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The audit log.
#[derive(Debug)]
pub struct AuditLog<S> {
    store: S,
    retention: AuditRetention,
}

impl<S: AuditStore> AuditLog<S> {
    pub fn new(store: S, retention: AuditRetention) -> Self {
        AuditLog { store, retention }
    }
    pub fn append(
        &self,
        _args: &AuditAppendArgs,
        _hashes: &BTreeMap<String, String>,
        _at_ms: u64,
    ) -> Result<AuditEntry, AuditError> {
        let _ = (&self.store, self.retention);
        Err(AuditError::Store(CoreError::NotImplemented {
            component: ID,
        }))
    }
    pub fn entries(&self, mog_id: &str) -> CoreResult<Vec<AuditEntry>> {
        self.store.entries(mog_id)
    }
    pub fn rotate(&self, _now_ms: u64) -> CoreResult<usize> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

#[cfg(test)]
mod tests;
