//! T12 ConsentService, Rust side (#63): content-free consent records in the R10 local store.
//!
//! The brain keeps the consent logic (versioned notice, per-session attestation, section 4.3).
//! This module keeps the records (SEC-32a) in the `consent_record` table of migration 0001, and
//! expires them (SEC-15b):
//! - A notice record (9.3 step 5) holds the notice version, the time, the user and the device.
//!   It is device-wide, so its `mog_id` is NULL.
//! - A session record (9.3 step 7) also holds the MOG, the session, the attestation kind and
//!   whether the user copied the disclosure.
//! - No record holds content: no transcript, answer, audio or token (U9 "content-free").
//! - No foreign key ties a record to a session, a transcript or a MOG, so "Delete session",
//!   Forget and Wipe leave it (SEC-32a "records outlive the transcript").
//! - [`ConsentRecords::purge_expired`] is the only delete. It removes records older than the
//!   retention (13 months; admin range 12-84 months, 9.2). WipeService (#124) calls it from
//!   `run_security_expiries`. There is no trigger and no background job (U9).
//!
//! MEM-01: the per-MOG statements filter by `mog_id = ?` and pass the guard. The device-wide
//! statements (a notice has no MOG; the hub's consent list and the purge cross MOGs) are listed
//! in [`STATEMENTS`] as [`Scope::DeviceWide`] with their reason. They touch `consent_record`
//! only, and they never reach `exec` or an agent (ADR 0052).
//! TODO(#37): fold [`STATEMENTS`] into the fixed local catalog with these IDs.
//! TODO(#186): read the admin retention from PolicyStore once Appendix C names its key.
//! TODO(#64): the consent token flow of `session_attest` calls [`ConsentRecords::record_session`].
//! TODO(#27): the first-run notice calls [`ConsentRecords::record_notice`] (needs an IPC command).

use std::sync::Arc;

use rusqlite::{params, OptionalExtension, Row};

use crate::clock::WallClock;
use crate::error::{CoreError, CoreResult};
use crate::ipc::AttestKind;
use crate::local_store::mog_guard::is_uuid;
use crate::local_store::SqlCipherLocalStore;

const ID: &str = "T12";

/// The default consent record retention in months (9.2, SEC-15b).
pub const DEFAULT_RETENTION_MONTHS: u32 = 13;
/// The smallest retention an admin may set (9.2).
pub const MIN_RETENTION_MONTHS: u32 = 12;
/// The largest retention an admin may set (9.2).
pub const MAX_RETENTION_MONTHS: u32 = 84;

/// How long consent records are kept: a whole number of calendar months in 12-84.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentRetention {
    months: u32,
}

impl ConsentRetention {
    /// A retention of `months`. A value outside 12-84 is refused, never clamped.
    pub fn months(months: u32) -> CoreResult<Self> {
        if (MIN_RETENTION_MONTHS..=MAX_RETENTION_MONTHS).contains(&months) {
            Ok(Self { months })
        } else {
            Err(CoreError::Unavailable {
                component: ID,
                reason: "consent_retention_out_of_range",
            })
        }
    }

    /// The number of months.
    pub fn get(self) -> u32 {
        self.months
    }
}

impl Default for ConsentRetention {
    fn default() -> Self {
        Self {
            months: DEFAULT_RETENTION_MONTHS,
        }
    }
}

/// What the user consented to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentKind {
    /// The first-run legal notice of one version (9.3, MB-08a).
    Notice,
    /// The attestation of one meeting session (9.3, CAP-06).
    Session,
}

