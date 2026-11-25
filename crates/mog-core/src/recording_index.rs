//! The local recording index (migration 0005; owner authorization 2026-11-25 in chat): one
//! table in the same SQLCipher store as Conversations, through the same store handle and the
//! same key (ADR 0105, `local_store::SqlCipherLocalStore`). See the migration's header for why
//! this table carries no `mog_id`.
//!
//! It holds metadata only: a recording's name, profile, times, status and the user's Remove
//! (`hidden`). No transcript, no notes, no token, no file path. `recording_cloud` writes a row
//! at every status transition (through [`RecordingIndexStore::upsert`]) and `recording_list`
//! reads it for the fast path (no network, PR C's `recording_cloud::contract::CloudAdapter`).
//! `recording_delete` only ever sets `hidden`.
//!
//! The statements are [`local_store::catalog::RECORDING`]: fixed SQL, Rust-only, validated by
//! `catalog::check_params` (no MEM-01 rule, the same way `catalog::CONSENT` runs). The brain
//! never reaches this table: it is not in `catalog::STATEMENTS`, so `exec` and `mem_local_exec`
//! never see it.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::error::{CoreError, CoreResult};
use crate::local_store::catalog::{self, Stmt};
use crate::local_store::SqlCipherLocalStore;
use crate::types::{RecordingErrorKind, RecordingStatus};

const ID: &str = "R10";

fn unknown_stmt() -> CoreError {
    CoreError::UnknownStatement { component: ID }
}

/// One row of the local recording index. Metadata only (see the module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingIndexRow {
    pub name: String,
    pub profile: String,
    /// Epoch milliseconds.
    pub started_at_ms: u64,
    pub duration_s: u32,
    pub status: RecordingStatus,
    pub error_kind: Option<RecordingErrorKind>,
    /// The step that failed (mirrors [`crate::types::RecordingSummary::error_stage`]).
    pub error_stage: Option<RecordingStatus>,
    pub on_device_only: bool,
    /// Epoch milliseconds of the last write to this row.
    pub updated_at_ms: i64,
}

/// A `RecordingStatus` or `RecordingErrorKind` as its plain snake_case wire string (both derive
/// `Serialize` with `rename_all = "snake_case"`), for the SQL `CHECK`ed TEXT columns.
fn code_of<T: serde::Serialize>(v: T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

fn parse_code<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(Value::String(s.to_owned())).ok()
}

/// Reads and writes [`RecordingIndexRow`]s. Rust only: `recording_cloud` is the only caller.
pub trait RecordingIndexStore: Send + Sync {
    /// Add or refresh one row at a status transition (start, upload progress, transcribing,
    /// writing notes, done, failed). Never un-hides a row the user removed.
    fn upsert(&self, row: &RecordingIndexRow) -> CoreResult<()>;
    /// `recording_delete`: set `hidden = 1`. Local and instant; no network.
    fn hide(&self, name: &str, now_ms: i64) -> CoreResult<()>;
    /// The visible rows (`hidden = 0`) of `profile`, newest first.
    fn list(&self, profile: &str) -> CoreResult<Vec<RecordingIndexRow>>;
    /// The hidden (`hidden = 1`) names of `profile` (Opus review round 2, M1/M2): the fast and
    /// reconcile paths filter the live/active/in-flight overlays and the final reply by this
    /// set directly, rather than by what [`Self::list`] does *not* contain — the difference
    /// matters when this store has no rows for a reason that is not "nothing is hidden"
    /// (`NoRecordingIndex`, or a transient read error), where that would read as everything
    /// hidden instead of nothing.
    fn hidden_names(&self, profile: &str) -> CoreResult<HashSet<String>>;
    /// Settings -> Wipe "everything": clear every row. Answers the number removed.
    fn wipe_everything(&self) -> CoreResult<u64>;
    /// Settings -> Wipe by age: rows that started strictly before `before_ms`.
    fn wipe_before(&self, before_ms: i64) -> CoreResult<u64>;
}

