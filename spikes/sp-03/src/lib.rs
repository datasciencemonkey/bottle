//! Spike SP-03 (#5): pure helpers for the talk-key probe.
//!
//! The probe binary (`src/main.rs`) talks to CoreGraphics, Carbon and IOKit. This library holds
//! the parts that run without a device or a permission: key-up pairing over a log, percentiles,
//! the PTT state machine (talk key, Secure Event Input, Carbon chord), modifier-flag transitions,
//! the ioreg text parsers, the CGEvent clock domain and the argument parser.
//! The ioreg parsers adapted from Handy live in `secure_input.rs` and `clamshell.rs`.

pub mod clamshell;
pub mod secure_input;

pub use clamshell::parse_clamshell_ioreg;
pub use secure_input::parse_secure_input_pid;

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
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
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
pub fn format_ev_line(e: &KeyEvent) -> String {
    let src = match e.source {
        Source::Tap => "tap",
        Source::Carbon => "carbon",
    };
    let kind = match e.kind {
        KeyKind::Down => "down",
        KeyKind::Up => "up",
    };
    let lat = e
        .latency_ms
        .map_or_else(|| "none".to_string(), |v| format!("{v}"));
    format!("EV src={src} kind={kind} t_ns={} lat_ms={lat}", e.t_ns)
}

/// Parse an `EV` log line. Other lines give `None`.
pub fn parse_ev_line(line: &str) -> Option<KeyEvent> {
    let rest = &line[line.find("EV ")? + 3..];
    let (mut source, mut kind, mut t_ns, mut lat) = (None, None, None, None);
    for field in rest.split_whitespace() {
        let (k, v) = field.split_once('=')?;
        match k {
            "src" => {
                source = Some(match v {
                    "tap" => Source::Tap,
                    "carbon" => Source::Carbon,
                    _ => return None,
                })
            }
            "kind" => {
                kind = Some(match v {
                    "down" => KeyKind::Down,
                    "up" => KeyKind::Up,
                    _ => return None,
                })
            }
            "t_ns" => t_ns = Some(v.parse().ok()?),
            "lat_ms" => {
                lat = Some(if v == "none" {
                    None
                } else {
                    Some(v.parse::<f64>().ok()?)
                })
            }
            _ => {}
        }
    }
    Some(KeyEvent {
        source: source?,
        kind: kind?,
        t_ns: t_ns?,
        latency_ms: lat?,
    })
}

/// Nearest-rank percentile (`p` in 0..=100). `None` if `samples` is empty.
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
pub fn summarize(samples: &[f64]) -> Option<Summary> {
    Some(Summary {
        n: samples.len(),
        min: percentile(samples, 0.0)?,
        p50: percentile(samples, 50.0)?,
        p95: percentile(samples, 95.0)?,
        max: percentile(samples, 100.0)?,
    })
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
        if self.downs == 0 {
            100.0
        } else {
            100.0 * self.pairs as f64 / self.downs as f64
        }
    }
}

/// Pair downs and ups of one `source`, in log order.
pub fn pair_events(events: &[KeyEvent], source: Source) -> PairReport {
    let (mut downs, mut ups, mut pairs, mut missed, mut orphan) = (0, 0, 0, 0, 0);
    let mut open: Option<u64> = None;
    let mut lats = Vec::new();
    let mut holds = Vec::new();
    for e in events.iter().filter(|e| e.source == source) {
        lats.extend(e.latency_ms);
        match e.kind {
            KeyKind::Down => {
                downs += 1;
                if open.replace(e.t_ns).is_some() {
                    missed += 1;
                }
            }
            KeyKind::Up => {
                ups += 1;
                match open.take() {
                    Some(t) => {
                        pairs += 1;
                        holds.push(e.t_ns.saturating_sub(t) as f64 / 1e6);
                    }
                    None => orphan += 1,
                }
            }
        }
    }
    if open.is_some() {
        missed += 1;
    }
    PairReport {
        downs,
        ups,
        pairs,
        missed_key_ups: missed,
        orphan_key_ups: orphan,
        latency: summarize(&lats),
        hold: summarize(&holds),
    }
}