/// One stored consent record (SEC-32a). Every field is content-free.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsentRecord {
    /// A random UUID v4.
    pub consent_id: String,
    /// The MOG of a session record. `None` for a notice record, which is device-wide.
    pub mog_id: Option<String>,
    /// SEC-32a `user`: the signed-in principal, if known (column `owner_principal`).
    pub user: Option<String>,
    /// SEC-32a `device` (column `device_id`).
    pub device: String,
    /// SEC-32a `notice_version`.
    pub notice_version: u32,
    /// Notice or session.
    pub kind: ConsentKind,
    /// SEC-32a `session_id`. Only a session record has it.
    pub session_id: Option<String>,
    /// SEC-32a `attestation_kind`. Only a session record has it.
    pub attestation_kind: Option<AttestKind>,
    /// 9.3 step 7 `disclosure_copied`. Only a session record has it.
    pub disclosure_copied: Option<bool>,
    /// SEC-32a `accepted_at` (9.3 `consent_ts`), Unix ms (column `recorded_at`).
    pub accepted_at: i64,
    /// `accepted_at` plus the retention in effect when the record was written, Unix ms. The
    /// purge uses the current retention (see [`ConsentRecords::purge_expired`]).
    pub expires_at: i64,
}

/// A session attestation to store (9.3 step 7).
#[derive(Debug, Clone)]
pub struct SessionAttestation<'a> {
    /// The MOG that runs the session (a UUID).
    pub mog_id: &'a str,
    /// The session (a UUID).
    pub session_id: &'a str,
    /// The radio choice of the attestation card.
    pub kind: AttestKind,
    /// The notice version the user accepted.
    pub notice_version: u32,
    /// The signed-in principal, if known.
    pub user: Option<&'a str>,
    /// Whether the user selected [Copy disclosure].
    pub disclosure_copied: bool,
}

/// How a statement keeps MEM-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// It filters by `mog_id = ?` and passes `require_mog_id_filter`.
    Mog,
    /// It is device-wide on purpose, for the reason given. It touches `consent_record` only.
    DeviceWide(&'static str),
}

/// One fixed, parameterised statement.
#[derive(Debug, Clone, Copy)]
pub struct Stmt {
    /// The catalog ID, for example `consent.purge`.
    pub id: &'static str,
    /// The SQL, with bound parameters only.
    pub sql: &'static str,
    /// How it keeps MEM-01.
    pub scope: Scope,
}

const SESSION_INSERT: Stmt = Stmt {
    id: "consent.session.insert",
    sql: "INSERT INTO consent_record (consent_id, mog_id, owner_principal, device_id, \
          notice_version, kind, session_id, attestation_kind, disclosure_copied, recorded_at, \
          expires_at, updated_at) \
          VALUES (?1, ?2, ?3, ?4, ?5, 'session', ?6, ?7, ?8, ?9, ?10, ?9)",
    scope: Scope::Mog,
};

const SESSION_GET: Stmt = Stmt {
    id: "consent.session.get",
    sql: "SELECT consent_id, mog_id, owner_principal, device_id, notice_version, kind, \
          session_id, attestation_kind, disclosure_copied, recorded_at, expires_at \
          FROM consent_record WHERE mog_id = ?1 AND session_id = ?2 AND kind = 'session' \
          ORDER BY recorded_at DESC, consent_id LIMIT 1",
    scope: Scope::Mog,
};

const NOTICE_INSERT: Stmt = Stmt {
    id: "consent.notice.insert",
    sql: "INSERT INTO consent_record (consent_id, mog_id, owner_principal, device_id, \
          notice_version, kind, recorded_at, expires_at, updated_at) \
          VALUES (?1, NULL, ?2, ?3, ?4, 'notice', ?5, ?6, ?5)",
    scope: Scope::DeviceWide("a notice acceptance is device-wide: it has no MOG (mog_id NULL)"),
};

const NOTICE_LATEST: Stmt = Stmt {
    id: "consent.notice.latest",
    sql: "SELECT max(CAST(notice_version AS INTEGER)) FROM consent_record \
          WHERE kind = 'notice' AND mog_id IS NULL",
    scope: Scope::DeviceWide("the notice gate (MB-08a) is device-wide"),
};

const LIST: Stmt = Stmt {
    id: "consent.list",
    sql: "SELECT consent_id, mog_id, owner_principal, device_id, notice_version, kind, \
          session_id, attestation_kind, disclosure_copied, recorded_at, expires_at \
          FROM consent_record ORDER BY recorded_at, consent_id",
    scope: Scope::DeviceWide("the hub lists the device's consent records; they hold no content"),
};

const PURGE: Stmt = Stmt {
    id: "consent.purge",
    sql: "DELETE FROM consent_record WHERE recorded_at <= ?1",
    scope: Scope::DeviceWide("the SEC-15b security expiry runs for the whole device (U9)"),
};

