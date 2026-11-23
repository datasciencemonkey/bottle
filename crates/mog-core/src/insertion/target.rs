//! ADR 0118 decisions 16-21 (UX slice 2): the paste target lock.
//!
//! At the talk-key down (or the hands-free start) Rust notes the app that has the focus. When R2
//! grants the PTT or hands-free lease, the host binds that note to the lease in the
//! [`TargetTable`]. `insert_text` pastes only if the app that has the focus now is that app.
//!
//! Fail closed: no entry, an evicted or expired entry, another MOG, an app id of pid 0 (no note
//! at grant, or a failed AX or Win32 read) all give no expected app, so the paste answers
//! `target_changed`. The pid never crosses IPC and never goes into a log: [`AppId`] redacts it
//! in `Debug`, and nothing here logs.
//!
//! Only a PTT or hands-free lease gets an entry (the host binds only those). A recording or
//! meeting primary lease gets none, so a paste with its lease answers `target_changed`. The
//! target of a hands-free session stays fixed for the whole session.

use crate::capture::{parse_lease, LeaseNo, HANDS_FREE_MAX_MS};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

/// The most entries in the table. When it is full, the oldest entry is evicted.
pub const TABLE_MAX: usize = 64;
/// An entry lives this long after its lease stops (10 min).
pub const STOP_GRACE_MS: u64 = 600_000;
/// An entry lives at most this long after the grant: the hands-free limit plus 10 min.
pub const HARD_LIMIT_MS: u64 = HANDS_FREE_MAX_MS + STOP_GRACE_MS;
/// A key-down note older than this at the grant is not used (the grant is not its press).
pub const NOTE_FRESH_MS: u64 = 2_000;

/// An app, as the OS names it: the pid, and on Windows the process creation time (so a reused
/// pid is a different app). A pid of 0 is "no app": it never matches anything.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AppId {
    pid: u32,
    created: u64,
}

impl AppId {
    /// No app: the read failed, or nothing has the focus.
    pub const NONE: AppId = AppId { pid: 0, created: 0 };

    /// An app of `pid` created at `created` (0 where the OS gives no creation time).
    pub const fn new(pid: u32, created: u64) -> Self {
        Self { pid, created }
    }

    /// Whether this is [`AppId::NONE`].
    pub const fn is_none(&self) -> bool {
        self.pid == 0
    }

    /// Whether `self` is a real app and the same app as `other` (`NONE` matches nothing).
    pub fn is_same_app(&self, other: AppId) -> bool {
        !self.is_none() && *self == other
    }
}

impl std::fmt::Debug for AppId {
    /// The pid never goes into a log (ADR 0118, decision 21).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppId(..)")
    }
}

/// Milliseconds on a process-local monotonic clock.
pub fn now_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

struct Note {
    gen: u64,
    at: u64,
    /// `None` while the read of the focused app still runs.
    app: Option<AppId>,
}

/// The app of an entry: known, or the read of note `gen` still runs when the lease is granted.
#[derive(Clone, Copy)]
enum Bound {
    Done(AppId),
    Pending(u64),
}

struct Entry {
    lease: LeaseNo,
    mog: String,
    app: Bound,
    hard_deadline: u64,
    stopped_at: Option<u64>,
}

impl Entry {
    fn expired(&self, now: u64) -> bool {
        now >= self.hard_deadline
            || self
                .stopped_at
                .is_some_and(|s| now >= s.saturating_add(STOP_GRACE_MS))
    }
}

struct Inner {
    gen: u64,
    note: Option<Note>,
    /// In grant order: the first is the oldest.
    entries: Vec<Entry>,
}

/// The bounded table of paste targets, one mutex. No call holds it across a paste.
pub struct TargetTable {
    inner: Mutex<Inner>,
}

impl TargetTable {
    /// An empty table.
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                gen: 0,
                note: None,
                entries: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A talk key went down at `now`: drop any earlier note and start a new one. Returns its
    /// number for [`TargetTable::finish_note`]. The read of the app runs after this, off the
    /// hook thread.
    pub fn begin_note(&self, now: u64) -> u64 {
        let mut g = self.lock();
        g.gen += 1;
        let gen = g.gen;
        g.note = Some(Note {
            gen,
            at: now,
            app: None,
        });
        gen
    }

