// Adapted from cjpais/Handy src-tauri/src/secure_input.rs and src-tauri/src/helpers/clamshell.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! macOS side of the SP-03 probe: raw FFI to CoreGraphics, Carbon, IOKit and CoreFoundation.
//!
//! Adapted from Handy: the `IsSecureEventInputEnabled` binding, the ioreg holder lookup
//! (`secure_input_holder`) and the ioreg clamshell cross-check (`clamshell_ioreg`).
//!
//! Privacy: the probe logs only the talk key (Right Option) and the Carbon chord. Every other
//! key event is counted by type only. Key identity of other keys is never read or logged.

use sp03::{
    event_latency_ns, format_ev_line, pair_events, parse_clamshell_ioreg, parse_secure_input_pid,
    right_option_transition, summarize, timestamp_domain, EndReason, KeyEvent, KeyKind, ListenArgs,
    PairReport, Ptt, PttOut, SelfTestArgs, Source, TsDomain, CARBON_CHORD_MODIFIERS,
    CG_FLAG_ALTERNATE, KVK_RIGHT_OPTION, KVK_SPACE, K_EVENT_CLASS_KEYBOARD, K_EVENT_HOTKEY_PRESSED,
    K_EVENT_HOTKEY_RELEASED, NX_DEVICE_RIGHT_ALT,
};
use std::ffi::{c_char, c_void, CString};
use std::io::Write;
use std::process::Command as Proc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFMachPortRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CGEventRef = *mut c_void;
type EventRef = *mut c_void;
type EventTargetRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;

type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;
type CarbonHandler = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> i32;

const K_CG_SESSION_EVENT_TAP: u32 = 1;
const K_CG_HID_EVENT_TAP: u32 = 0;
const K_CG_HEAD_INSERT: u32 = 0;
const K_CG_TAP_LISTEN_ONLY: u32 = 1;
const K_CG_EVENT_KEY_DOWN: u32 = 10;
const K_CG_EVENT_KEY_UP: u32 = 11;
const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
const K_CG_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const K_CG_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
const K_CG_EVENT_SOURCE_USER_DATA: u32 = 42;
const K_CG_EVENT_SOURCE_STATE_HID: u32 = 1;
const CLOCK_UPTIME_RAW: u32 = 8;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
/// Tag on the synthetic events of `self-test`, in `kCGEventSourceUserData`.
const SYNTHETIC_TAG: i64 = 0x5350_3033; // "SP03"
const SEI_POLL: Duration = Duration::from_millis(10);
const CLAMSHELL_POLL: Duration = Duration::from_millis(200);

#[repr(C)]
#[derive(Default)]
struct MachTimebase {
    numer: u32,
    denom: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetTimestamp(event: CGEventRef) -> u64;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventGetFlags(event: CGEventRef) -> u64;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventSetType(event: CGEventRef, kind: u32);
    fn CGEventCreate(source: *const c_void) -> CGEventRef;
    fn CGEventCreateKeyboardEvent(source: *const c_void, keycode: u16, down: bool) -> CGEventRef;
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CGEventSourceFlagsState(state: u32) -> u64;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFRunLoopDefaultMode: CFStringRef;
    fn CFMachPortCreateRunLoopSource(
        alloc: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after: u8) -> i32;
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        s: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(b: CFTypeRef) -> u8;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: CFStringRef,
        alloc: *const c_void,
        options: u32,
    ) -> CFTypeRef;
    fn IOObjectRelease(object: u32) -> i32;
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    // Carbon HIToolbox; Boolean is an unsigned char.
    fn IsSecureEventInputEnabled() -> u8;
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: CarbonHandler,
        num_types: usize,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: EventHotKeyRef) -> i32;
    fn GetEventKind(event: EventRef) -> u32;
    fn GetEventTime(event: EventRef) -> f64;
    fn GetCurrentEventTime() -> f64;
    fn RunCurrentEventLoop(timeout: f64) -> i32;
}

extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebase) -> i32;
    fn clock_gettime_nsec_np(clock: u32) -> u64;
}

// ---------------------------------------------------------------- shared probe state

struct Probe {
    t0: Instant,
    domain: TsDomain,
    numer: u32,
    denom: u32,
    events: Mutex<Vec<KeyEvent>>,
    ptt: Mutex<Ptt>,
    stop: AtomicBool,
    tap_ready: AtomicBool,
    tap_port: AtomicPtr<c_void>,
    tap_disabled: AtomicU64,
    synthetic_seen: AtomicU64,
    other_key_down: AtomicU64,
    other_key_up: AtomicU64,
    other_flags: AtomicU64,
    sei_on_at: AtomicU64,
    ptt_ends: Mutex<Vec<(EndReason, f64)>>,
    sei_to_end_ms: Mutex<Vec<f64>>,
}

static PROBE: OnceLock<Probe> = OnceLock::new();

fn probe() -> &'static Probe {
    PROBE.get_or_init(|| {
        let (domain, numer, denom) = clock_domain();
        Probe {
            t0: Instant::now(),
            domain,
            numer,
            denom,
            events: Mutex::new(Vec::new()),
            ptt: Mutex::new(Ptt::new()),
            stop: AtomicBool::new(false),
            tap_ready: AtomicBool::new(false),
            tap_port: AtomicPtr::new(std::ptr::null_mut()),
            tap_disabled: AtomicU64::new(0),
            synthetic_seen: AtomicU64::new(0),
            other_key_down: AtomicU64::new(0),
            other_key_up: AtomicU64::new(0),
            other_flags: AtomicU64::new(0),
            sei_on_at: AtomicU64::new(0),
            ptt_ends: Mutex::new(Vec::new()),
            sei_to_end_ms: Mutex::new(Vec::new()),
        }
    })
}

fn now_ns() -> u64 {
    probe().t0.elapsed().as_nanos() as u64
}

fn log(src: &str, event: &str, detail: &str) {
    let t = now_ns() as f64 / 1e6;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "[t={t:11.3} ms] src={src} event={event} {detail}");
    let _ = out.flush();
}

fn emit(e: KeyEvent, extra: &str) {
    {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{} {extra}", format_ev_line(&e));
        let _ = out.flush();
    }
    probe().events.lock().unwrap().push(e);
}

fn ptt_out(out: PttOut) {
    let p = probe();
    match out {
        PttOut::Started { by, .. } => log("ptt", "started", &format!("by={by:?}")),
        PttOut::Ended {
            reason, held_ms, ..
        } => {
            p.ptt_ends.lock().unwrap().push((reason, held_ms));
            log(
                "ptt",
                "ended",
                &format!("reason={reason:?} held_ms={held_ms:.3}"),
            );
        }
        PttOut::Ignored => {}
    }
}

// ---------------------------------------------------------------- clocks

fn timebase() -> (u32, u32) {
    let mut tb = MachTimebase::default();
    // SAFETY: valid out pointer.
    unsafe { mach_timebase_info(&mut tb) };
    (tb.numer.max(1), tb.denom.max(1))
}

/// Which clock CGEventGetTimestamp uses: compare a fresh event with both clocks.
fn clock_domain() -> (TsDomain, u32, u32) {
    let (numer, denom) = timebase();
    // SAFETY: CGEventCreate(NULL) returns a new event or NULL; we release it.
    unsafe {
        let ev = CGEventCreate(std::ptr::null());
        if ev.is_null() {
            return (TsDomain::Nanos, numer, denom);
        }
        let ts = CGEventGetTimestamp(ev);
        let mach = mach_absolute_time();
        let up = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
        CFRelease(ev);
        (timestamp_domain(ts, mach, up, numer, denom), numer, denom)
    }
}

// ---------------------------------------------------------------- (a) listen-only event tap

