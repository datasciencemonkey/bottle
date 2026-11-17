//! The forward-only migration runner (MEM-35a, ADR 0033).
//!
//! `schema_migrations` records each applied version with the app version that applied it. `run`
//! applies every newer migration in one transaction each, in order. A database whose newest
//! version is above what this binary knows is refused with [`CoreError::StoreSchemaNewer`] and left
//! as it is: there is no down migration and no delete (U9).
//!
//! The migration files under `migrations/` are contract files: a later schema change is a new
//! numbered file added through a `contract-change` issue, never an edit of an applied one.

use rusqlite::{params, Connection};

use crate::error::{CoreError, CoreResult};

const ID: &str = "R10";

/// One forward migration.
pub(crate) struct Migration {
    /// 1, 2, 3 ...
    pub(crate) version: i64,
    /// The file name without `.sql`. Only the MEM-35a test reads it: it ties each listed migration
    /// to its file and checks that no file looks like a down migration.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) name: &'static str,
    /// The SQL.
    pub(crate) sql: &'static str,
}

/// Every migration, in order.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_local_mirror",
        sql: include_str!("migrations/0001_local_mirror.sql"),
    },
    Migration {
        version: 2,
        name: "0002_utterance_session_tie",
        sql: include_str!("migrations/0002_utterance_session_tie.sql"),
    },
];

/// The newest version this binary knows.
pub(crate) fn latest_version() -> i64 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

fn unavailable<E>(_: E) -> CoreError {
    CoreError::StoreUnavailable { component: ID }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Bring the schema up to date.
pub(crate) fn run(conn: &mut Connection) -> CoreResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
           version INTEGER PRIMARY KEY,
           applied_at INTEGER NOT NULL,
           app_version TEXT NOT NULL)",
    )
    .map_err(unavailable)?;
    let current: i64 = conn
        .query_row(
            "SELECT coalesce(max(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    if current > latest_version() {
        return Err(CoreError::StoreSchemaNewer { component: ID });
    }
    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        let tx = conn.transaction().map_err(unavailable)?;
        tx.execute_batch(m.sql).map_err(unavailable)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at, app_version) VALUES (?1, ?2, ?3)",
            params![m.version, now_ms(), env!("CARGO_PKG_VERSION")],
        )
        .map_err(unavailable)?;
        tx.commit().map_err(unavailable)?;
    }
    Ok(())
}

/// One KNN on the 256-d index, so the first real recall finds the vec0 pages warm (SP-21).
pub(crate) fn warm_up(conn: &Connection) -> CoreResult<()> {
    let zero = vec![0u8; 256];
    let mut st = conn
        .prepare(
            "SELECT rowid FROM memory_item_vec_256 WHERE embedding MATCH vec_int8(?1) AND k = 1",
        )
        .map_err(unavailable)?;
    let mut rows = st.query([zero]).map_err(unavailable)?;
    while rows.next().map_err(unavailable)?.is_some() {}
    Ok(())
}
