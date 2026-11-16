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
