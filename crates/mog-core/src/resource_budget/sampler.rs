//! An RSS sampler on any probe (#173). The runner gives a probe that reads `ps` (macOS) or
//! `tasklist` (Windows); the tests give [`super::doubles::ScriptedProbe`] and the #229 `SimClock`.

use std::time::Duration;

use super::samples::{RssRun, MAX_MOGS};

/// Something that reads the summed RSS of the Bottle processes, in bytes.
pub trait RssProbe {
    /// One sample, bytes. An error stops the run.
    fn sample(&mut self) -> Result<u64, String>;
}

/// Take `count` samples from `probe`, with `wait(interval)` between two samples (not after the
/// last one). `wait` is `std::thread::sleep` in the runner and `SimClock::advance` in tests.
pub fn collect_rss(
    probe: &mut dyn RssProbe,
    mogs: u8,
    count: usize,
    interval: Duration,
    wait: &mut dyn FnMut(Duration),
) -> Result<RssRun, String> {
    if mogs > MAX_MOGS {
        return Err(format!("mogs must be 0..={MAX_MOGS}"));
    }
    if count == 0 {
        return Err("count must be 1 or more".into());
    }
    let mut bytes = Vec::with_capacity(count);
    for i in 0..count {
        if i > 0 {
            wait(interval);
        }
        bytes.push(probe.sample()?);
    }
    Ok(RssRun { mogs, bytes })
}