/// Every consent statement, for the #37 catalog. The MEM-01 test checks each one.
pub const STATEMENTS: &[Stmt] = &[
    SESSION_INSERT,
    SESSION_GET,
    NOTICE_INSERT,
    NOTICE_LATEST,
    LIST,
    PURGE,
];

const DAY_MS: i64 = 86_400_000;

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number (H. Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

fn days_in_month(y: i64, m: i64) -> i64 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)
}

/// `ms` (Unix ms, UTC) plus `months` calendar months. The day of the month clamps to the last
/// day of the target month (Jan 31 + 1 month = Feb 28 or 29); the time of day stays.
pub fn add_months_utc(ms: i64, months: i32) -> i64 {
    let day = ms.div_euclid(DAY_MS);
    let time = ms.rem_euclid(DAY_MS);
    let (y, m, d) = civil_from_days(day);
    let total = y * 12 + (m - 1) + i64::from(months);
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let nd = d.min(days_in_month(ny, nm));
    days_from_civil(ny, nm, nd)
        .saturating_mul(DAY_MS)
        .saturating_add(time)
}

fn months_i32(r: ConsentRetention) -> i32 {
    // 12-84 always fits.
    i32::try_from(r.get()).unwrap_or(i32::MAX)
}

fn new_consent_id() -> CoreResult<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|_| CoreError::StoreUnavailable { component: ID })?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

fn attest_str(k: &AttestKind) -> &'static str {
    match k {
        AttestKind::OnlyMe => "only_me",
        AttestKind::OthersAgreed => "others_agreed",
    }
}

fn bad_row(i: usize) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(i, String::new(), rusqlite::types::Type::Text)
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<ConsentRecord> {
    let kind = match r.get::<_, String>(5)?.as_str() {
        "notice" => ConsentKind::Notice,
        "session" => ConsentKind::Session,
        _ => return Err(bad_row(5)),
    };
    let attestation_kind = match r.get::<_, Option<String>>(7)?.as_deref() {
        None => None,
        Some("only_me") => Some(AttestKind::OnlyMe),
        Some("others_agreed") => Some(AttestKind::OthersAgreed),
        Some(_) => return Err(bad_row(7)),
    };
    let notice_version = r
        .get::<_, String>(4)?
        .parse::<u32>()
        .map_err(|_| bad_row(4))?;
    Ok(ConsentRecord {
        consent_id: r.get(0)?,
        mog_id: r.get(1)?,
        user: r.get(2)?,
        device: r.get(3)?,
        notice_version,
        kind,
        session_id: r.get(6)?,
        attestation_kind,
        disclosure_copied: r.get::<_, Option<i64>>(8)?.map(|v| v != 0),
        accepted_at: r.get(9)?,
        expires_at: r.get(10)?,
    })
}

/// The consent records of this device, in the R10 local store.
pub struct ConsentRecords {
    store: Arc<SqlCipherLocalStore>,
    clock: Arc<dyn WallClock>,
    device_id: String,
    retention: ConsentRetention,
}

impl ConsentRecords {
    /// Records for the device `device_id` (a UUID) in `store`, on `clock`, kept for `retention`.
    pub fn new(
        store: Arc<SqlCipherLocalStore>,
        clock: Arc<dyn WallClock>,
        device_id: &str,
        retention: ConsentRetention,
    ) -> CoreResult<Self> {
        if !is_uuid(device_id) {
            return Err(CoreError::Unavailable {
                component: ID,
                reason: "device_id_invalid",
            });
        }
        Ok(Self {
            store,
            clock,
            device_id: device_id.to_owned(),
            retention,
        })
    }

    /// The retention in effect.
    pub fn retention(&self) -> ConsentRetention {
        self.retention
    }

    fn stamp(&self) -> CoreResult<(String, i64, i64)> {
        let now = self.clock.now_ms();
        let expires = add_months_utc(now, months_i32(self.retention));
        Ok((new_consent_id()?, now, expires))
    }

