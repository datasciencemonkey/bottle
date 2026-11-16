//! sp08-probe: SP-08 (#15) mic indicator probe.
//!
//! Each run builds a cpal input stream on the default input device in one mode, prints
//! timestamped events and measures the simulated key-down to the first input callback.
//! On macOS it also reads the CoreAudio device-running flags as supporting evidence.
//! The flags are NOT the orange dot: a person must still watch the dot (docs/spikes/SP-08.md).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use sp08::{
    evidence, latency_ms, parse_args, summarize, Command, Evidence, Mode, ModeArg, ProbeArgs,
    RunFlags, USAGE,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long to wait for the first callback after play() before the run counts as "no audio".
const FIRST_CALLBACK_TIMEOUT: Duration = Duration::from_secs(3);
/// Sample interval of the device-running flags while the probe waits or holds.
const FLAG_POLL: Duration = Duration::from_millis(50);
/// Settle time after the stream is dropped, before the last flag read.
const DROP_SETTLE: Duration = Duration::from_millis(300);

#[cfg(target_os = "macos")]
mod coreaudio {
    //! Raw CoreAudio HAL reads. No permission is needed to read these properties.
    use sp08::{
        K_DEFAULT_INPUT_DEVICE, K_DEVICE_IS_RUNNING, K_DEVICE_IS_RUNNING_SOMEWHERE, K_SCOPE_GLOBAL,
    };
    use std::ffi::c_void;

    #[repr(C)]
    struct AudioObjectPropertyAddress {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const K_AUDIO_OBJECT_SYSTEM_OBJECT: u32 = 1;
    const K_ELEMENT_MAIN: u32 = 0;

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyData(
            object_id: u32,
            address: *const AudioObjectPropertyAddress,
            qualifier_size: u32,
            qualifier: *const c_void,
            data_size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    fn read_u32(object_id: u32, selector: u32) -> Option<u32> {
        let addr = AudioObjectPropertyAddress {
            selector,
            scope: K_SCOPE_GLOBAL,
            element: K_ELEMENT_MAIN,
        };
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: `addr`, `size` and `value` are valid for the call; the property is a UInt32.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object_id,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
                (&mut value as *mut u32).cast(),
            )
        };
        (status == 0).then_some(value)
    }

    /// The AudioObjectID of the default input device.
    pub fn default_input_device() -> Option<u32> {
        read_u32(K_AUDIO_OBJECT_SYSTEM_OBJECT, K_DEFAULT_INPUT_DEVICE).filter(|&id| id != 0)
    }