extern "C" fn tap_callback(
    _proxy: *mut c_void,
    etype: u32,
    event: CGEventRef,
    _info: *mut c_void,
) -> CGEventRef {
    let p = probe();
    let t = now_ns();
    match etype {
        K_CG_TAP_DISABLED_BY_TIMEOUT | K_CG_TAP_DISABLED_BY_USER_INPUT => {
            // CAP-38a watchdog: the OS disabled the tap; enable it again.
            p.tap_disabled.fetch_add(1, Ordering::Relaxed);
            let port = p.tap_port.load(Ordering::Acquire);
            if !port.is_null() {
                // SAFETY: port is the live tap created by this process.
                unsafe { CGEventTapEnable(port, true) };
            }
            let why = if etype == K_CG_TAP_DISABLED_BY_TIMEOUT {
                "timeout"
            } else {
                "user_input"
            };
            log("tap", "tap_disabled", &format!("reason={why} re_enabled=1"));
        }
        K_CG_EVENT_KEY_DOWN => {
            p.other_key_down.fetch_add(1, Ordering::Relaxed);
        }
        K_CG_EVENT_KEY_UP => {
            p.other_key_up.fetch_add(1, Ordering::Relaxed);
        }
        K_CG_EVENT_FLAGS_CHANGED => {
            // SAFETY: `event` is valid for the duration of the callback.
            let (keycode, flags, ts, tag) = unsafe {
                (
                    CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE),
                    CGEventGetFlags(event),
                    CGEventGetTimestamp(event),
                    CGEventGetIntegerValueField(event, K_CG_EVENT_SOURCE_USER_DATA),
                )
            };
            match right_option_transition(keycode, flags) {
                Some(kind) => {
                    // SAFETY: plain clock reads.
                    let (mach, up) = unsafe {
                        (
                            mach_absolute_time(),
                            clock_gettime_nsec_np(CLOCK_UPTIME_RAW),
                        )
                    };
                    let lat =
                        event_latency_ns(ts, mach, up, p.numer, p.denom).map(|ns| ns as f64 / 1e6);
                    let synthetic = tag == SYNTHETIC_TAG;
                    if synthetic {
                        p.synthetic_seen.fetch_add(1, Ordering::Relaxed);
                    }
                    emit(
                        KeyEvent {
                            source: Source::Tap,
                            kind,
                            t_ns: t,
                            latency_ms: lat,
                        },
                        &format!(
                            "key=RightOption synthetic={} ts_raw={ts} mach_now={mach} uptime_ns={up}",
                            u8::from(synthetic)
                        ),
                    );
                    let out = p.ptt.lock().unwrap().on_talk(kind, t);
                    ptt_out(out);
                }
                None => {
                    p.other_flags.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        _ => {}
    }
    event
}

/// Start the tap on its own run-loop thread (CAP-35a). Returns false if the tap cannot be made.
fn start_tap() -> Option<thread::JoinHandle<()>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let h = thread::Builder::new()
        .name("sp03-tap".into())
        .spawn(move || {
            let mask: u64 = (1 << K_CG_EVENT_KEY_DOWN)
                | (1 << K_CG_EVENT_KEY_UP)
                | (1 << K_CG_EVENT_FLAGS_CHANGED);
            // SAFETY: valid callback and arguments; NULL return is handled.
            let port = unsafe {
                CGEventTapCreate(
                    K_CG_SESSION_EVENT_TAP,
                    K_CG_HEAD_INSERT,
                    K_CG_TAP_LISTEN_ONLY,
                    mask,
                    tap_callback,
                    std::ptr::null_mut(),
                )
            };
            if port.is_null() {
                let _ = tx.send(false);
                return;
            }
            let p = probe();
            p.tap_port.store(port, Ordering::Release);
            // SAFETY: port is valid; the source is added to this thread's run loop.
            unsafe {
                let src = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
                CFRunLoopAddSource(CFRunLoopGetCurrent(), src, kCFRunLoopCommonModes);
                CGEventTapEnable(port, true);
            }
            p.tap_ready.store(true, Ordering::Release);
            let _ = tx.send(true);
            while !p.stop.load(Ordering::Acquire) {
                // SAFETY: runs this thread's run loop for at most 0.1 s.
                unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, 0) };
            }
            // SAFETY: disable before exit; the process ends soon after.
            unsafe { CGEventTapEnable(port, false) };
        })
        .ok()?;
    match rx.recv() {
        Ok(true) => Some(h),
        _ => {
            let _ = h.join();
            None
        }
    }
}