/// No index (tests, and a host that has not wired the local store in yet). Every write is a
/// no-op; every read answers empty. `recording_list` falls back to the network path when this
/// is in use (see `recording_cloud::contract::CloudAdapter::list_fast`).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRecordingIndex;

impl RecordingIndexStore for NoRecordingIndex {
    fn upsert(&self, _row: &RecordingIndexRow) -> CoreResult<()> {
        Ok(())
    }
    fn hide(&self, _name: &str, _now_ms: i64) -> CoreResult<()> {
        Ok(())
    }
    fn list(&self, _profile: &str) -> CoreResult<Vec<RecordingIndexRow>> {
        Ok(Vec::new())
    }
    fn hidden_names(&self, _profile: &str) -> CoreResult<HashSet<String>> {
        Ok(HashSet::new())
    }
    fn wipe_everything(&self) -> CoreResult<u64> {
        Ok(0)
    }
    fn wipe_before(&self, _before_ms: i64) -> CoreResult<u64> {
        Ok(0)
    }
}

/// The real index, on the one R10 local store.
pub struct SqlRecordingIndex {
    store: Arc<SqlCipherLocalStore>,
}

impl SqlRecordingIndex {
    /// An index on `store` (the same store and key as Conversations; ADR 0105).
    pub fn new(store: Arc<SqlCipherLocalStore>) -> Self {
        Self { store }
    }

    fn stmt(id: &'static str) -> CoreResult<&'static Stmt> {
        catalog::find_recording(id).ok_or_else(unknown_stmt)
    }

    /// Run one fixed write statement (MEM-34a: catalog SQL only, bound values only). Answers
    /// the number of rows it changed.
    fn write(&self, s: &Stmt, params: &Value) -> CoreResult<u64> {
        let values = catalog::check_params(s.params, params)?;
        let n = self.store.with_conn(|c| {
            let mut st = c.prepare_cached(s.sql)?;
            for (name, value) in &values {
                if let Some(i) = st.parameter_index(&format!(":{name}"))? {
                    st.raw_bind_parameter(i, value.clone())?;
                }
            }
            st.raw_execute()
        })?;
        Ok(n as u64)
    }
}

impl RecordingIndexStore for SqlRecordingIndex {
    fn upsert(&self, row: &RecordingIndexRow) -> CoreResult<()> {
        let s = Self::stmt("recording.upsert")?;
        self.write(
            s,
            &json!({
                "name": row.name,
                "profile": row.profile,
                "startedAtMs": row.started_at_ms,
                "durationS": row.duration_s,
                "status": code_of(row.status),
                "errorKind": row.error_kind.map(code_of),
                "errorStage": row.error_stage.map(code_of),
                "onDeviceOnly": row.on_device_only,
                "updatedAtMs": row.updated_at_ms,
            }),
        )?;
        Ok(())
    }

    fn hide(&self, name: &str, now_ms: i64) -> CoreResult<()> {
        let s = Self::stmt("recording.hide")?;
        self.write(s, &json!({ "name": name, "updatedAtMs": now_ms }))?;
        Ok(())
    }

    fn list(&self, profile: &str) -> CoreResult<Vec<RecordingIndexRow>> {
        let s = Self::stmt("recording.list")?;
        let values = catalog::check_params(s.params, &json!({ "profile": profile }))?;
        self.store.with_conn(|c| {
            let mut st = c.prepare_cached(s.sql)?;
            for (name, value) in &values {
                if let Some(i) = st.parameter_index(&format!(":{name}"))? {
                    st.raw_bind_parameter(i, value.clone())?;
                }
            }
            let mut rows = st.raw_query();
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let status_s: String = r.get("status")?;
                let error_kind_s: Option<String> = r.get("errorKind")?;
                let error_stage_s: Option<String> = r.get("errorStage")?;
                let on_device_only: i64 = r.get("onDeviceOnly")?;
                let started_at_ms: i64 = r.get("startedAtMs")?;
                let duration_s: i64 = r.get("durationS")?;
                out.push(RecordingIndexRow {
                    name: r.get("name")?,
                    profile: r.get("profile")?,
                    started_at_ms: started_at_ms.max(0) as u64,
                    duration_s: duration_s.clamp(0, i64::from(u32::MAX)) as u32,
                    status: parse_code(&status_s).unwrap_or(RecordingStatus::Failed),
                    error_kind: error_kind_s.as_deref().and_then(parse_code),
                    error_stage: error_stage_s.as_deref().and_then(parse_code),
                    on_device_only: on_device_only != 0,
                    updated_at_ms: r.get("updatedAtMs")?,
                });
            }
            Ok(out)
        })
    }