    /// (IsRunningSomewhere, IsRunning in this process) for the default input device.
    pub fn flags() -> (Option<bool>, Option<bool>) {
        match default_input_device() {
            Some(dev) => (
                read_u32(dev, K_DEVICE_IS_RUNNING_SOMEWHERE).map(|v| v != 0),
                read_u32(dev, K_DEVICE_IS_RUNNING).map(|v| v != 0),
            ),
            None => (None, None),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod coreaudio {
    //! TODO(#15): no automated device-running flag off macOS. On Windows the person reads the
    //! CapabilityAccessManager registry keys (docs/spikes/SP-08.md, "Windows 11").
    pub fn default_input_device() -> Option<u32> {
        None
    }
    pub fn flags() -> (Option<bool>, Option<bool>) {
        (None, None)
    }
}

/// The two CoreAudio flags of the default input device. `None` off macOS or on a read error.
#[derive(Debug, Clone, Copy, Default)]
struct Flags {
    /// `kAudioDevicePropertyDeviceIsRunningSomewhere`: any process runs IO on the device.
    somewhere: Option<bool>,
    /// `kAudioDevicePropertyDeviceIsRunning`: this process runs IO on the device.
    here: Option<bool>,
}

impl Flags {
    fn read() -> Self {
        let (somewhere, here) = coreaudio::flags();
        Flags { somewhere, here }
    }

    fn or(self, other: Flags) -> Self {
        fn or1(a: Option<bool>, b: Option<bool>) -> Option<bool> {
            match (a, b) {
                (None, x) | (x, None) => x,
                (Some(a), Some(b)) => Some(a || b),
            }
        }
        Flags {
            somewhere: or1(self.somewhere, other.somewhere),
            here: or1(self.here, other.here),
        }
    }

    fn show(self) -> String {
        format!("{}/{}", fmt_flag(self.somewhere), fmt_flag(self.here))
    }
}

fn fmt_flag(f: Option<bool>) -> &'static str {
    match f {
        Some(true) => "1",
        Some(false) => "0",
        None => "n/a",
    }
}

/// Logical OR of the flag samples over `dur`.
fn poll_running_for(dur: Duration) -> Flags {
    let end = Instant::now() + dur;
    let mut seen = Flags::default();
    loop {
        seen = seen.or(Flags::read());
        let now = Instant::now();
        if now >= end {
            return seen;
        }
        sleep(FLAG_POLL.min(end - now));
    }
}

/// State the audio callback shares with the control thread. Marks are ns since `base` + 1,
/// so 0 means "not yet".
struct Shared {
    base: Instant,
    keydown_ns: AtomicU64,
    first_any_ns: AtomicU64,
    first_after_keydown_ns: AtomicU64,
    callbacks: AtomicU64,
    callbacks_before_keydown: AtomicU64,
    nonzero: AtomicBool,
}

impl Shared {
    fn new(base: Instant) -> Self {
        Shared {
            base,
            keydown_ns: AtomicU64::new(0),
            first_any_ns: AtomicU64::new(0),
            first_after_keydown_ns: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            callbacks_before_keydown: AtomicU64::new(0),
            nonzero: AtomicBool::new(false),
        }
    }

    fn now_ns(&self) -> u64 {
        self.base.elapsed().as_nanos() as u64 + 1
    }

    fn on_callback<T: Sample>(&self, data: &[T]) {
        let now = self.now_ns();
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        let _ = self
            .first_any_ns
            .compare_exchange(0, now, Ordering::AcqRel, Ordering::Relaxed);
        let kd = self.keydown_ns.load(Ordering::Acquire);
        if kd == 0 {
            self.callbacks_before_keydown
                .fetch_add(1, Ordering::Relaxed);
        } else {
            let _ = self.first_after_keydown_ns.compare_exchange(
                0,
                now,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
        }
        if !self.nonzero.load(Ordering::Relaxed) && data.iter().any(|s| *s != T::EQUILIBRIUM) {
            self.nonzero.store(true, Ordering::Relaxed);
        }
    }

    fn key_down(&self) -> u64 {
        let now = self.now_ns();
        self.keydown_ns.store(now, Ordering::Release);
        now
    }

    fn wait_first_after_keydown(&self, timeout: Duration) -> u64 {
        let end = Instant::now() + timeout;
        loop {
            let v = self.first_after_keydown_ns.load(Ordering::Acquire);
            if v != 0 || Instant::now() >= end {
                return v;
            }
            sleep(Duration::from_micros(200));
        }
    }
}

struct Ctx {
    t0: Instant,
    mode: Mode,
    run: u32,
}

impl Ctx {
    fn ev(&self, event: &str, detail: &str) {
        let t = self.t0.elapsed().as_secs_f64() * 1e3;
        println!(
            "[t={t:10.3} ms] mode={} run={} event={event} {detail}",
            self.mode, self.run
        );
    }
}

fn build_typed<T: SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Shared>,
) -> Result<Stream, cpal::Error> {
    device.build_input_stream::<T, _, _>(
        *config,
        move |data: &[T], _info| shared.on_callback(data),
        |err| eprintln!("stream error: {err}"),
        None,
    )
}

fn build_stream(
    device: &cpal::Device,
    format: SampleFormat,
    config: &StreamConfig,
    shared: Arc<Shared>,
) -> Result<Stream, String> {
    let r = match format {
        SampleFormat::F32 => build_typed::<f32>(device, config, shared),
        SampleFormat::I16 => build_typed::<i16>(device, config, shared),
        SampleFormat::I32 => build_typed::<i32>(device, config, shared),
        SampleFormat::U16 => build_typed::<u16>(device, config, shared),
        other => return Err(format!("unsupported sample format {other:?}")),
    };
    r.map_err(|e| e.to_string())
}

/// The result of one run.
struct RunResult {
    latency_ms: Option<f64>,
    evidence: Evidence,
    evidence_here: Evidence,
    nonzero: bool,
}

fn run_once(
    ctx: &Ctx,
    args: &ProbeArgs,
    device: &cpal::Device,
    format: SampleFormat,
    config: &StreamConfig,
) -> Result<RunResult, String> {
    let shared = Arc::new(Shared::new(ctx.t0));
    let baseline = Flags::read();
    ctx.ev(
        "start",
        &format!("baseline_running(somewhere/here)={}", baseline.show()),
    );

    let wait = Duration::from_secs_f64(args.wait_s);
    let hold = Duration::from_secs_f64(args.hold_s);
    let before_play;
    let mut after_play: Option<Flags> = None;
    let mut keydown_ns = 0;
    let build_ms;
    let mut play_ms = f64::NAN;

    let do_build = |label: &str| -> Result<(Stream, f64), String> {
        let t = Instant::now();
        let s = build_stream(device, format, config, shared.clone())?;
        let ms = t.elapsed().as_secs_f64() * 1e3;
        ctx.ev(label, &format!("build_ms={ms:.3}"));
        Ok((s, ms))
    };
    let do_play = |stream: &Stream| -> Result<f64, String> {
        let t = Instant::now();
        stream.play().map_err(|e| e.to_string())?;
        let ms = t.elapsed().as_secs_f64() * 1e3;
        ctx.ev("play", &format!("play_call_ms={ms:.3}"));
        Ok(ms)
    };

    let stream = match ctx.mode {
        Mode::BuildOnly => {
            let (s, ms) = do_build("built")?;
            build_ms = ms;
            before_play = poll_running_for(hold);
            ctx.ev(
                "held",
                &format!(
                    "hold_s={} running_during_hold={} callbacks={}",
                    args.hold_s,
                    before_play.show(),
                    shared.callbacks.load(Ordering::Relaxed)
                ),
            );
            s
        }
        Mode::BuildThenPlay | Mode::BuildPauseThenPlay => {
            let (s, ms) = do_build("built")?;
            build_ms = ms;
            if ctx.mode == Mode::BuildPauseThenPlay {
                let r = s.pause().map_err(|e| e.to_string());
                ctx.ev("paused", &format!("ok={}", r.is_ok()));
                r?;
            }
            before_play = poll_running_for(wait);
            ctx.ev(
                "pre_keydown",
                &format!(
                    "running_before_play={} callbacks={}",
                    before_play.show(),
                    shared.callbacks.load(Ordering::Relaxed)
                ),
            );
            keydown_ns = shared.key_down();
            ctx.ev("keydown", "simulated");
            play_ms = do_play(&s)?;
            s
        }
        Mode::Cold => {
            before_play = baseline;
            keydown_ns = shared.key_down();
            ctx.ev("keydown", "simulated");
            let (s, ms) = do_build("built")?;
            build_ms = ms;
            play_ms = do_play(&s)?;
            s
        }
    };

    let mut lat = None;
    if keydown_ns != 0 {
        let first = shared.wait_first_after_keydown(FIRST_CALLBACK_TIMEOUT);
        lat = latency_ms(keydown_ns, first);
        match lat {
            Some(ms) => ctx.ev("first_callback", &format!("keydown_to_live_ms={ms:.3}")),
            None => ctx.ev(
                "first_callback",
                &format!("NONE within {} s", FIRST_CALLBACK_TIMEOUT.as_secs()),
            ),
        }
        after_play = Some(poll_running_for(hold.max(Duration::from_millis(100))));
        ctx.ev(
            "held",
            &format!(
                "hold_s={} running_after_play={}",
                args.hold_s,
                after_play.unwrap_or_default().show()
            ),
        );
    }

    let callbacks_total = shared.callbacks.load(Ordering::Relaxed);
    drop(stream);
    sleep(DROP_SETTLE);
    let after_drop = Flags::read();
    ctx.ev(
        "dropped",
        &format!("running_after_drop={}", after_drop.show()),
    );

    let cb_before_play = shared.callbacks_before_keydown.load(Ordering::Relaxed);
    // Two verdicts: one from the system-wide flag (what the OS indicator follows) and one
    // from the in-process flag (immune to another app that uses the mic at the same time).
    let ev = evidence(RunFlags {
        baseline: baseline.somewhere,
        before_play: before_play.somewhere,
        after_play: after_play.and_then(|f| f.somewhere),
        callbacks_before_play: cb_before_play,
    });
    let ev_here = evidence(RunFlags {
        baseline: baseline.here,
        before_play: before_play.here,
        after_play: after_play.and_then(|f| f.here),
        callbacks_before_play: cb_before_play,
    });
    let nonzero = shared.nonzero.load(Ordering::Relaxed);
    let first_any = shared.first_any_ns.load(Ordering::Relaxed);
    println!(
        "RUN mode={} run={} baseline={} before_play={} after_play={} after_drop={} \
         cb_before_play={} cb_total={} first_cb_any_ms={} build_ms={:.3} play_ms={:.3} \
         latency_ms={} nonzero_audio={} evidence_somewhere={:?} evidence_here={:?}",
        ctx.mode,
        ctx.run,
        baseline.show(),
        before_play.show(),
        after_play.map_or("n/a".to_string(), Flags::show),
        after_drop.show(),
        cb_before_play,
        callbacks_total,
        if first_any == 0 {
            "none".to_string()
        } else {
            format!("{:.3}", (first_any - 1) as f64 / 1e6)
        },
        build_ms,
        play_ms,
        lat.map_or("none".to_string(), |v| format!("{v:.3}")),
        nonzero,
        ev,
        ev_here
    );
    Ok(RunResult {
        latency_ms: lat,
        evidence: ev,
        evidence_here: ev_here,
        nonzero,
    })
}

fn print_device_state() {
    let dev = coreaudio::default_input_device();
    let (somewhere, here) = coreaudio::flags();
    println!(
        "DEVICE coreaudio_default_input_id={} is_running_somewhere={} is_running_in_this_process={}",
        dev.map_or("n/a".to_string(), |d| d.to_string()),
        fmt_flag(somewhere),
        fmt_flag(here)
    );
}

fn probe(args: ProbeArgs) -> Result<(), String> {
    let t0 = Instant::now();
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("no default input device")?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "?".into());
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.config();
    println!(
        "START unix_ms={unix_ms} os={} host={:?} device={name:?} format={format:?} \
         channels={} rate={} runs={} hold_s={} wait_s={} gap_s={}",
        std::env::consts::OS,
        host.id(),
        config.channels,
        config.sample_rate,
        args.runs,
        args.hold_s,
        args.wait_s,
        args.gap_s
    );
    print_device_state();

    let modes: Vec<Mode> = match args.mode {
        ModeArg::All => Mode::ALL.to_vec(),
        ModeArg::One(m) => vec![m],
    };
    let mut summaries = Vec::new();
    for mode in modes {
        let mut lats = Vec::new();
        let mut counts = [[0u32; 3]; 2];
        let mut nonzero_runs = 0;
        let slot = |e: Evidence| match e {
            Evidence::IdleUntilPlay => 0,
            Evidence::LiveBeforePlay => 1,
            Evidence::Inconclusive => 2,
        };
        for run in 1..=args.runs {
            let ctx = Ctx { t0, mode, run };
            let r = run_once(&ctx, &args, &device, format, &config)?;
            lats.extend(r.latency_ms);
            counts[0][slot(r.evidence)] += 1;
            counts[1][slot(r.evidence_here)] += 1;
            nonzero_runs += u32::from(r.nonzero);
            sleep(Duration::from_secs_f64(args.gap_s));
        }
        let lat = match summarize(&lats) {
            Some(s) => format!(
                "n={} min={:.3} p50={:.3} p95={:.3} max={:.3}",
                s.n, s.min, s.p50, s.p95, s.max
            ),
            None => "n=0 (no key-down in this mode)".to_string(),
        };
        summaries.push(format!(
            "SUMMARY mode={mode} runs={} latency_ms[{lat}] \
             evidence_somewhere[idle_until_play={} live_before_play={} inconclusive={}] \
             evidence_here[idle_until_play={} live_before_play={} inconclusive={}] \
             nonzero_audio_runs={nonzero_runs}",
            args.runs,
            counts[0][0],
            counts[0][1],
            counts[0][2],
            counts[1][0],
            counts[1][1],
            counts[1][2],
        ));
    }
    for s in summaries {
        println!("{s}");
    }
    print_device_state();
    Ok(())
}

fn main() {
    let cmd = match parse_args(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    match cmd {
        Command::Help => print!("{USAGE}"),
        Command::DeviceState => print_device_state(),
        Command::Probe(args) => {
            if let Err(e) = probe(args) {
                eprintln!("probe failed: {e}");
                std::process::exit(1);
            }
        }
    }
}
