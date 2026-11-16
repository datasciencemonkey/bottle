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
        todo!("TODO(#15) green")
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
pub fn parse_mode(_s: &str) -> Result<ModeArg, String> {
    todo!("TODO(#15) green")
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
pub fn parse_args<I, S>(_args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    todo!("TODO(#15) green")
}

/// Nearest-rank percentile (`p` in 0..=100) of `samples`. `None` if `samples` is empty.
/// The input does not have to be sorted.
pub fn percentile(_samples: &[f64], _p: f64) -> Option<f64> {
    todo!("TODO(#15) green")
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
pub fn summarize(_samples: &[f64]) -> Option<Summary> {
    todo!("TODO(#15) green")
}

/// Key-down to first audio callback, in ms, from nanosecond marks on one clock.
/// `first_cb_ns` 0 means no callback came. A callback before key-down is not a "live" mark.
pub fn latency_ms(_keydown_ns: u64, _first_cb_ns: u64) -> Option<f64> {
    todo!("TODO(#15) green")
}

/// A CoreAudio four-char code, for example `fourcc(b"gone")`.
pub const fn fourcc(_code: &[u8; 4]) -> u32 {
    // TODO(#15) green: a const fn cannot panic at compile time, so the red stub returns 0.
    0
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
pub fn evidence(_f: RunFlags) -> Evidence {
    todo!("TODO(#15) green")
}