// ---------------------------------------------------------------- (b) Secure Event Input

fn sei_enabled() -> bool {
    // SAFETY: no arguments.
    unsafe { IsSecureEventInputEnabled() != 0 }
}

/// Best-effort holder of Secure Event Input (Handy pattern): PID from ioreg, name from ps.
fn secure_input_holder() -> String {
    let Ok(out) = Proc::new("ioreg").args(["-l", "-w", "0"]).output() else {
        return "unknown".into();
    };
    let Some(pid) = parse_secure_input_pid(&String::from_utf8_lossy(&out.stdout)) else {
        return "none_visible".into();
    };
    let name = Proc::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| s.rsplit('/').next().unwrap_or(&s).to_string())
        .unwrap_or_else(|| "(not running)".into());
    format!("pid={pid} name={name:?}")
}

fn right_option_physically_down() -> bool {
    // SAFETY: reads the HID system modifier state.
    unsafe { CGEventSourceFlagsState(K_CG_EVENT_SOURCE_STATE_HID) & NX_DEVICE_RIGHT_ALT != 0 }
}

fn start_sei_poller() -> thread::JoinHandle<()> {
    thread::spawn(|| {
        let p = probe();
        let mut on = sei_enabled();
        log("sei", "initial", &format!("secure_input={}", u8::from(on)));
        p.ptt.lock().unwrap().on_secure_input(on, now_ns());
        while !p.stop.load(Ordering::Acquire) {
            thread::sleep(SEI_POLL);
            let now = sei_enabled();
            if now == on {
                continue;
            }
            on = now;
            let t_det = now_ns();
            let out = p.ptt.lock().unwrap().on_secure_input(on, t_det);
            let t_end = now_ns();
            if on {
                p.sei_on_at.store(t_det, Ordering::Relaxed);
                log("sei", "on", &format!("poll_ms={}", SEI_POLL.as_millis()));
                if let PttOut::Ended { .. } = out {
                    let ms = (t_end - t_det) as f64 / 1e6;
                    p.sei_to_end_ms.lock().unwrap().push(ms);
                    log(
                        "sei",
                        "ptt_end_by_sei",
                        &format!(
                            "detect_to_end_ms={ms:.3} upper_bound_sei_to_end_ms={:.3}",
                            ms + SEI_POLL.as_secs_f64() * 1e3
                        ),
                    );
                }
                ptt_out(out);
                log("sei", "holder", &secure_input_holder());
            } else {
                // CAP-45: read the physical key state again when SEI ends.
                log(
                    "sei",
                    "off",
                    &format!(
                        "right_option_physically_down={}",
                        u8::from(right_option_physically_down())
                    ),
                );
                ptt_out(out);
            }
        }
    })
}

// ---------------------------------------------------------------- (c) Carbon chord

