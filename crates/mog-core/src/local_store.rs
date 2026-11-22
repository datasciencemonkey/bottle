//! R10 LocalStore (section 4.3).
//!
//! Responsibilities: **The MVP memory store.** One SQLCipher 4 file with the key in the OS keychain, WAL, FTS5, `sqlite-vec`, outbox (embedding queue, v1 sync), audit log. Fixed statement catalog.

use crate::error::{CoreError, CoreResult};
use crate::ipc::MemExecArgs;

pub mod catalog;
pub mod key_vault;
pub mod memory_files;
mod migrate;
pub mod mog_guard;
pub mod quarantine;
pub mod secret_check;
mod store;
#[cfg(test)]
mod tests;
pub mod vec_codec;

pub use store::{SqlCipherLocalStore, DB_FILE_NAME};

const ID: &str = "R10";

/// The encrypted local memory store. Callers run fixed statements by ID, never raw SQL.
pub trait LocalStore: Send + Sync {
    /// Open the store with the key from the OS keychain.
    fn open(&self) -> CoreResult<()>;
    /// Run one statement of the fixed local catalog by its ID.
    fn exec(&self, stmt_id: &str, params: serde_json::Value) -> CoreResult<serde_json::Value>;
    /// Run one device-wide query of [`catalog::DEVICE_STMTS`] by its ID (it has no params and
    /// crosses MOGs on purpose, with its reason). A store with none answers `NotImplemented`.
    fn device_rows(&self, stmt_id: &str) -> CoreResult<Vec<serde_json::Value>> {
        let _ = stmt_id;
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// A store that is not there: every method returns [`CoreError::NotImplemented`]. Hosts and tests
/// use it where no memory is wired. The real store is [`SqlCipherLocalStore`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubLocalStore;

impl LocalStore for StubLocalStore {
    fn open(&self) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn exec(&self, _stmt_id: &str, _params: serde_json::Value) -> CoreResult<serde_json::Value> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The fixed IPC code of a store error (SEC-03: no value in it).
fn ipc_code(e: &CoreError) -> &'static str {
    match e {
        CoreError::UnknownStatement { .. }
        | CoreError::InvalidStatementParams { .. }
        | CoreError::MogIdRequired { .. } => "invalid_args",
        CoreError::SecretRefused { .. } => "secret_refused",
        CoreError::NotImplemented { .. } => "not_implemented",
        CoreError::IpcRefused { code } => code,
        _ => "host_failed",
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

/// The R10 side of the `mem_local_exec` IPC command (#37, MEM-34a, MEM-34b).
///
/// It parses `MemExecArgs`, refuses a `stmtId` that is not a [`catalog::Access::Brain`] entry,
/// group or handler of the fixed catalog, runs it on `store` and answers `{ "rows": [...] }`.
/// Every refusal is a fixed IPC code with no value in it: `invalid_args` for an unknown or
/// Rust-only id and for bad params, `secret_refused` when the secret check refuses a memory
/// write (ADR 0113), `not_implemented` for a store that is not there, and `host_failed` for a
/// store that cannot run the statement.
///
/// The `actionTicket` of the args is the host's business: the app host (`memory_host.rs`)
/// redeems it before it calls this function. This function never pins on its own account; the
/// host refuses a pin with no valid ticket.
/// The app host routes `mem_local_exec` here (`src-tauri/src/ipc.rs`, #426).
pub fn mem_local_exec(
    store: &dyn LocalStore,
    args: serde_json::Value,
) -> CoreResult<serde_json::Value> {
    let a: MemExecArgs = serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
    if catalog::access_of(&a.stmt_id) != Some(catalog::Access::Brain) {
        return Err(refused("invalid_args"));
    }
    let params = serde_json::Value::Object(a.params.into_iter().collect());
    let out = match a.stmt_id.as_str() {
        "mem.item.list" => files::item_list(store, &params),
        "mem.file.get" => files::file_get(store, &params),
        id => store.exec(id, params),
    };
    out.map_err(|e| refused(ipc_code(&e)))
}

/// The user-only memory writes (ADR 0113). The app host calls this only after it redeemed a
/// ticket of the right purpose for these exact params: `mem.item.rate` and `mem.item.edit`
/// (Rust-only groups) and a pinned `mem.item.write`. Any other id is `invalid_args`. Errors are
/// the same fixed codes as [`mem_local_exec`].
pub fn exec_ticketed(
    store: &dyn LocalStore,
    stmt_id: &str,
    params: serde_json::Value,
) -> CoreResult<serde_json::Value> {
    if !matches!(
        stmt_id,
        "mem.item.rate" | "mem.item.edit" | "mem.item.write"
    ) {
        return Err(refused("invalid_args"));
    }
    store
        .exec(stmt_id, params)
        .map_err(|e| refused(ipc_code(&e)))
}

/// The `mem.item.list` and `mem.file.get` handlers (ADR 0112): fixed MOG-scoped statements, then
/// the renderer of [`memory_files`]. Nothing here logs: the rows are user content (SEC-16c).
mod files {
    use super::memory_files::{self, Audience, Ctx, File, MemoryRow, VPath};
    use super::{catalog, LocalStore};
    use crate::error::{CoreError, CoreResult};
    use serde_json::{json, Map, Value};

    fn bad() -> CoreError {
        CoreError::InvalidStatementParams {
            component: super::ID,
        }
    }

    /// MEM-01: a UUID `mogId` is required.
    fn mog_id(params: &Value) -> CoreResult<&str> {
        super::mog_guard::require_mog_id_param(params)
    }

    /// Only the keys in `allowed`, as every catalog entry refuses an unknown key.
    fn only(params: &Value, allowed: &[&str]) -> CoreResult<()> {
        let obj = params.as_object().ok_or_else(bad)?;
        if obj.keys().all(|k| allowed.contains(&k.as_str())) {
            Ok(())
        } else {
            Err(bad())
        }
    }

    fn rows(v: Value) -> Vec<Value> {
        v.get("rows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// The folder slug and the display name of `mog`.
    fn folder_of(store: &dyn LocalStore, mog: &str) -> CoreResult<(String, String)> {
        let names: Vec<(String, String)> = store
            .device_rows(catalog::MOG_NAMES.stmt.id)?
            .iter()
            .filter_map(|r| {
                Some((
                    r.get("mogId")?.as_str()?.to_owned(),
                    r.get("name")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        let slugs = memory_files::mog_slugs(&names);
        let at = names.iter().position(|(id, _)| id == mog).ok_or_else(bad)?;
        Ok((slugs[at].1.clone(), names[at].1.clone()))
    }

    fn parse_rows(v: Vec<Value>) -> Vec<MemoryRow> {
        v.iter().filter_map(MemoryRow::from_json).collect()
    }

    /// One row as the hub's `MemoryFileRow`: no body, `tags` as a list, `pinned` as a bool, and
    /// no key for a time that is not set.
    fn file_row(r: &MemoryRow, folder: &str) -> Option<Value> {
        let file = r.file(folder)?;
        let mut o = Map::new();
        o.insert("itemId".into(), json!(r.item_id));
        o.insert("kind".into(), json!(r.kind));
        o.insert("layer".into(), json!(r.layer));
        o.insert("file".into(), json!(file.path()));
        o.insert("path".into(), json!(r.path(folder)?));
        o.insert("title".into(), json!(r.title));
        o.insert("summary".into(), json!(r.text));
        o.insert("status".into(), json!(r.status));
        o.insert("pinned".into(), json!(r.pinned));
        o.insert("source".into(), json!(r.source));
        o.insert("origin".into(), json!(r.origin));
        o.insert("tags".into(), json!(r.tags));
        o.insert("rating".into(), json!(r.rating.clamp(-1, 1)));
        if let Some(t) = r.reviewed_ms {
            o.insert("reviewedAt".into(), json!(t));
        }
        if let Some(t) = r.released_ms {
            o.insert("releasedAt".into(), json!(t));
        }
        o.insert("createdAt".into(), json!(r.created_ms));
        o.insert("updatedAt".into(), json!(r.updated_ms));
        Some(Value::Object(o))
    }

    /// `mem.item.list {mogId, limit?}`: the items in use, as `MemoryFileRow`s, plus the folder.
    pub(super) fn item_list(store: &dyn LocalStore, params: &Value) -> CoreResult<Value> {
        only(params, &["mogId", "limit"])?;
        let mog = mog_id(params)?;
        let limit = match params.get("limit") {
            None | Some(Value::Null) => 500,
            Some(v) => v
                .as_i64()
                .filter(|n| (1..=500).contains(n))
                .ok_or_else(bad)?,
        };
        let (folder, _) = folder_of(store, mog)?;
        let list = parse_rows(rows(store.exec(
            "mem.item.list_rows",
            json!({ "mogId": mog, "limit": limit }),
        )?));
        let out: Vec<Value> = list.iter().filter_map(|r| file_row(r, &folder)).collect();
        Ok(json!({ "rows": out, "folder": folder }))
    }

    /// `mem.file.get {mogId, path}`: one rendered file or item section, for the user.
    pub(super) fn file_get(store: &dyn LocalStore, params: &Value) -> CoreResult<Value> {
        only(params, &["mogId", "path"])?;
        let mog = mog_id(params)?;
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .filter(|p| p.chars().count() <= 300)
            .ok_or_else(bad)?;
        let vpath = memory_files::parse_path(path).ok_or_else(bad)?;
        let (folder, name) = folder_of(store, mog)?;
        let ctx = Ctx {
            folder: &folder,
            mog_name: &name,
            audience: Audience::User,
            offset_s: catalog::current_day_offset(),
        };
        let file_rows = |file: &File| -> CoreResult<Vec<MemoryRow>> {
            let (layer, day) = match file {
                File::User => ("user", None),
                File::Memory { .. } => ("durable", None),
                File::Daily { day, .. } => ("daily", Some(day.clone())),
            };
            Ok(parse_rows(rows(store.exec(
                "mem.item.file_rows",
                json!({ "mogId": mog, "layer": layer, "noteDay": day }),
            )?)))
        };
        // A MOG reads only its own folder (MEM-01). USER.md is shared; slice 2 fills it.
        let own = |file: &File| match file {
            File::User => true,
            File::Memory { folder: f } | File::Daily { folder: f, .. } => *f == folder,
        };
        let text = match vpath {
            VPath::Root => {
                let all = parse_rows(rows(
                    store.exec("mem.item.list_rows", json!({ "mogId": mog, "limit": 500 }))?,
                ));
                let mut out = String::from("# memories\n\n- [USER.md](USER.md)\n");
                for (f, n) in memory_files::files_of(&ctx, &all) {
                    let p = f.path();
                    let rel = p.trim_start_matches("/memories/");
                    out.push_str(&format!("- [{rel}]({rel}) — {n} items\n"));
                }
                out
            }
            VPath::File(file) if own(&file) => {
                memory_files::render_file(&file, &ctx, &file_rows(&file)?)
            }
            VPath::Item { file, slug } if own(&file) => {
                memory_files::render_one(&file, &slug, &ctx, &file_rows(&file)?).ok_or_else(bad)?
            }
            _ => return Err(bad()),
        };
        Ok(json!({ "rows": [{ "path": path, "text": text }] }))
    }
}

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod exchange_tests;
#[cfg(test)]
mod forget_tests;
#[cfg(test)]
mod outbox_tests;
#[cfg(test)]
mod quarantine_tests;