    /// The read of note `gen` ended with `app` ([`AppId::NONE`] on an error). A newer press
    /// has made the note stale: then this does nothing.
    pub fn finish_note(&self, gen: u64, app: AppId) {
        let mut g = self.lock();
        if let Some(n) = g.note.as_mut().filter(|n| n.gen == gen) {
            n.app = Some(app);
        }
        // An entry bound while this read ran gets its result now (it holds the note, not a value).
        for e in g.entries.iter_mut() {
            if matches!(e.app, Bound::Pending(n) if n == gen) {
                e.app = Bound::Done(app);
            }
        }
    }

    /// Bind the note of the last key down to `lease` of `mog`, at the grant. The note is used
    /// once, and only if it is not older than [`NOTE_FRESH_MS`]. A note whose read still runs
    /// is bound as pending: [`TargetTable::finish_note`] fills it, and until then every paste
    /// fails closed. With no fresh note the entry holds [`AppId::NONE`] and fails closed.
    pub fn bind(&self, lease: LeaseNo, mog: &str, now: u64) {
        let mut g = self.lock();
        let app = match g.note.take() {
            Some(Note { at, app, gen }) if now.saturating_sub(at) <= NOTE_FRESH_MS => match app {
                Some(app) => Bound::Done(app),
                None => Bound::Pending(gen),
            },
            _ => Bound::Done(AppId::NONE),
        };
        g.entries.retain(|e| !e.expired(now) && e.lease != lease);
        while g.entries.len() >= TABLE_MAX {
            g.entries.remove(0);
        }
        g.entries.push(Entry {
            lease,
            mog: mog.to_owned(),
            app,
            hard_deadline: now.saturating_add(HARD_LIMIT_MS),
            stopped_at: None,
        });
    }

    /// The lease stopped: its entry ends [`STOP_GRACE_MS`] from `now`.
    pub fn stopped(&self, lease: LeaseNo, now: u64) {
        if let Some(e) = self.lock().entries.iter_mut().find(|e| e.lease == lease) {
            e.stopped_at.get_or_insert(now);
        }
    }

    /// Forget every entry and the note (`capture_stop_all`, MOG close, quit).
    pub fn clear(&self) {
        let mut g = self.lock();
        g.entries.clear();
        g.note = None;
    }

    /// Forget the entries of `mog` (its window closed).
    pub fn clear_mog(&self, mog: &str) {
        self.lock().entries.retain(|e| e.mog != mog);
    }

    /// The app that `insert_text` of `mog` with wire lease `lease_id` may paste into at `now`.
    /// [`AppId::NONE`] for every other case: this is the fail-closed answer.
    pub fn expected(&self, lease_id: &str, mog: &str, now: u64) -> AppId {
        let Ok(lease) = parse_lease(lease_id) else {
            return AppId::NONE;
        };
        self.lock()
            .entries
            .iter()
            .find(|e| e.lease == lease)
            .filter(|e| !e.expired(now) && e.mog == mog)
            .map_or(AppId::NONE, |e| match e.app {
                Bound::Done(app) => app,
                // The read has not ended: fail closed.
                Bound::Pending(_) => AppId::NONE,
            })
    }

    /// How many entries the table holds.
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Whether the table holds no entry.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for TargetTable {
    /// No entry, no pid (ADR 0118, decision 21).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TargetTable").finish_non_exhaustive()
    }
}

impl Default for TargetTable {
    fn default() -> Self {
        Self::new()
    }
}

static TARGETS: TargetTable = TargetTable::new();