/// The talk-key transition that a `kCGEventFlagsChanged` event means for Right Option.
/// `None` if the event is for another key.
pub fn right_option_transition(keycode: i64, flags: u64) -> Option<KeyKind> {
    if keycode != KVK_RIGHT_OPTION {
        return None;
    }
    Some(if flags & NX_DEVICE_RIGHT_ALT != 0 {
        KeyKind::Down
    } else {
        KeyKind::Up
    })
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
        self.active.is_some()
    }

    /// True while Secure Event Input is on.
    pub fn secure_input(&self) -> bool {
        self.sei
    }

    fn start(&mut self, by: Source, t_ns: u64) -> PttOut {
        self.active = Some((by, t_ns));
        PttOut::Started { by, t_ns }
    }

    fn end(&mut self, reason: EndReason, t_ns: u64) -> PttOut {
        match self.active.take() {
            Some((_, t0)) => PttOut::Ended {
                reason,
                t_ns,
                held_ms: t_ns.saturating_sub(t0) as f64 / 1e6,
            },
            None => PttOut::Ignored,
        }
    }

    /// A talk-key event from the tap.
    pub fn on_talk(&mut self, kind: KeyKind, t_ns: u64) -> PttOut {
        if self.sei {
            return PttOut::Ignored;
        }
        match (kind, self.active) {
            (KeyKind::Down, None) => self.start(Source::Tap, t_ns),
            (KeyKind::Up, Some((Source::Tap, _))) => self.end(EndReason::KeyUp, t_ns),
            _ => PttOut::Ignored,
        }
    }

    /// A Carbon chord event.
    pub fn on_chord(&mut self, kind: KeyKind, t_ns: u64) -> PttOut {
        match (kind, self.active) {
            (KeyKind::Down, None) if self.sei => self.start(Source::Carbon, t_ns),
            (KeyKind::Up, Some((Source::Carbon, _))) => self.end(EndReason::ChordUp, t_ns),
            _ => PttOut::Ignored,
        }
    }

    /// Secure Event Input changed (from the poller).
    pub fn on_secure_input(&mut self, on: bool, t_ns: u64) -> PttOut {
        let was = std::mem::replace(&mut self.sei, on);
        match self.active {
            Some((Source::Tap, _)) if on && !was => self.end(EndReason::SecureInput, t_ns),
            _ => PttOut::Ignored,
        }
    }
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
    ts: u64,
    mach_now: u64,
    uptime_ns: u64,
    numer: u32,
    denom: u32,
) -> TsDomain {
    let as_ticks = ts_to_ns(mach_now, TsDomain::MachTicks, numer, denom).abs_diff(ts_to_ns(
        ts,
        TsDomain::MachTicks,
        numer,
        denom,
    ));
    let as_nanos = uptime_ns.abs_diff(ts);
    if as_ticks <= as_nanos {
        TsDomain::MachTicks
    } else {
        TsDomain::Nanos
    }
}

/// Convert an event timestamp to ns in its domain.
pub fn ts_to_ns(ts: u64, domain: TsDomain, numer: u32, denom: u32) -> u64 {
    match domain {
        TsDomain::Nanos => ts,
        TsDomain::MachTicks => (ts as u128 * numer as u128 / denom.max(1) as u128) as u64,
    }
}

/// Event timestamp to "now" in ns. Events can carry either clock (a posted event and a HID
/// event do not always match `CGEventCreate`), so take the clock on which the age is not
/// negative and below 10 s, and the smaller age if both fit. `None` if neither fits.
pub fn event_latency_ns(
    ts: u64,
    mach_now: u64,
    uptime_ns: u64,
    numer: u32,
    denom: u32,
) -> Option<u64> {
    const MAX_AGE_NS: u64 = 10_000_000_000;
    let ticks = ts_to_ns(mach_now, TsDomain::MachTicks, numer, denom).checked_sub(ts_to_ns(
        ts,
        TsDomain::MachTicks,
        numer,
        denom,
    ));
    let nanos = uptime_ns.checked_sub(ts);
    [ticks, nanos]
        .into_iter()
        .flatten()
        .filter(|&age| age < MAX_AGE_NS)
        .min()
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
pub fn parse_args<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|a| a.as_ref().to_string()).collect();
    let Some((cmd, rest)) = args.split_first() else {
        return Ok(Command::Help);
    };
    match cmd.as_str() {
        "--help" | "-h" | "help" => Ok(Command::Help),
        "status" if rest.is_empty() => Ok(Command::Status),
        "analyze" if rest.len() == 1 => Ok(Command::Analyze {
            path: rest[0].clone(),
        }),
        "analyze" => Err("analyze needs one log file".into()),
        "listen" => {
            let mut a = ListenArgs::default();
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--seconds" => {
                        i += 1;
                        let v = rest.get(i).ok_or("--seconds needs a value")?;
                        a.seconds = match v.parse::<f64>() {
                            Ok(x) if x.is_finite() && x >= 0.0 => x,
                            _ => return Err(format!("--seconds needs a number >= 0, got {v:?}")),
                        };
                    }
                    "--request-access" => a.request_access = true,
                    "--no-tap" => a.tap = false,
                    "--no-carbon" => a.carbon = false,
                    "--no-sei" => a.sei = false,
                    "--no-clamshell" => a.clamshell = false,
                    other => return Err(format!("unknown listen argument {other:?}\n\n{USAGE}")),
                }
                i += 1;
            }
            Ok(Command::Listen(a))
        }
        "self-test" => {
            let mut a = SelfTestArgs::default();
            let mut it = rest.iter();
            while let Some(flag) = it.next() {
                let v = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
                let n: u64 = v
                    .parse()
                    .map_err(|_| format!("{flag} needs a whole number, got {v:?}"))?;
                match flag.as_str() {
                    "--pairs" if n > 0 && n <= 100_000 => a.pairs = n as u32,
                    "--pairs" => return Err("--pairs needs 1..=100000".into()),
                    "--hold-ms" => a.hold_ms = n,
                    "--gap-ms" => a.gap_ms = n,
                    other => return Err(format!("unknown self-test argument {other:?}")),
                }
            }
            Ok(Command::SelfTest(a))
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    }
}