    /// Store the acceptance of notice `notice_version` (9.3 step 5, SEC-32a).
    pub fn record_notice(
        &self,
        user: Option<&str>,
        notice_version: u32,
    ) -> CoreResult<ConsentRecord> {
        let (id, now, expires) = self.stamp()?;
        let version = notice_version.to_string();
        self.store.with_conn(|c| {
            c.execute(
                NOTICE_INSERT.sql,
                params![id, user, self.device_id, version, now, expires],
            )
        })?;
        Ok(ConsentRecord {
            consent_id: id,
            mog_id: None,
            user: user.map(str::to_owned),
            device: self.device_id.clone(),
            notice_version,
            kind: ConsentKind::Notice,
            session_id: None,
            attestation_kind: None,
            disclosure_copied: None,
            accepted_at: now,
            expires_at: expires,
        })
    }

    /// Store one session attestation (9.3 step 7, SEC-32a).
    pub fn record_session(&self, a: &SessionAttestation<'_>) -> CoreResult<ConsentRecord> {
        if !is_uuid(a.mog_id) {
            return Err(CoreError::MogIdRequired { component: "R10" });
        }
        if !is_uuid(a.session_id) {
            return Err(CoreError::Unavailable {
                component: ID,
                reason: "session_id_invalid",
            });
        }
        let (id, now, expires) = self.stamp()?;
        let version = a.notice_version.to_string();
        self.store.with_conn(|c| {
            c.execute(
                SESSION_INSERT.sql,
                params![
                    id,
                    a.mog_id,
                    a.user,
                    self.device_id,
                    version,
                    a.session_id,
                    attest_str(&a.kind),
                    i64::from(a.disclosure_copied),
                    now,
                    expires
                ],
            )
        })?;
        Ok(ConsentRecord {
            consent_id: id,
            mog_id: Some(a.mog_id.to_owned()),
            user: a.user.map(str::to_owned),
            device: self.device_id.clone(),
            notice_version: a.notice_version,
            kind: ConsentKind::Session,
            session_id: Some(a.session_id.to_owned()),
            attestation_kind: Some(a.kind.clone()),
            disclosure_copied: Some(a.disclosure_copied),
            accepted_at: now,
            expires_at: expires,
        })
    }

    /// The highest notice version with a stored acceptance, or `None` (MB-08a, MB-08b).
    pub fn latest_notice_version(&self) -> CoreResult<Option<u32>> {
        let v: Option<i64> = self
            .store
            .with_conn(|c| c.query_row(NOTICE_LATEST.sql, [], |r| r.get(0)))?;
        Ok(v.and_then(|v| u32::try_from(v).ok()))
    }

    /// The latest record of `session_id` in the MOG `mog_id` (MEM-01: another MOG sees none).
    pub fn session_record(
        &self,
        mog_id: &str,
        session_id: &str,
    ) -> CoreResult<Option<ConsentRecord>> {
        if !is_uuid(mog_id) {
            return Err(CoreError::MogIdRequired { component: "R10" });
        }
        self.store.with_conn(|c| {
            c.query_row(SESSION_GET.sql, params![mog_id, session_id], from_row)
                .optional()
        })
    }

    /// Every consent record of the device, oldest first.
    pub fn records(&self) -> CoreResult<Vec<ConsentRecord>> {
        self.store.with_conn(|c| {
            let mut st = c.prepare(LIST.sql)?;
            let rows = st.query_map([], from_row)?;
            rows.collect()
        })
    }

    /// SEC-15b: delete the records whose age is the retention or more, under the current
    /// retention. Returns the number deleted. Only `consent_record` rows go; no user content
    /// (U9). WipeService `run_security_expiries` (#124) calls it.
    ///
    /// The cutoff is now minus the retention in calendar months. Where the day clamps (for
    /// example a record of 31 March and a 30-day target month), a record can stay up to 3 days
    /// past its `expires_at`; it is never deleted early.
    pub fn purge_expired(&self) -> CoreResult<usize> {
        let cutoff = add_months_utc(self.clock.now_ms(), -months_i32(self.retention));
        self.store
            .with_conn(|c| c.execute(PURGE.sql, params![cutoff]))
    }
}

#[cfg(test)]
mod tests;
