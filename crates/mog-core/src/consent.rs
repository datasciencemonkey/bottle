//! T12 ConsentService, Rust side (#63): content-free consent records in the R10 local store.
//! RED: the API only. Every method fails until the green commit.

use std::sync::Arc;

use crate::clock::WallClock;
pub use crate::error::{CoreError, CoreResult};
pub use crate::ipc::AttestKind;
use crate::local_store::SqlCipherLocalStore;

const ID: &str = "T12";

pub const DEFAULT_RETENTION_MONTHS: u32 = 13;
pub const MIN_RETENTION_MONTHS: u32 = 12;
pub const MAX_RETENTION_MONTHS: u32 = 84;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentRetention {
    months: u32,
}

impl ConsentRetention {
    pub fn months(_months: u32) -> CoreResult<Self> {
        Err(CoreError::NotImplemented { component: ID })
    }
    pub fn get(self) -> u32 {
        self.months
    }
}

impl Default for ConsentRetention {
    fn default() -> Self {
        Self { months: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentKind {
    Notice,
    Session,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConsentRecord {
    pub consent_id: String,
    pub mog_id: Option<String>,
    pub user: Option<String>,
    pub device: String,
    pub notice_version: u32,
    pub kind: ConsentKind,
    pub session_id: Option<String>,
    pub attestation_kind: Option<AttestKind>,
    pub disclosure_copied: Option<bool>,
    pub accepted_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct SessionAttestation<'a> {
    pub mog_id: &'a str,
    pub session_id: &'a str,
    pub kind: AttestKind,
    pub notice_version: u32,
    pub user: Option<&'a str>,
    pub disclosure_copied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Mog,
    #[allow(dead_code)]
    DeviceWide(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Stmt {
    pub id: &'static str,
    pub sql: &'static str,
    pub scope: Scope,
}

pub(crate) const STATEMENTS: &[Stmt] = &[];

pub fn add_months_utc(ms: i64, _months: i32) -> i64 {
    ms
}

pub struct ConsentRecords {
    #[allow(dead_code)]
    store: Arc<SqlCipherLocalStore>,
    #[allow(dead_code)]
    clock: Arc<dyn WallClock>,
}

impl ConsentRecords {
    pub fn new(
        store: Arc<SqlCipherLocalStore>,
        clock: Arc<dyn WallClock>,
        _device_id: &str,
        _retention: ConsentRetention,
    ) -> CoreResult<Self> {
        Ok(Self { store, clock })
    }
    pub fn record_notice(&self, _user: Option<&str>, _v: u32) -> CoreResult<ConsentRecord> {
        Err(CoreError::NotImplemented { component: ID })
    }
    pub fn record_session(&self, _a: &SessionAttestation<'_>) -> CoreResult<ConsentRecord> {
        Err(CoreError::NotImplemented { component: ID })
    }
    pub fn latest_notice_version(&self) -> CoreResult<Option<u32>> {
        Err(CoreError::NotImplemented { component: ID })
    }
    pub fn session_record(&self, _m: &str, _s: &str) -> CoreResult<Option<ConsentRecord>> {
        Err(CoreError::NotImplemented { component: ID })
    }
    pub fn records(&self) -> CoreResult<Vec<ConsentRecord>> {
        Err(CoreError::NotImplemented { component: ID })
    }
    pub fn purge_expired(&self) -> CoreResult<usize> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

#[cfg(test)]
mod tests;