extern "C" fn carbon_handler(_call: EventHandlerCallRef, event: EventRef, _ud: *mut c_void) -> i32 {
    let p = probe();
    let t = now_ns();
    // SAFETY: `event` is valid during the handler.
    let (kind, lat_ms) = unsafe {
        (
            GetEventKind(event),
            (GetCurrentEventTime() - GetEventTime(event)) * 1e3,
        )
    };
    let kind = match kind {
        K_EVENT_HOTKEY_PRESSED => KeyKind::Down,
        K_EVENT_HOTKEY_RELEASED => KeyKind::Up,
        _ => return 0,
    };
    emit(
        KeyEvent {
            source: Source::Carbon,
            kind,
            t_ns: t,
            latency_ms: Some(lat_ms.max(0.0)),
        },
        "key=Ctrl+Shift+Space",
    );
    let out = p.ptt.lock().unwrap().on_chord(kind, t);
    ptt_out(out);
    0
}

/// Register Ctrl+Shift+Space on the application target. Returns the hotkey ref.
fn register_chord() -> Option<EventHotKeyRef> {
    let specs = [
        EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOTKEY_PRESSED,
        },
        EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOTKEY_RELEASED,
        },
    ];
    let mut handler: EventHandlerRef = std::ptr::null_mut();
    let mut hk: EventHotKeyRef = std::ptr::null_mut();
    // SAFETY: valid arguments; out pointers are valid.
    let (s1, s2) = unsafe {
        let target = GetApplicationEventTarget();
        let s1 = InstallEventHandler(
            target,
            carbon_handler,
            specs.len(),
            specs.as_ptr(),
            std::ptr::null_mut(),
            &mut handler,
        );
        let s2 = RegisterEventHotKey(
            KVK_SPACE,
            CARBON_CHORD_MODIFIERS,
            EventHotKeyId {
                signature: sp03::fourcc(b"S03P"),
                id: 1,
            },
            target,
            0,
            &mut hk,
        );
        (s1, s2)
    };
    log(
        "carbon",
        "register",
        &format!("chord=Ctrl+Shift+Space install_handler_status={s1} register_status={s2}"),
    );
    (s1 == 0 && s2 == 0).then_some(hk)
}

// ---------------------------------------------------------------- (d) clamshell

/// `AppleClamshellState` of IOPMrootDomain through IOKit. `None` if absent (desktop Mac).
fn clamshell_iokit() -> Option<bool> {
    let name = CString::new("IOPMrootDomain").ok()?;
    let key = CString::new("AppleClamshellState").ok()?;
    // SAFETY: IOKit/CF calls with valid C strings; every created object is released.
    unsafe {
        let svc = IOServiceGetMatchingService(0, IOServiceMatching(name.as_ptr()));
        if svc == 0 {
            return None;
        }
        let cf_key =
            CFStringCreateWithCString(std::ptr::null(), key.as_ptr(), K_CF_STRING_ENCODING_UTF8);
        let v = IORegistryEntryCreateCFProperty(svc, cf_key, std::ptr::null(), 0);
        CFRelease(cf_key);
        IOObjectRelease(svc);
        if v.is_null() {
            return None;
        }
        let r = (CFGetTypeID(v) == CFBooleanGetTypeID()).then(|| CFBooleanGetValue(v) != 0);
        CFRelease(v);
        r
    }
}

/// Cross-check through the `ioreg` tool (Handy's method).
fn clamshell_ioreg() -> Option<bool> {
    let out = Proc::new("ioreg")
        .args(["-r", "-k", "AppleClamshellState", "-d", "4"])
        .output()
        .ok()?;
    parse_clamshell_ioreg(&String::from_utf8_lossy(&out.stdout))
}

fn fmt_opt(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "closed",
        Some(false) => "open",
        None => "n/a",
    }
}

fn start_clamshell_poller() -> thread::JoinHandle<()> {
    thread::spawn(|| {
        let p = probe();
        let mut state = clamshell_iokit();
        log("lid", "initial", &format!("lid={}", fmt_opt(state)));
        while !p.stop.load(Ordering::Acquire) {
            thread::sleep(CLAMSHELL_POLL);
            let now = clamshell_iokit();
            if now != state {
                state = now;
                log(
                    "lid",
                    "changed",
                    &format!(
                        "lid={} poll_ms={}",
                        fmt_opt(now),
                        CLAMSHELL_POLL.as_millis()
                    ),
                );
            }
        }
    })
}

