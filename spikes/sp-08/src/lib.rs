//! Spike SP-08 (#15): pure helpers for the mic-indicator probe.
//!
//! The probe binary (`src/main.rs`) opens the default input device with cpal. This library
//! holds the parts that run without a device: the argument parser, the percentile math,
//! the latency arithmetic, the CoreAudio four-char codes and the evidence verdict.

use std::fmt;

/// How the probe builds and starts the input stream in one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Build the input stream and never call `play()`. Hold it for `--hold` seconds.
    BuildOnly,
    /// Build the stream, wait `--wait` seconds, then call `play()` at a simulated key-down.
    BuildThenPlay,
    /// Build the stream, call `pause()` at once, wait, then `play()` at a simulated key-down.
    BuildPauseThenPlay,
    /// At a simulated key-down, build the stream and call `play()` (the SP-08 fallback).
    Cold,
}

impl Mode {
    /// Every mode, in the order `--mode all` runs them.
    pub const ALL: [Mode; 4] = [
        Mode::BuildOnly,
        Mode::BuildThenPlay,
        Mode::BuildPauseThenPlay,
        Mode::Cold,
    ];

    /// The command-line name of the mode.
    pub fn name(self) -> &'static str {
        match self {
            Mode::BuildOnly => "build-only",
            Mode::BuildThenPlay => "build-then-play",
            Mode::BuildPauseThenPlay => "build-pause-then-play",
            Mode::Cold => "cold",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What `--mode` selects: one mode or all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeArg {
    One(Mode),
    All,
}

/// Parse a `--mode` value.
pub fn parse_mode(s: &str) -> Result<ModeArg, String> {
    if s == "all" {
        return Ok(ModeArg::All);
    }
    Mode::ALL
        .into_iter()
        .find(|m| m.name() == s)
        .map(ModeArg::One)
        .ok_or_else(|| {
            format!("unknown mode {s:?}: use build-only, build-then-play, build-pause-then-play, cold or all")
        })
}

fn parse_secs(flag: &str, v: &str) -> Result<f64, String> {
    match v.parse::<f64>() {
        Ok(x) if x.is_finite() && x >= 0.0 => Ok(x),
        _ => Err(format!("{flag} needs a number of seconds >= 0, got {v:?}")),
    }
}

/// The probe command.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Run the stream probe.
    Probe(ProbeArgs),
    /// Print the CoreAudio device-running flags once and exit.
    DeviceState,
    /// Print the usage text.
    Help,
}

/// Arguments of the stream probe.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeArgs {
    pub mode: ModeArg,
    /// K: runs per mode.
    pub runs: u32,
    /// N: seconds to hold the stream after the last step of a run.
    pub hold_s: f64,
    /// M: seconds from build to the simulated key-down (build-then-play, build-pause-then-play).
    pub wait_s: f64,
    /// Seconds to sleep between runs, so the device can go idle.
    pub gap_s: f64,
}

impl Default for ProbeArgs {
    fn default() -> Self {
        ProbeArgs {
            mode: ModeArg::All,
            runs: 5,
            hold_s: 1.0,
            wait_s: 1.0,
            gap_s: 0.5,
        }
    }
}

/// The usage text.
pub const USAGE: &str = "\
sp08-probe: SP-08 mic indicator probe (#15)

USAGE:
  sp08-probe [--mode build-only|build-then-play|build-pause-then-play|cold|all]
             [--runs K] [--hold N] [--wait M] [--gap G]
  sp08-probe device-state
  sp08-probe --help

  --mode   which mode to run (default all)
  --runs   K runs per mode (default 5)
  --hold   N seconds to hold the stream after the last step (default 1)
  --wait   M seconds from build to the simulated key-down (default 1)
  --gap    seconds between runs (default 0.5)
";

