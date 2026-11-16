//! Sample files (#171): the recorder export and the SP-03 probe log (#5).
//!
//! The sample file has a fixed set of columns: trial number, STT path, route, event name and the
//! time in microseconds. There is no free-text column, so it cannot hold a transcript (SEC-16b).

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use super::marks::{HotkeySamples, KeySampleTool, Mark, Route, SttPath, Trial};

/// The first line of a sample file.
pub const CSV_MAGIC: &str = "# bottle-latency-samples v1";
/// The column line of a sample file.
pub const CSV_HEADER: &str = "trial,path,route,mark,t_us";

/// A sample file line that cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    /// The line number, from 1.
    pub line: usize,
    /// What is wrong.
    pub reason: String,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for ImportError {}

/// Write `trials` as a sample file.
pub fn to_csv(trials: &[Trial]) -> String {
    let mut out = format!("{CSV_MAGIC}\n{CSV_HEADER}\n");
    for t in trials {
        for (mark, at) in t.marks() {
            out.push_str(&format!(
                "{},{},{},{},{}\n",
                t.id().get(),
                t.path().name(),
                t.route().name(),
                mark.name(),
                at.as_micros()
            ));
        }
    }
    out
}

/// Read a sample file. Every row must have the five columns with known values, and the marks of
/// each trial must be in pipeline order.
pub fn parse_csv(text: &str) -> Result<Vec<Trial>, ImportError> {
    let err = |line: usize, reason: String| ImportError { line, reason };
    let mut lines = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty());
    match lines.next() {
        Some((_, l)) if l == CSV_MAGIC => {}
        Some((n, _)) => return Err(err(n, format!("the first line must be `{CSV_MAGIC}`"))),
        None => return Err(err(0, "the file is empty".into())),
    }
    match lines.next() {
        Some((_, l)) if l == CSV_HEADER => {}
        Some((n, _)) => return Err(err(n, format!("the column line must be `{CSV_HEADER}`"))),
        None => return Err(err(0, "the column line is missing".into())),
    }
    let mut trials: BTreeMap<u64, Trial> = BTreeMap::new();
    for (n, line) in lines {
        let cells: Vec<&str> = line.split(',').collect();
        let [id, path, route, mark, t_us] = cells[..] else {
            return Err(err(n, format!("expected 5 columns, found {}", cells.len())));
        };
        let id: u64 = id
            .parse()
            .map_err(|_| err(n, format!("bad trial number `{id}`")))?;
        let path = SttPath::parse(path).ok_or_else(|| err(n, format!("bad path `{path}`")))?;
        let route = Route::parse(route).ok_or_else(|| err(n, format!("bad route `{route}`")))?;
        let mark = Mark::parse(mark).ok_or_else(|| err(n, format!("bad mark `{mark}`")))?;
        let t_us: u64 = t_us
            .parse()
            .map_err(|_| err(n, format!("bad time `{t_us}`")))?;
        let trial = trials
            .entry(id)
            .or_insert_with(|| Trial::new(id, path, route));
        if trial.path() != path || trial.route() != route {
            return Err(err(n, format!("trial {id} changes its path or route")));
        }
        trial
            .record(mark, Duration::from_micros(t_us))
            .map_err(|e| err(n, format!("trial {id}: {e}")))?;
    }
    Ok(trials.into_values().collect())
}

/// Read the talk-key `EV` lines of an SP-03 probe log (`sp03-probe listen`, #5).
///
/// It uses only `src=tap` lines (the Carbon chord is not the talk key). A key-down that is not
/// followed by a key-up before the next key-down, or at the end of the log, is a missed key-up.
/// A numeric, non-negative `lat_ms` is a delivery time. The samples are marked
/// [`KeySampleTool::Sp03Probe`]. TODO(#323): the HotkeyService log on macOS and Windows.
pub fn parse_sp03_log(text: &str) -> HotkeySamples {
    let mut out = HotkeySamples {
        tool: Some(KeySampleTool::Sp03Probe),
        ..HotkeySamples::default()
    };
    let mut pending = false;
    for line in text.lines() {
        let Some(start) = line.find("EV ") else {
            continue;
        };
        let (mut tap, mut kind, mut lat) = (false, None, None);
        for field in line[start + 3..].split_whitespace() {
            match field.split_once('=') {
                Some(("src", v)) => tap = v == "tap",
                Some(("kind", v)) => kind = Some(v),
                Some(("lat_ms", v)) => lat = v.parse::<f64>().ok(),
                _ => {}
            }
        }
        if !tap {
            continue;
        }
        match kind {
            Some("down") => {
                if pending {
                    out.record_cycle(false, &[]);
                }
                pending = true;
            }
            Some("up") => {
                if pending {
                    out.record_cycle(true, &[]);
                    pending = false;
                }
            }
            _ => continue,
        }
        if let Some(ms) = lat.filter(|v| v.is_finite() && *v >= 0.0) {
            out.event_latencies.push(Duration::from_secs_f64(ms / 1e3));
        }
    }
    if pending {
        out.record_cycle(false, &[]);
    }
    out
}