// ---------------------------------------------------------------- commands

fn permissions() -> (bool, bool, bool) {
    // SAFETY: preflight calls never show a prompt.
    unsafe {
        (
            CGPreflightListenEventAccess(),
            CGPreflightPostEventAccess(),
            AXIsProcessTrusted(),
        )
    }
}

pub fn status() {
    let p = probe();
    let (listen, post, ax) = permissions();
    println!(
        "STATUS input_monitoring_preflight={} post_event_preflight={} accessibility_trusted={}",
        u8::from(listen),
        u8::from(post),
        u8::from(ax)
    );
    let sei = sei_enabled();
    println!(
        "STATUS secure_input={} holder={}",
        u8::from(sei),
        if sei {
            secure_input_holder()
        } else {
            "-".into()
        }
    );
    println!(
        "STATUS lid_iokit={} lid_ioreg={} right_option_physically_down={}",
        fmt_opt(clamshell_iokit()),
        fmt_opt(clamshell_ioreg()),
        u8::from(right_option_physically_down())
    );
    println!(
        "STATUS cgevent_create_timestamp_domain={:?} mach_timebase={}/{} (per-event clock is picked by event_latency_ns)",
        p.domain, p.numer, p.denom
    );
}

fn print_pairs(name: &str, r: &PairReport) {
    let lat = r.latency.map_or("n=0".to_string(), |s| {
        format!(
            "n={} min={:.3} p50={:.3} p95={:.3} max={:.3}",
            s.n, s.min, s.p50, s.p95, s.max
        )
    });
    println!(
        "SUMMARY src={name} downs={} ups={} pairs={} missed_key_ups={} orphan_key_ups={} \
         key_up_percent={:.3} latency_ms[{lat}]",
        r.downs,
        r.ups,
        r.pairs,
        r.missed_key_ups,
        r.orphan_key_ups,
        r.key_up_percent()
    );
}

fn final_summary() {
    let p = probe();
    let events = p.events.lock().unwrap().clone();
    print_pairs("tap", &pair_events(&events, Source::Tap));
    print_pairs("carbon", &pair_events(&events, Source::Carbon));
    let ends = p.ptt_ends.lock().unwrap().clone();
    let count = |r: EndReason| ends.iter().filter(|(x, _)| *x == r).count();
    let sei = p.sei_to_end_ms.lock().unwrap().clone();
    println!(
        "SUMMARY ptt_ended key_up={} secure_input={} chord_up={} sei_detect_to_end_ms[{}] \
         tap_disabled={} other_key_down={} other_key_up={} other_flags_changed={}",
        count(EndReason::KeyUp),
        count(EndReason::SecureInput),
        count(EndReason::ChordUp),
        summarize(&sei).map_or("n=0".into(), |s| format!(
            "n={} p50={:.3} p95={:.3} max={:.3}",
            s.n, s.p50, s.p95, s.max
        )),
        p.tap_disabled.load(Ordering::Relaxed),
        p.other_key_down.load(Ordering::Relaxed),
        p.other_key_up.load(Ordering::Relaxed),
        p.other_flags.load(Ordering::Relaxed),
    );
}