/// The one table of the process.
pub fn targets() -> &'static TargetTable {
    &TARGETS
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: AppId = AppId::new(4242, 9);
    const OTHER: AppId = AppId::new(5151, 9);
    const MOG: &str = "00000000-0000-4000-8000-0000000000a1";

    fn noted(t: &TargetTable, app: AppId, at: u64) {
        let g = t.begin_note(at);
        t.finish_note(g, app);
    }

    #[test]
    fn ux2_a_note_bound_at_grant_is_the_expected_app() {
        let t = TargetTable::new();
        noted(&t, A, 100);
        t.bind(1, MOG, 150);
        assert_eq!(t.expected("1", MOG, 200), A);
    }

    #[test]
    fn ux2_no_note_a_stale_note_or_a_note_in_flight_binds_none() {
        let t = TargetTable::new();
        t.bind(1, MOG, 0);
        assert!(t.expected("1", MOG, 1).is_none());
        noted(&t, A, 0);
        t.bind(2, MOG, NOTE_FRESH_MS + 1);
        assert!(t.expected("2", MOG, NOTE_FRESH_MS + 2).is_none());
    }

    #[test]
    fn ux2_m1_a_note_in_flight_at_grant_is_bound_and_filled_when_the_read_ends() {
        let t = TargetTable::new();
        let g = t.begin_note(10);
        t.bind(3, MOG, 20);
        assert!(t.expected("3", MOG, 21).is_none(), "pending: fail closed");
        t.finish_note(g, A);
        assert_eq!(t.expected("3", MOG, 22), A, "filled by the read");
    }

    #[test]
    fn ux2_m1_a_new_press_does_not_fill_an_entry_of_an_older_note() {
        let t = TargetTable::new();
        let old = t.begin_note(10);
        t.bind(3, MOG, 11);
        let new = t.begin_note(12);
        t.finish_note(new, OTHER);
        assert!(t.expected("3", MOG, 13).is_none(), "still the older read");
        t.finish_note(old, A);
        assert_eq!(t.expected("3", MOG, 14), A);
    }

    #[test]
    fn ux2_m1_a_read_that_never_ends_stays_closed() {
        let t = TargetTable::new();
        t.begin_note(10);
        t.bind(3, MOG, 11);
        assert!(t.expected("3", MOG, 10_000_000).is_none());
    }

    #[test]
    fn ux2_a_note_is_used_once_and_a_new_press_voids_the_old_read() {
        let t = TargetTable::new();
        noted(&t, A, 0);
        t.bind(1, MOG, 1);
        t.bind(2, MOG, 2);
        assert!(t.expected("2", MOG, 3).is_none(), "second grant: no note");
        let old = t.begin_note(10);
        let new = t.begin_note(20);
        t.finish_note(old, A);
        t.bind(3, MOG, 21);
        assert!(t.expected("3", MOG, 22).is_none(), "old read is stale");
        t.finish_note(new, A);
    }

    #[test]
    fn ux2_fail_closed_cases_give_none() {
        let t = TargetTable::new();
        noted(&t, A, 0);
        t.bind(1, MOG, 1);
        assert_eq!(t.expected("1", MOG, 2), A);
        assert!(t.expected("2", MOG, 2).is_none(), "unknown lease");
        assert!(t.expected("x", MOG, 2).is_none(), "parse failure");
        assert!(t.expected("01", MOG, 2).is_none(), "not a lease R2 issued");
        assert!(t.expected("0", MOG, 2).is_none());
        assert!(t.expected("1", "other-mog", 2).is_none(), "another MOG");
        assert!(
            !AppId::NONE.is_same_app(AppId::NONE),
            "NONE matches nothing"
        );
    }

    #[test]
    fn ux2_an_entry_ends_10_min_after_the_stop_or_at_the_hard_limit() {
        let t = TargetTable::new();
        noted(&t, A, 0);
        t.bind(1, MOG, 0);
        t.stopped(1, 1_000);
        assert_eq!(t.expected("1", MOG, 1_000 + STOP_GRACE_MS - 1), A);
        assert!(t.expected("1", MOG, 1_000 + STOP_GRACE_MS).is_none());
        noted(&t, A, 0);
        t.bind(2, MOG, 0);
        assert_eq!(t.expected("2", MOG, HARD_LIMIT_MS - 1), A);
        assert!(t.expected("2", MOG, HARD_LIMIT_MS).is_none());
        assert_eq!(HARD_LIMIT_MS, HANDS_FREE_MAX_MS + 600_000);
    }

    #[test]
    fn ux2_the_table_keeps_64_entries_and_evicts_the_oldest() {
        let t = TargetTable::new();
        for n in 1..=(TABLE_MAX as u64 + 1) {
            noted(&t, A, n);
            t.bind(n, MOG, n);
        }
        assert_eq!(t.len(), TABLE_MAX);
        assert!(t.expected("1", MOG, 100).is_none(), "evicted");
        assert_eq!(t.expected("2", MOG, 100), A);
        assert_eq!(t.expected("65", MOG, 100), A);
    }

    #[test]
    fn ux2_clear_and_clear_mog_forget_entries() {
        let t = TargetTable::new();
        noted(&t, A, 0);
        t.bind(1, MOG, 0);
        noted(&t, A, 0);
        t.bind(2, "other", 0);
        t.clear_mog(MOG);
        assert!(t.expected("1", MOG, 1).is_none());
        assert_eq!(t.expected("2", "other", 1), A);
        t.clear();
        assert!(t.is_empty());
        assert!(t.expected("2", "other", 1).is_none());
    }

    #[test]
    fn ux2_sec_the_app_id_debug_holds_no_pid() {
        let a = AppId::new(987_654, 123_456_789);
        let shown = format!("{a:?} {:?}", Some(a));
        assert!(!shown.contains("987654") && !shown.contains("123456789"));
    }
}