    fn hidden_names(&self, profile: &str) -> CoreResult<HashSet<String>> {
        let s = Self::stmt("recording.hidden_names")?;
        let values = catalog::check_params(s.params, &json!({ "profile": profile }))?;
        self.store.with_conn(|c| {
            let mut st = c.prepare_cached(s.sql)?;
            for (name, value) in &values {
                if let Some(i) = st.parameter_index(&format!(":{name}"))? {
                    st.raw_bind_parameter(i, value.clone())?;
                }
            }
            let mut rows = st.raw_query();
            let mut out = HashSet::new();
            while let Some(r) = rows.next()? {
                out.insert(r.get("name")?);
            }
            Ok(out)
        })
    }

    fn wipe_everything(&self) -> CoreResult<u64> {
        let s = Self::stmt("recording.wipe_everything")?;
        self.write(s, &json!({}))
    }

    fn wipe_before(&self, before_ms: i64) -> CoreResult<u64> {
        let s = Self::stmt("recording.wipe_before")?;
        self.write(s, &json!({ "before": before_ms.max(0) }))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::local_store::key_vault::MemoryKeyVault;
    use crate::local_store::LocalStore;

    /// A unique temp folder that is removed on drop (no extra crate; mirrors
    /// `local_store::tests::TempDir`).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir()
                .join(format!("bottle-rec-idx-{tag}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_store(dir: &Path) -> Arc<SqlCipherLocalStore> {
        test_store_with_vault(dir, Arc::new(MemoryKeyVault::new()))
    }

    fn test_store_with_vault(dir: &Path, vault: Arc<MemoryKeyVault>) -> Arc<SqlCipherLocalStore> {
        Arc::new(SqlCipherLocalStore::new(dir.join("memory"), vault))
    }

    fn row(
        name: &str,
        profile: &str,
        status: RecordingStatus,
        updated_at_ms: i64,
    ) -> RecordingIndexRow {
        RecordingIndexRow {
            name: name.to_owned(),
            profile: profile.to_owned(),
            started_at_ms: 1_000,
            duration_s: 12,
            status,
            error_kind: None,
            error_stage: None,
            on_device_only: false,
            updated_at_ms,
        }
    }

    /// The migration is tested, and the table is in the existing store: the same
    /// `SqlCipherLocalStore` that Conversations uses answers a recording upsert and a
    /// `mem.mog.upsert`-shaped write, with the one store handle and the one key (ADR 0105): no
    /// second connection and no second keychain item.
    #[test]
    fn recording_table_is_in_the_existing_store_and_key() {
        let dir = TempDir::new("case");
        let store = test_store(dir.path());
        let index = SqlRecordingIndex::new(Arc::clone(&store));
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Recording, 1))
            .unwrap();
        // The same store answers a MEM-01 mog write too: one file, one handle, one key.
        let mog_id = "11111111-1111-1111-1111-111111111111";
        store
            .exec(
                "mem.mog.upsert",
                json!({
                    "mogId": mog_id,
                    "displayName": "Home",
                    "backendKind": "genie_one_mcp",
                    "rowHlc": 1,
                    "deviceId": "22222222-2222-2222-2222-222222222222",
                }),
            )
            .unwrap();
        assert_eq!(index.list("home").unwrap().len(), 1);
        // One database file (`test_store` puts it at `<dir>/memory/memory.db`): no second
        // `.db` sibling from a second connection or key.
        let siblings: Vec<_> = std::fs::read_dir(dir.path().join("memory"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("db"))
            .collect();
        assert_eq!(siblings.len(), 1);
    }

    #[test]
    fn upsert_then_list_reads_back_the_row() {
        let dir = TempDir::new("case");
        let index = SqlRecordingIndex::new(test_store(dir.path()));
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Uploading, 10))
            .unwrap();
        let rows = index.list("home").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "rec-1");
        assert_eq!(rows[0].status, RecordingStatus::Uploading);
    }

    #[test]
    fn upsert_updates_status_in_place() {
        let dir = TempDir::new("case");
        let index = SqlRecordingIndex::new(test_store(dir.path()));
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Recording, 1))
            .unwrap();
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Done, 2))
            .unwrap();
        let rows = index.list("home").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, RecordingStatus::Done);
        assert_eq!(rows[0].updated_at_ms, 2);
    }

    #[test]
    fn hide_removes_a_row_from_list_and_upsert_never_undoes_it() {
        let dir = TempDir::new("case");
        let index = SqlRecordingIndex::new(test_store(dir.path()));
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Failed, 1))
            .unwrap();
        index.hide("rec-1", 2).unwrap();
        assert!(index.list("home").unwrap().is_empty());
        // A reconcile sees it again (still on the volume, say) and upserts it: it stays hidden.
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Failed, 3))
            .unwrap();
        assert!(index.list("home").unwrap().is_empty());
    }

    #[test]
    fn hidden_names_reports_only_what_is_hidden() {
        let dir = TempDir::new("case");
        let index = SqlRecordingIndex::new(test_store(dir.path()));
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Failed, 1))
            .unwrap();
        index
            .upsert(&row("rec-2", "home", RecordingStatus::Done, 1))
            .unwrap();
        assert!(index.hidden_names("home").unwrap().is_empty());
        index.hide("rec-1", 2).unwrap();
        let hidden = index.hidden_names("home").unwrap();
        assert_eq!(hidden, HashSet::from(["rec-1".to_owned()]));
    }

    #[test]
    fn list_is_scoped_to_one_profile() {
        let dir = TempDir::new("case");
        let index = SqlRecordingIndex::new(test_store(dir.path()));
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Done, 1))
            .unwrap();
        index
            .upsert(&row("rec-2", "work", RecordingStatus::Done, 1))
            .unwrap();
        assert_eq!(index.list("home").unwrap().len(), 1);
        assert_eq!(index.list("work").unwrap().len(), 1);
    }

    #[test]
    fn wipe_everything_clears_the_table() {
        let dir = TempDir::new("case");
        let index = SqlRecordingIndex::new(test_store(dir.path()));
        index
            .upsert(&row("rec-1", "home", RecordingStatus::Done, 1))
            .unwrap();
        index
            .upsert(&row("rec-2", "home", RecordingStatus::Done, 1))
            .unwrap();
        assert_eq!(index.wipe_everything().unwrap(), 2);
        assert!(index.list("home").unwrap().is_empty());
    }

    #[test]
    fn wipe_before_clears_only_older_rows() {
        let dir = TempDir::new("case");
        let index = SqlRecordingIndex::new(test_store(dir.path()));
        let mut old = row("rec-old", "home", RecordingStatus::Done, 1);
        old.started_at_ms = 100;
        let mut new = row("rec-new", "home", RecordingStatus::Done, 1);
        new.started_at_ms = 5_000;
        index.upsert(&old).unwrap();
        index.upsert(&new).unwrap();
        assert_eq!(index.wipe_before(1_000).unwrap(), 1);
        let rows = index.list("home").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "rec-new");
    }

    #[test]
    fn survives_a_reopen_of_the_store() {
        let dir = TempDir::new("case");
        let vault = Arc::new(MemoryKeyVault::new());
        {
            let store = test_store_with_vault(dir.path(), Arc::clone(&vault));
            let index = SqlRecordingIndex::new(store);
            index
                .upsert(&row("rec-1", "home", RecordingStatus::Done, 1))
                .unwrap();
        }
        // A new store instance, same directory and the same key (as a relaunch reads the same
        // keychain item): the table and its row are still there.
        let reopened = test_store_with_vault(dir.path(), vault);
        let index = SqlRecordingIndex::new(reopened);
        let rows = index.list("home").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "rec-1");
    }
}
