//! The forward-only migration runner (MEM-35a). Red skeleton for #35.

use rusqlite::Connection;

use crate::error::CoreResult;

/// One forward migration.
pub(crate) struct Migration {
    /// 1, 2, 3 ...
    pub(crate) version: i64,
    /// The file name without `.sql`.
    pub(crate) name: &'static str,
    /// The SQL.
    pub(crate) sql: &'static str,
}

/// Every migration, in order.
pub(crate) const MIGRATIONS: &[Migration] = &[];

/// The newest version this binary knows.
pub(crate) fn latest_version() -> i64 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

/// Bring the schema up to date.
pub(crate) fn run(_conn: &Connection) -> CoreResult<()> {
    Ok(())
}