pub fn listen(a: ListenArgs) -> i32 {
    let p = probe();
    status();
    if a.request_access {
        // SAFETY: this can show the Input Monitoring prompt; that is the point (CAP-36).
        let granted = unsafe { CGRequestListenEventAccess() };
        log(
            "tcc",
            "request_listen_access",
            &format!("returned={}", u8::from(granted)),
        );
    }
    let mut threads = Vec::new();
    let (listen_ok, _, _) = permissions();
    if a.tap && !listen_ok && !a.request_access {
        // Creating a tap without Input Monitoring can show the OS prompt. Do that only on request.
        log(
            "tap",
            "skipped",
            "Input Monitoring is not granted (preflight 0); rerun with --request-access",
        );
    } else if a.tap {
        match start_tap() {
            Some(h) => {
                log(
                    "tap",
                    "created",
                    "type=listen_only location=session talk_key=RightOption(kVK 61)",
                );
                threads.push(h);
            }
            None => log(
                "tap",
                "create_failed",
                "CGEventTapCreate returned NULL: Input Monitoring is missing for this terminal",
            ),
        }
    }
    if a.sei {
        threads.push(start_sei_poller());
    }
    if a.clamshell {
        threads.push(start_clamshell_poller());
    }
    let hk = if a.carbon { register_chord() } else { None };
    log(
        "probe",
        "listening",
        &format!(
            "seconds={} (0 = until Ctrl+C; the summary prints only on a timed run)",
            a.seconds
        ),
    );
    let deadline = (a.seconds > 0.0).then(|| Instant::now() + Duration::from_secs_f64(a.seconds));
    while deadline.is_none_or(|d| Instant::now() < d) {
        // SAFETY: pumps Carbon events on the main thread for at most 0.1 s.
        unsafe { RunCurrentEventLoop(0.1) };
    }
    p.stop.store(true, Ordering::Release);
    if let Some(h) = hk {
        // SAFETY: h came from RegisterEventHotKey.
        unsafe { UnregisterEventHotKey(h) };
    }
    for t in threads {
        let _ = t.join();
    }
    final_summary();
    0
}

fn post_right_option(down: bool) {
    // SAFETY: creates, posts and releases one event.
    unsafe {
        let ev = CGEventCreateKeyboardEvent(std::ptr::null(), KVK_RIGHT_OPTION as u16, down);
        if ev.is_null() {
            return;
        }
        CGEventSetType(ev, K_CG_EVENT_FLAGS_CHANGED);
        CGEventSetFlags(
            ev,
            if down {
                NX_DEVICE_RIGHT_ALT | CG_FLAG_ALTERNATE | 0x100
            } else {
                0x100
            },
        );
        CGEventSetIntegerValueField(ev, K_CG_EVENT_SOURCE_USER_DATA, SYNTHETIC_TAG);
        CGEventPost(K_CG_HID_EVENT_TAP, ev);
        CFRelease(ev);
    }
}

pub fn self_test(a: SelfTestArgs) -> i32 {
    let p = probe();
    let (listen, post, ax) = permissions();
    println!(
        "SELFTEST input_monitoring_preflight={} post_event_preflight={} accessibility_trusted={}",
        u8::from(listen),
        u8::from(post),
        u8::from(ax)
    );
    if !listen || !post {
        let mut missing = Vec::new();
        if !listen {
            missing.push("Input Monitoring (listen)");
        }
        if !post {
            missing.push("Accessibility (post events)");
        }
        println!(
            "SELFTEST BLOCKED missing={:?}: no event posted. Grant is for a person to decide; \
             the probe does not request it.",
            missing.join(", ")
        );
        return 3;
    }
    let Some(tap) = start_tap() else {
        println!("SELFTEST BLOCKED tap_create_failed");
        return 3;
    };
    thread::sleep(Duration::from_millis(100));
    for _ in 0..a.pairs {
        post_right_option(true);
        thread::sleep(Duration::from_millis(a.hold_ms));
        post_right_option(false);
        thread::sleep(Duration::from_millis(a.gap_ms));
    }
    thread::sleep(Duration::from_millis(300));
    p.stop.store(true, Ordering::Release);
    let _ = tap.join();
    let events = p.events.lock().unwrap().clone();
    let r = pair_events(&events, Source::Tap);
    println!(
        "SELFTEST posted_pairs={} synthetic_events_seen={} of {}",
        a.pairs,
        p.synthetic_seen.load(Ordering::Relaxed),
        2 * a.pairs
    );
    print_pairs("tap", &r);
    0
}