/// Parse the arguments after the program name.
pub fn parse_args<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|a| a.as_ref().to_string()).collect();
    match args.first().map(String::as_str) {
        Some("device-state") if args.len() == 1 => return Ok(Command::DeviceState),
        Some("--help" | "-h" | "help") => return Ok(Command::Help),
        _ => {}
    }
    let mut p = ProbeArgs::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f.to_string(), Some(v.to_string())),
            None => (arg, None),
        };
        if !matches!(
            flag.as_str(),
            "--mode" | "--runs" | "--hold" | "--wait" | "--gap"
        ) {
            return Err(format!("unknown argument {flag:?}\n\n{USAGE}"));
        }
        let value = match inline {
            Some(v) => v,
            None => it.next().ok_or_else(|| format!("{flag} needs a value"))?,
        };
        match flag.as_str() {
            "--mode" => p.mode = parse_mode(&value)?,
            "--runs" => {
                p.runs = match value.parse::<u32>() {
                    Ok(k) if k > 0 => k,
                    _ => return Err(format!("--runs needs a whole number > 0, got {value:?}")),
                }
            }
            "--hold" => p.hold_s = parse_secs("--hold", &value)?,
            "--wait" => p.wait_s = parse_secs("--wait", &value)?,
            _ => p.gap_s = parse_secs("--gap", &value)?,
        }
    }
    Ok(Command::Probe(p))
}

/// Nearest-rank percentile (`p` in 0..=100) of `samples`. `None` if `samples` is empty.
/// The input does not have to be sorted.
pub fn percentile(samples: &[f64], p: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let rank = ((p.clamp(0.0, 100.0) / 100.0) * n as f64).ceil() as usize;
    Some(v[rank.clamp(1, n) - 1])
}

/// Summary of a latency sample set, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub min: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

/// Summarise `samples`. `None` if `samples` is empty.
pub fn summarize(samples: &[f64]) -> Option<Summary> {
    Some(Summary {
        n: samples.len(),
        min: percentile(samples, 0.0)?,
        p50: percentile(samples, 50.0)?,
        p95: percentile(samples, 95.0)?,
        max: percentile(samples, 100.0)?,
    })
}

/// Key-down to first audio callback, in ms, from nanosecond marks on one clock.
/// `first_cb_ns` 0 means no callback came. A callback before key-down is not a "live" mark.
pub fn latency_ms(keydown_ns: u64, first_cb_ns: u64) -> Option<f64> {
    if first_cb_ns == 0 || first_cb_ns < keydown_ns {
        return None;
    }
    Some((first_cb_ns - keydown_ns) as f64 / 1e6)
}

/// A CoreAudio four-char code, for example `fourcc(b"gone")`.
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

/// `kAudioDevicePropertyDeviceIsRunningSomewhere`: some process runs IO on the device.
pub const K_DEVICE_IS_RUNNING_SOMEWHERE: u32 = fourcc(b"gone");
/// `kAudioDevicePropertyDeviceIsRunning`: this process runs IO on the device.
pub const K_DEVICE_IS_RUNNING: u32 = fourcc(b"goin");
/// `kAudioHardwarePropertyDefaultInputDevice`.
pub const K_DEFAULT_INPUT_DEVICE: u32 = fourcc(b"dIn ");
/// `kAudioObjectPropertyScopeGlobal`.
pub const K_SCOPE_GLOBAL: u32 = fourcc(b"glob");

/// What the automated evidence says about the SP-08 pass rule for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// The device was idle after build and before play, and ran after play.
    IdleUntilPlay,
    /// The device ran (or callbacks came) before play: the indicator is expected on.
    LiveBeforePlay,
    /// Not enough data (no flag, a baseline that was already running, or no run after play).
    Inconclusive,
}

/// Inputs to [`evidence`] for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunFlags {
    /// `IsRunningSomewhere` before the stream was built (baseline). `None` off macOS.
    pub baseline: Option<bool>,
    /// `IsRunningSomewhere` after build, before `play()`.
    pub before_play: Option<bool>,
    /// `IsRunningSomewhere` after `play()`. `None` if the mode never plays.
    pub after_play: Option<bool>,
    /// Number of input callbacks before the simulated key-down / `play()`.
    pub callbacks_before_play: u64,
}

/// Classify one run. Callbacks before `play()` always mean [`Evidence::LiveBeforePlay`].
pub fn evidence(f: RunFlags) -> Evidence {
    if f.callbacks_before_play > 0 {
        return Evidence::LiveBeforePlay;
    }
    if f.baseline == Some(true) {
        return Evidence::Inconclusive;
    }
    match (f.before_play, f.after_play) {
        (Some(true), _) => Evidence::LiveBeforePlay,
        (Some(false), None | Some(true)) => Evidence::IdleUntilPlay,
        _ => Evidence::Inconclusive,
    }
}
