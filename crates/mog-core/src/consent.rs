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
//! MEM-01: the statements are the [`catalog::CONSENT`] entry set of the fixed local catalog
//! (#431, ADR 0071). The per-MOG statements filter by `mog_id = :mogId` and pass the guard. The
//! device-wide statements (a notice has no MOG; the consent list and the purge cross MOGs) are
//! [`catalog::Scope::DeviceWide`] with their reason. They touch `consent_record` only, and they
//! never reach `exec`, `mem_local_exec` or an agent (ADR 0052).
//! The first-run notice (#27) reaches [`ConsentRecords::record_notice`] through
//! `consent_notice_record` (#430, `consent_ipc`). The host opens the records with
//! [`open_device_records`] (#431).
//! TODO(#186): read the admin retention from PolicyStore once Appendix C names its key.
//! TODO(#64): the consent token flow of `session_attest` calls [`ConsentRecords::record_session`].

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use rusqlite::Row;
use serde_json::{json, Value};

use crate::clock::WallClock;
use crate::error::{CoreError, CoreResult};
use crate::ipc::AttestKind;
use crate::local_store::catalog::{self, ConsentStmt};
use crate::local_store::key_vault::KeyVault;
use crate::local_store::mog_guard::is_uuid;
use crate::local_store::{LocalStore, SqlCipherLocalStore};

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

/// A random UUID v4 from the OS random source.
fn new_uuid_v4() -> CoreResult<String> {
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

/// The folder of the R10 database in the app data folder (MEM-08a: one file for all MOGs).
pub const MEMORY_DIR: &str = "memory";
/// The file of the device ID in the app data folder (SEC-32a `device`).
pub const DEVICE_ID_FILE: &str = "device-id";

/// The device ID in the folder `dir`: a random UUID v4, made on the first call and kept in
/// [`DEVICE_ID_FILE`]. A file that does not hold a UUID fails with `device_id_invalid`, and it
/// is never overwritten, so a record never gets a second device by accident (ADR 0071).
pub fn device_id_in(dir: &Path) -> CoreResult<String> {
    let invalid = CoreError::Unavailable {
        component: ID,
        reason: "device_id_invalid",
    };
    let io = |_| CoreError::StoreUnavailable { component: ID };
    std::fs::create_dir_all(dir).map_err(io)?;
    let path = dir.join(DEVICE_ID_FILE);
    let read = |path: &Path| -> CoreResult<String> {
        let text = std::fs::read_to_string(path).map_err(io)?;
        let id = text.trim();
        if is_uuid(id) {
            Ok(id.to_owned())
        } else {
            Err(invalid.clone())
        }
    };
    if path.exists() {
        return read(&path);
    }
    let id = new_uuid_v4()?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => {
            f.write_all(id.as_bytes()).map_err(io)?;
            f.sync_all().map_err(io)?;
            Ok(id)
        }
        // Another start made it first: use that one.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read(&path),
        Err(_) => Err(CoreError::StoreUnavailable { component: ID }),
    }
}

/// Open the device's consent records in the app data folder `dir` (#431, ADR 0071): the one
/// R10 store in `<dir>/memory` with the key from `vault`, and the device ID of
/// [`device_id_in`]. Any failure (no OS key store, a lost key, a damaged device ID) opens
/// nothing; the host then manages no records and `consent_*` answers `consent_storage_failed`.
pub fn open_device_records(
    dir: &Path,
    vault: Arc<dyn KeyVault>,
    clock: Arc<dyn WallClock>,
    retention: ConsentRetention,
) -> CoreResult<(Arc<SqlCipherLocalStore>, ConsentRecords)> {
    let store = Arc::new(SqlCipherLocalStore::new(dir.join(MEMORY_DIR), vault));
    store.open()?;
    let device = device_id_in(dir)?;
    let records = ConsentRecords::new(Arc::clone(&store), clock, &device, retention)?;
    Ok((store, records))
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

    /// Run one catalog consent statement: check `params` against its declared types, bind each
    /// `:name`, and pass the prepared statement to `f` (MEM-34a: fixed SQL, bound values only).
    fn run<T>(
        &self,
        s: &ConsentStmt,
        params: &Value,
        f: impl FnOnce(&mut rusqlite::Statement<'_>) -> rusqlite::Result<T>,
    ) -> CoreResult<T> {
        let values = catalog::check_params(s.stmt.params, params)?;
        self.store.with_conn(|c| {
            let mut st = c.prepare_cached(s.stmt.sql)?;
            for (name, value) in &values {
                if let Some(i) = st.parameter_index(&format!(":{name}"))? {
                    st.raw_bind_parameter(i, value)?;
                }
            }
            f(&mut st)
        })
    }

    fn stamp(&self) -> CoreResult<(String, i64, i64)> {
        let now = self.clock.now_ms();
        let expires = add_months_utc(now, months_i32(self.retention));
        Ok((new_uuid_v4()?, now, expires))
    }

    /// Store the acceptance of notice `notice_version` (9.3 step 5, SEC-32a).
    pub fn record_notice(
        &self,
        user: Option<&str>,
        notice_version: u32,
    ) -> CoreResult<ConsentRecord> {
        let (id, now, expires) = self.stamp()?;
        let version = notice_version.to_string();
        let p = json!({
            "consentId": id,
            "user": user,
            "deviceId": self.device_id,
            "noticeVersion": version,
            "now": now,
            "expiresAt": expires,
        });
        self.run(&catalog::CONSENT_NOTICE_INSERT, &p, |st| st.raw_execute())?;
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
        let p = json!({
            "consentId": id,
            "mogId": a.mog_id,
            "user": a.user,
            "deviceId": self.device_id,
            "noticeVersion": version,
            "sessionId": a.session_id,
            "attestationKind": attest_str(&a.kind),
            "disclosureCopied": a.disclosure_copied,
            "now": now,
            "expiresAt": expires,
        });
        self.run(&catalog::CONSENT_SESSION_INSERT, &p, |st| st.raw_execute())?;
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
        let v: Option<i64> = self.run(&catalog::CONSENT_NOTICE_LATEST, &json!({}), |st| {
            let mut rows = st.raw_query();
            match rows.next()? {
                Some(r) => r.get(0),
                None => Ok(None),
            }
        })?;
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
        let p = json!({ "mogId": mog_id, "sessionId": session_id });
        self.run(&catalog::CONSENT_SESSION_GET, &p, |st| {
            let mut rows = st.raw_query();
            rows.next()?.map(from_row).transpose()
        })
    }

    /// Every consent record of the device, oldest first.
    pub fn records(&self) -> CoreResult<Vec<ConsentRecord>> {
        self.run(&catalog::CONSENT_LIST, &json!({}), |st| {
            let mut rows = st.raw_query();
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(from_row(r)?);
            }
            Ok(out)
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
        self.run(
            &catalog::CONSENT_PURGE,
            &json!({ "cutoff": cutoff }),
            |st| st.raw_execute(),
        )
    }
}

#[cfg(test)]
mod tests;
