// Adapted from cjpais/Handy src-tauri/src/secure_input.rs and src-tauri/src/helpers/clamshell.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! Spike SP-03 (#5): pure helpers for the talk-key probe.
//!
//! The probe binary (`src/main.rs`) talks to CoreGraphics, Carbon and IOKit. This library holds
//! the parts that run without a device or a permission: key-up pairing over a log, percentiles,
//! the PTT state machine (talk key, Secure Event Input, Carbon chord), modifier-flag transitions,
//! the ioreg text parsers, the CGEvent clock domain and the argument parser.
//! The adapted Handy parts are `parse_secure_input_pid` and `parse_clamshell_ioreg`.

/// macOS virtual key code of Right Option (`kVK_RightOption`). D66 default talk key.
pub const KVK_RIGHT_OPTION: i64 = 0x3D;
/// macOS virtual key code of Space (`kVK_Space`). The Carbon chord key (CAP-39b).
pub const KVK_SPACE: u32 = 0x31;
/// `NX_DEVICERALTKEYMASK`: the device-dependent Right Option bit in `CGEventFlags`.
pub const NX_DEVICE_RIGHT_ALT: u64 = 0x0000_0040;
/// `kCGEventFlagMaskAlternate`: the device-independent Option bit.
pub const CG_FLAG_ALTERNATE: u64 = 0x0008_0000;
/// Carbon `controlKey | shiftKey`: the modifiers of the Ctrl+Shift+Space chord.
pub const CARBON_CHORD_MODIFIERS: u32 = 0x1000 | 0x0200;
/// Carbon `kEventClassKeyboard` ('keyb').
pub const K_EVENT_CLASS_KEYBOARD: u32 = fourcc(b"keyb");
/// Carbon `kEventHotKeyPressed`.
pub const K_EVENT_HOTKEY_PRESSED: u32 = 5;
/// Carbon `kEventHotKeyReleased`.
pub const K_EVENT_HOTKEY_RELEASED: u32 = 6;

/// A CoreAudio/Carbon four-char code, for example `fourcc(b"keyb")`.
pub const fn fourcc(_code: &[u8; 4]) -> u32 {
    // TODO(#5) green: a const fn cannot panic at compile time, so the red stub returns 0.
    0
}

/// Key-down or key-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Down,
    Up,
}

/// Where a key event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The listen-only CGEventTap (talk key).
    Tap,
    /// The Carbon `RegisterEventHotKey` chord.
    Carbon,
}

/// One logged key event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyEvent {
    pub source: Source,
    pub kind: KeyKind,
    /// Callback time, ns on the probe's monotonic clock.
    pub t_ns: u64,
    /// Event timestamp to callback, ms. `None` if not known.
    pub latency_ms: Option<f64>,
}

/// Format one event as an `EV` log line (the format [`parse_ev_line`] reads).
pub fn format_ev_line(_e: &KeyEvent) -> String {
    todo!("TODO(#5) green")
}

/// Parse an `EV` log line. Other lines give `None`.
pub fn parse_ev_line(_line: &str) -> Option<KeyEvent> {
    todo!("TODO(#5) green")
}

/// Nearest-rank percentile (`p` in 0..=100). `None` if `samples` is empty.
pub fn percentile(_samples: &[f64], _p: f64) -> Option<f64> {
    todo!("TODO(#5) green")
}

/// Summary of a sample set (ms).
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
    todo!("TODO(#5) green")
}

/// The key-up pairing result over one source.
#[derive(Debug, Clone, PartialEq)]
pub struct PairReport {
    pub downs: usize,
    pub ups: usize,
    /// Down followed by its up.
    pub pairs: usize,
    /// A down with no up before the next down, or at the end of the log (NFR-08a failure).
    pub missed_key_ups: usize,
    /// An up with no down before it.
    pub orphan_key_ups: usize,
    /// Event-to-callback latency over every event that has one (NFR-08b).
    pub latency: Option<Summary>,
    /// Down-to-up hold time of each pair.
    pub hold: Option<Summary>,
}

impl PairReport {
    /// Percent of key-downs whose key-up was seen. 100.0 when there is no key-down.
    pub fn key_up_percent(&self) -> f64 {
        todo!("TODO(#5) green")
    }
}

/// Pair downs and ups of one `source`, in log order.
pub fn pair_events(_events: &[KeyEvent], _source: Source) -> PairReport {
    todo!("TODO(#5) green")
}

/// The talk-key transition that a `kCGEventFlagsChanged` event means for Right Option.
/// `None` if the event is for another key.
pub fn right_option_transition(_keycode: i64, _flags: u64) -> Option<KeyKind> {
    todo!("TODO(#5) green")
}

/// Why a PTT ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    KeyUp,
    SecureInput,
    ChordUp,
}

/// What the PTT state machine did with an input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PttOut {
    Started {
        by: Source,
        t_ns: u64,
    },
    Ended {
        reason: EndReason,
        t_ns: u64,
        held_ms: f64,
    },
    Ignored,
}

