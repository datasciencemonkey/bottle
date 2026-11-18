//! R13 PolicyStore (section 4.3).
//!
//! Responsibilities: MDM and admin keys (Appendix C) and the telemetry gate (D42).
//!
//! The telemetry gate (#184, ADR 0079) fails closed: a missing, malformed or unreadable key
//! keeps telemetry and content logging off (SEC-16a, SEC-16c).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{CoreError, CoreResult};

const ID: &str = "R13";

/// Appendix C: the Unity Catalog table for OTLP spans. Unset by default (SEC-16a, ADM-19).
pub const OTLP_TABLE: &str = "OtlpTable";
/// Appendix C: the admin half of the content-logging opt-in. `false` by default (SEC-16c).
pub const CONTENT_TELEMETRY_ALLOWED: &str = "ContentTelemetryAllowed";

/// The effective MDM and admin policy values.
pub trait PolicyStore: Send + Sync {
    /// The effective value of one MDM or admin key, or `None` if it is not set.
    fn get(&self, key: &str) -> CoreResult<Option<Value>>;

    /// The telemetry gate: true only if `OtlpTable` holds a valid table name (SEC-16a).
    /// A read error is returned; [`otlp_table`] turns it into "off".
    fn telemetry_allowed(&self) -> CoreResult<bool> {
        Ok(self
            .get(OTLP_TABLE)?
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(is_uc_table_name))
    }
}

impl<P: PolicyStore + ?Sized> PolicyStore for &P {
    fn get(&self, key: &str) -> CoreResult<Option<Value>> {
        (**self).get(key)
    }
}

/// A three-part Unity Catalog name `catalog.schema.table`. Each part is 1-255 ASCII letters,
/// digits or `_`. No quoting, no spaces, no URL (ADR 0079).
pub fn is_uc_table_name(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            (1..=255).contains(&p.len())
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

/// The OTLP table, or `None` when telemetry is off. Fails closed on a read error (SEC-16a).
pub fn otlp_table<P: PolicyStore + ?Sized>(policy: &P) -> Option<String> {
    match policy.get(OTLP_TABLE) {
        Ok(Some(Value::String(s))) if is_uc_table_name(&s) => Some(s),
        _ => None,
    }
}

/// The admin opt-in for content logging: only a boolean `true` counts (SEC-16c).
pub fn content_telemetry_allowed<P: PolicyStore + ?Sized>(policy: &P) -> bool {
    matches!(
        policy.get(CONTENT_TELEMETRY_ALLOWED),
        Ok(Some(Value::Bool(true)))
    )
}

/// The R13 stub. Every read returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubPolicyStore;

impl PolicyStore for StubPolicyStore {
    fn get(&self, _key: &str) -> CoreResult<Option<Value>> {
        // TODO(#186): MDM keys of Appendix C.
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// A map of key values: the test double, and the store a host fills until #186 reads MDM.
#[derive(Debug, Default, Clone)]
pub struct MemoryPolicyStore {
    values: BTreeMap<String, Value>,
}

impl MemoryPolicyStore {
    /// The store with `key` set to `value`.
    pub fn with(mut self, key: &str, value: Value) -> Self {
        self.values.insert(key.to_owned(), value);
        self
    }
}

impl PolicyStore for MemoryPolicyStore {
    fn get(&self, key: &str) -> CoreResult<Option<Value>> {
        Ok(self.values.get(key).cloned())
    }
}