/// The Secure Input part of an SP-03 probe log (AT-30, CAP-39a, CAP-39b, #230).
///
/// The person runs `sp03-probe listen` with Secure Input on (no `--no-sei`), holds the talk key
/// and turns on Secure Keyboard Entry, then presses and releases the chord Ctrl+Shift+Space.
/// The log has only event names and times, no key contents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecureInputSamples {
    /// The probe that wrote the log.
    pub tool: Option<KeySampleTool>,
    /// `src=sei event=on` lines: Secure Input turned on.
    pub secure_input_starts: u32,
    /// For each PTT that Secure Input ended: the upper bound of the time from Secure Input start
    /// to the PTT end (`upper_bound_sei_to_end_ms`, which adds one poll interval).
    pub sei_to_end: Vec<Duration>,
    /// Carbon chord key-downs (`EV src=carbon kind=down`).
    pub chord_presses: u32,
    /// Chord key-downs with a key-up before the next key-down.
    pub chord_releases_seen: u32,
}

/// What the Secure Input samples show against CAP-39a (250 ms) and CAP-39b (press and release).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecureInputVerdict {
    /// Every PTT ended in time and every chord press had its release, in a log of the
    /// HotkeyService that ships.
    Within,
    /// The same as [`SecureInputVerdict::Within`], but the log comes from the SP-03 probe. The
    /// probe is not the HotkeyService that ships, so the AT-30 result is at most `partial`
    /// (the same rule as the #171 bench). TODO(#323).
    WithinProbeOnly,
    /// A PTT ended later than [`crate::hotkey_core::SECURE_INPUT_DEADLINE`].
    Over {
        /// The slowest end.
        worst: Duration,
    },
    /// A chord press had no release.
    ChordReleaseMissed {
        /// Chord presses.
        presses: u32,
        /// Releases seen.
        releases: u32,
    },
    /// No PTT ended by Secure Input, or no chord press: the run did not do the steps.
    NoSamples,
}

impl SecureInputSamples {
    /// The verdict. A run with no sample is never within.
    pub fn verdict(&self) -> SecureInputVerdict {
        if self.sei_to_end.is_empty() || self.chord_presses == 0 {
            return SecureInputVerdict::NoSamples;
        }
        if let Some(worst) = self.sei_to_end.iter().copied().max() {
            if worst > crate::hotkey_core::SECURE_INPUT_DEADLINE {
                return SecureInputVerdict::Over { worst };
            }
        }
        if self.chord_releases_seen < self.chord_presses {
            return SecureInputVerdict::ChordReleaseMissed {
                presses: self.chord_presses,
                releases: self.chord_releases_seen,
            };
        }
        match self.tool {
            Some(KeySampleTool::HotkeyService) => SecureInputVerdict::Within,
            Some(KeySampleTool::Sp03Probe) | None => SecureInputVerdict::WithinProbeOnly,
        }
    }
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .filter_map(|f| f.split_once('='))
        .find_map(|(k, v)| (k == key).then_some(v))
}

/// Read the Secure Input lines of an SP-03 probe log (`sp03-probe listen`, #5).
///
/// It reads `src=sei event=on`, `src=sei event=ptt_end_by_sei upper_bound_sei_to_end_ms=<ms>`
/// and `EV src=carbon kind=down|up`. Other lines are ignored. An end time that is not a finite,
/// non-negative number (or too large for a `Duration`) is skipped. A run with no end time is
/// never within.
pub fn parse_sp03_secure_input(text: &str) -> SecureInputSamples {
    let mut out = SecureInputSamples {
        tool: Some(KeySampleTool::Sp03Probe),
        ..SecureInputSamples::default()
    };
    let mut chord_down = false;
    for line in text.lines() {
        if let Some(start) = line.find("EV ") {
            let ev = &line[start + 3..];
            if field(ev, "src") != Some("carbon") {
                continue;
            }
            match field(ev, "kind") {
                Some("down") => {
                    out.chord_presses = out.chord_presses.saturating_add(1);
                    chord_down = true;
                }
                Some("up") if chord_down => {
                    out.chord_releases_seen = out.chord_releases_seen.saturating_add(1);
                    chord_down = false;
                }
                _ => {}
            }
            continue;
        }
        if field(line, "src") != Some("sei") {
            continue;
        }
        match field(line, "event") {
            Some("on") => out.secure_input_starts = out.secure_input_starts.saturating_add(1),
            Some("ptt_end_by_sei") => {
                let ms = field(line, "upper_bound_sei_to_end_ms")
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v >= 0.0);
                if let Some(d) = ms.and_then(|ms| Duration::try_from_secs_f64(ms / 1e3).ok()) {
                    out.sei_to_end.push(d);
                }
            }
            _ => {}
        }
    }
    out
}