/// PTT state machine of the probe (CAP-39a, CAP-39b). While Secure Event Input is on, the
/// talk key is ignored and the Carbon chord drives PTT.
#[derive(Debug, Default, Clone)]
pub struct Ptt {
    active: Option<(Source, u64)>,
    sei: bool,
}

impl Ptt {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a PTT is active.
    pub fn is_active(&self) -> bool {
        todo!("TODO(#5) green")
    }

    /// True while Secure Event Input is on.
    pub fn secure_input(&self) -> bool {
        todo!("TODO(#5) green")
    }

    /// A talk-key event from the tap.
    pub fn on_talk(&mut self, _kind: KeyKind, _t_ns: u64) -> PttOut {
        todo!("TODO(#5) green")
    }

    /// A Carbon chord event.
    pub fn on_chord(&mut self, _kind: KeyKind, _t_ns: u64) -> PttOut {
        todo!("TODO(#5) green")
    }

    /// Secure Event Input changed (from the poller).
    pub fn on_secure_input(&mut self, _on: bool, _t_ns: u64) -> PttOut {
        todo!("TODO(#5) green")
    }
}

/// The holder PID of Secure Event Input from `ioreg -l -w 0` text (`kCGSSessionSecureInputPID`).
/// Best effort: Apple documents no reliable API, and the PID can be missing or wrong.
pub fn parse_secure_input_pid(_ioreg: &str) -> Option<i32> {
    todo!("TODO(#5) green")
}

/// `AppleClamshellState` from `ioreg -r -k AppleClamshellState -d 4` text. `None` if absent
/// (for example a desktop Mac).
pub fn parse_clamshell_ioreg(_ioreg: &str) -> Option<bool> {
    todo!("TODO(#5) green")
}

/// The clock of `CGEventGetTimestamp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsDomain {
    /// Nanoseconds since boot (`CLOCK_UPTIME_RAW`).
    Nanos,
    /// `mach_absolute_time` ticks (convert with the timebase).
    MachTicks,
}

/// Decide which clock a fresh event timestamp `ts` uses: the one it is nearest to.
/// `mach_now` is `mach_absolute_time()`, `uptime_ns` is `CLOCK_UPTIME_RAW` in ns, both read
/// next to `ts`. `numer`/`denom` is the mach timebase.
pub fn timestamp_domain(
    _ts: u64,
    _mach_now: u64,
    _uptime_ns: u64,
    _numer: u32,
    _denom: u32,
) -> TsDomain {
    todo!("TODO(#5) green")
}

/// Convert an event timestamp to ns in its domain.
pub fn ts_to_ns(_ts: u64, _domain: TsDomain, _numer: u32, _denom: u32) -> u64 {
    todo!("TODO(#5) green")
}

/// The probe command.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Print permissions, SEI, clamshell and the clock domain. Never prompts.
    Status,
    /// Run the watchers.
    Listen(ListenArgs),
    /// Post synthetic talk-key pairs and measure them.
    SelfTest(SelfTestArgs),
    /// Pair the `EV` lines of a log file.
    Analyze {
        path: String,
    },
    Help,
}

/// Arguments of `listen`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListenArgs {
    /// Stop after this many seconds. 0 = until Ctrl+C.
    pub seconds: f64,
    /// Call `CGRequestListenEventAccess` (this can show the Input Monitoring prompt).
    pub request_access: bool,
    pub tap: bool,
    pub carbon: bool,
    pub sei: bool,
    pub clamshell: bool,
}

impl Default for ListenArgs {
    fn default() -> Self {
        ListenArgs {
            seconds: 0.0,
            request_access: false,
            tap: true,
            carbon: true,
            sei: true,
            clamshell: true,
        }
    }
}

/// Arguments of `self-test`.
#[derive(Debug, Clone, PartialEq)]
pub struct SelfTestArgs {
    pub pairs: u32,
    pub hold_ms: u64,
    pub gap_ms: u64,
}

impl Default for SelfTestArgs {
    fn default() -> Self {
        SelfTestArgs {
            pairs: 20,
            hold_ms: 30,
            gap_ms: 30,
        }
    }
}

/// The usage text.
pub const USAGE: &str = "\
sp03-probe: SP-03 talk-key probe (#5)

USAGE:
  sp03-probe status
  sp03-probe listen [--seconds S] [--request-access] [--no-tap] [--no-carbon] [--no-sei] [--no-clamshell]
  sp03-probe self-test [--pairs N] [--hold-ms H] [--gap-ms G]
  sp03-probe analyze <log-file>
  sp03-probe --help

  status      permissions (no prompt), Secure Event Input, clamshell, CGEvent clock
  listen      talk key Right Option (tap), chord Ctrl+Shift+Space (Carbon), SEI and lid watchers
  self-test   post N synthetic Right Option down/up pairs (needs post and listen access)
  analyze     key-up pairing and latency p50/p95 over the EV lines of a listen log
";

/// Parse the arguments after the program name.
pub fn parse_args<I, S>(_args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    todo!("TODO(#5) green")
}
