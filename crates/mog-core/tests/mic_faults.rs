//! #22: mic permission (CAP-17a), the 2 s zero-buffer stop (CAP-17b), the reopen on a default
//! device change with a gap marker (CAP-18) and the built-in mic preference (CAP-22).
//!
//! Every test uses a fake [`MicHost`]; no test opens the user's mic. The device checks (the
//! TCC prompt, AirPods in A2DP, a real default-device switch) are in the `needs:human` issue.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mog_core::audio_device::{
    choose_input, DataFn, DevicePreference, FaultFn, FrameSink, InputChoice, InputKind,
    Interleaved, MicConfig, MicDevice, MicFault, MicFrame, MicHost, SinkEvent, StreamFault,
    ZeroWatch,
};
use mog_core::capture::{
    AudioDevice, AudioSource, CaptureCore, CaptureRequest, CaptureService, DeviceError, StopReason,
};
use mog_core::clock::SimClock;
use mog_core::testkit::capture::{FakeAudioDevice, RecordingCaptureEvents};
use mog_core::types::{CaptureMode, MicHealth};

// ---------------------------------------------------------------------------------------------
// The fake host. The test thread drives each stream's data and fault callbacks.

#[derive(Default)]
struct HostState {
    /// What the audio thread asked of the host, in order.
    log: Vec<String>,
    /// The callbacks of each built stream, by build index.
    callbacks: Vec<(Option<DataFn>, Option<FaultFn>)>,
    /// Whether each built stream is still alive (not dropped).
    alive: Vec<bool>,
    /// The build indexes that must fail.
    fail_builds: Vec<usize>,
    /// The opaque ID of the default input; `None`: the host cannot tell.
    default_id: Option<String>,
}

type Shared = Arc<Mutex<HostState>>;

struct FakeStream {
    idx: usize,
    shared: Shared,
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.shared.lock().unwrap().alive[self.idx] = false;
    }
}

struct FakeHost(Shared);

impl MicHost for FakeHost {
    type Stream = FakeStream;

    fn build(&mut self, on_data: DataFn, on_fault: FaultFn) -> Result<FakeStream, DeviceError> {
        let mut s = self.0.lock().unwrap();
        let idx = s.callbacks.len();
        s.log.push(format!("build {idx}"));
        s.callbacks.push((Some(on_data), Some(on_fault)));
        s.alive.push(false);
        if s.fail_builds.contains(&idx) {
            s.alive[idx] = false;
            return Err(DeviceError { code: "no_device" });
        }
        s.alive[idx] = true;
        Ok(FakeStream {
            idx,
            shared: Arc::clone(&self.0),
        })
    }

    fn play(&mut self, stream: &FakeStream) -> Result<(), DeviceError> {
        self.0
            .lock()
            .unwrap()
            .log
            .push(format!("play {}", stream.idx));
        Ok(())
    }

    fn default_input_id(&mut self) -> Option<String> {
        self.0.lock().unwrap().default_id.clone()
    }

    fn set_preference(&mut self, preference: DevicePreference) {
        self.0
            .lock()
            .unwrap()
            .log
            .push(format!("prefer_built_in {}", preference.prefer_built_in));
    }
}

/// Run the data callback of stream `idx` once with `samples` (mono, 16 kHz).
fn feed(shared: &Shared, idx: usize, samples: &[f32]) {
    let mut cb = shared.lock().unwrap().callbacks[idx]
        .0
        .take()
        .expect("data fn");
    cb(Interleaved {
        samples,
        channels: 1,
        sample_rate: 16_000,
    });
    shared.lock().unwrap().callbacks[idx].0 = Some(cb);
}

/// Run the fault callback of stream `idx` once.
fn fault(shared: &Shared, idx: usize, f: StreamFault) {
    let mut cb = shared.lock().unwrap().callbacks[idx]
        .1
        .take()
        .expect("fault fn");
    cb(f);
    shared.lock().unwrap().callbacks[idx].1 = Some(cb);
}

#[derive(Debug, Clone, PartialEq)]
enum Got {
    Frame(Vec<f32>),
    Event(SinkEvent),
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<Got>>>);

impl FrameSink for Sink {
    fn frame(&mut self, frame: MicFrame<'_>) {
        self.0
            .lock()
            .unwrap()
            .push(Got::Frame(frame.samples.to_vec()));
    }
    fn event(&mut self, event: SinkEvent) {
        self.0.lock().unwrap().push(Got::Event(event));
    }
}

struct Rig {
    device: MicDevice,
    host: Shared,
    sink: Sink,
    faults: Arc<Mutex<Vec<(AudioSource, MicFault)>>>,
}

fn rig_with(zero_window: Duration, fail_builds: Vec<usize>) -> Rig {
    rig_full(
        MicConfig {
            zero_window,
            ..MicConfig::default()
        },
        fail_builds,
        None,
    )
}

fn rig_full(config: MicConfig, fail_builds: Vec<usize>, default_id: Option<&str>) -> Rig {
    let host: Shared = Arc::new(Mutex::new(HostState {
        fail_builds,
        default_id: default_id.map(str::to_string),
        ..HostState::default()
    }));
    let sink = Sink::default();
    let h = Arc::clone(&host);
    let (device, link) =
        MicDevice::spawn_with(move || FakeHost(h), Box::new(sink.clone()), config).expect("spawn");
    let faults = Arc::new(Mutex::new(Vec::new()));
    let f = Arc::clone(&faults);
    link.connect_fault_fn(move |source, fault| f.lock().unwrap().push((source, fault)));
    Rig {
        device,
        host,
        sink,
        faults,
    }
}

/// A rig whose zero watch never fires during a test.
fn rig() -> Rig {
    rig_with(Duration::from_secs(60), Vec::new())
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn log(r: &Rig) -> Vec<String> {
    r.host.lock().unwrap().log.clone()
}

fn alive(r: &Rig, idx: usize) -> bool {
    r.host
        .lock()
        .unwrap()
        .alive
        .get(idx)
        .copied()
        .unwrap_or(false)
}

fn got(r: &Rig) -> Vec<Got> {
    r.sink.0.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------------------------
// CAP-17a: no permission prompt at launch.

#[test]
fn cap_17a_launch_asks_the_os_for_nothing_before_a_user_open() {
    let r = rig();
    r.device.set_prefer_built_in(true);
    thread::sleep(Duration::from_millis(50));
    // No stream build (on macOS the first build shows the TCC prompt) before `open`.
    assert!(log(&r).is_empty(), "host calls at launch: {:?}", log(&r));
    // The user action (capture_start) is the first open; only then does the host build.
    r.device.open(AudioSource::Mic).unwrap();
    assert_eq!(log(&r), ["prefer_built_in true", "build 0", "play 0"]);
}

#[test]
fn cap_17a_a_denied_build_is_permission_denied_and_leaves_no_stream() {
    struct Denied;
    struct NoStream;
    impl MicHost for Denied {
        type Stream = NoStream;
        fn build(&mut self, _: DataFn, _: FaultFn) -> Result<NoStream, DeviceError> {
            Err(DeviceError {
                code: "permission_denied",
            })
        }
        fn play(&mut self, _: &NoStream) -> Result<(), DeviceError> {
            Ok(())
        }
    }
    let (device, _link) = MicDevice::spawn(|| Denied, Box::new(Sink::default())).unwrap();
    let err = device.open(AudioSource::Mic).unwrap_err();
    assert_eq!(err.code, "permission_denied");
}

// ---------------------------------------------------------------------------------------------
// CAP-17b: 2 s of zero buffers stops capture, discards the audio and reports a fix-it fault.

#[test]
fn cap_17b_zero_watch_fires_after_the_window_of_silence() {
    let mut w = ZeroWatch::new(2_000, 0);
    w.block(true, 500);
    w.block(true, 1_999);
    assert!(!w.expired(1_999));
    assert!(w.expired(2_000));
}

#[test]
fn cap_17b_zero_watch_resets_on_sound() {
    let mut w = ZeroWatch::new(2_000, 0);
    w.block(true, 1_000);
    w.block(false, 1_500);
    assert!(!w.expired(3_000));
    assert!(w.expired(3_500));
}

#[test]
fn cap_17b_no_blocks_at_all_count_as_zero_buffers() {
    let w = ZeroWatch::new(2_000, 100);
    assert!(!w.expired(2_099));
    assert!(w.expired(2_100));
}

#[test]
fn cap_17b_the_default_zero_window_is_2_s() {
    assert_eq!(MicConfig::default().zero_window, Duration::from_secs(2));
}

#[test]
fn cap_18_the_default_poll_is_200_ms_and_the_lost_grace_10_s() {
    assert_eq!(MicConfig::default().device_poll, Duration::from_millis(200));
    assert_eq!(MicConfig::default().lost_grace, Duration::from_secs(10));
}

fn polled(fail_builds: Vec<usize>, lost_grace: Duration) -> Rig {
    rig_full(
        MicConfig {
            zero_window: Duration::from_secs(60),
            device_poll: Duration::from_millis(200),
            lost_grace,
        },
        fail_builds,
        Some("device-a"),
    )
}

fn set_default(r: &Rig, id: Option<&str>) {
    r.host.lock().unwrap().default_id = id.map(str::to_string);
}

#[test]
fn cap_18_a_new_default_input_reopens_the_stream_within_500_ms_with_a_gap_marker() {
    let r = polled(Vec::new(), Duration::from_secs(10));
    r.device.open(AudioSource::Mic).unwrap();
    feed(&r.host, 0, &[0.25; 4]);
    let t0 = Instant::now();
    // The OS gives no stream error for a new default input: the poll finds it.
    set_default(&r, Some("device-b"));
    wait_until("the reopen", || alive(&r, 1));
    let took = t0.elapsed();
    assert!(took <= Duration::from_millis(500), "reopen took {took:?}");
    assert!(!alive(&r, 0));
    feed(&r.host, 1, &[0.5; 4]);
    wait_until("the new frame", || got(&r).len() == 3);
    let g = got(&r);
    assert_eq!(g[0], Got::Frame(vec![0.25; 4]));
    assert!(matches!(g[1], Got::Event(SinkEvent::Gap { .. })), "{g:?}");
    assert_eq!(g[2], Got::Frame(vec![0.5; 4]));
    assert!(r.faults.lock().unwrap().is_empty());
}

#[test]
fn cap_18_an_unchanged_default_keeps_the_stream() {
    let r = polled(Vec::new(), Duration::from_secs(10));
    r.device.open(AudioSource::Mic).unwrap();
    feed(&r.host, 0, &[0.1; 4]);
    thread::sleep(Duration::from_millis(500));
    assert!(alive(&r, 0));
    assert_eq!(log(&r), ["prefer_built_in false", "build 0", "play 0"]);
}

#[test]
fn cap_18_a_lost_device_waits_for_a_replacement_and_reopens_under_the_same_lease() {
    // Build 1 fails (no device yet); build 2, 250 ms later, finds the replacement.
    let r = polled(vec![1], Duration::from_secs(10));
    r.device.open(AudioSource::Mic).unwrap();
    fault(&r.host, 0, StreamFault::Fatal);
    wait_until("the replacement", || alive(&r, 2));
    // The host marks a stream alive in `build`; the gap marker comes after `play`, on the
    // audio thread. Wait for the marker, not only for the build (Windows CI, PR #554).
    wait_until("the gap marker", || {
        got(&r)
            .iter()
            .any(|g| matches!(g, Got::Event(SinkEvent::Gap { .. })))
    });
    assert!(
        r.faults.lock().unwrap().is_empty(),
        "no loss inside the grace"
    );
    r.device.close(AudioSource::Mic);
    assert!(!alive(&r, 2));
}

#[test]
fn cap_18_no_replacement_within_the_grace_reports_the_device_lost() {
    let r = polled((1..200).collect(), Duration::from_millis(300));
    r.device.open(AudioSource::Mic).unwrap();
    fault(&r.host, 0, StreamFault::Fatal);
    wait_until("the lost fault", || !r.faults.lock().unwrap().is_empty());
    assert_eq!(
        *r.faults.lock().unwrap(),
        [(AudioSource::Mic, MicFault::Lost)]
    );
}

#[test]
fn cap_18_a_close_during_the_wait_ends_it_without_a_fault() {
    let r = polled((1..200).collect(), Duration::from_secs(10));
    r.device.open(AudioSource::Mic).unwrap();
    fault(&r.host, 0, StreamFault::Fatal);
    wait_until("a failed retry", || log(&r).iter().any(|l| l == "build 1"));
    r.device.close(AudioSource::Mic);
    thread::sleep(Duration::from_millis(400));
    assert!(r.faults.lock().unwrap().is_empty());
    let builds = log(&r).iter().filter(|l| l.starts_with("build")).count();
    thread::sleep(Duration::from_millis(400));
    let later = log(&r).iter().filter(|l| l.starts_with("build")).count();
    assert_eq!(builds, later, "no retry after the close");
}

#[test]
fn cap_22_a_headset_that_becomes_the_default_is_checked_against_the_preference() {
    let r = polled(Vec::new(), Duration::from_secs(10));
    r.device.set_prefer_built_in(true);
    r.device.open(AudioSource::Mic).unwrap();
    // AirPods connect and become the default input: the stream builds again with the
    // preference, so the host picks the built-in mic.
    set_default(&r, Some("headset"));
    wait_until("the reopen", || alive(&r, 1));
    assert_eq!(log(&r)[3..], ["prefer_built_in true", "build 1", "play 1"]);
}

#[test]
fn cap_17b_zero_buffers_stop_the_stream_discard_the_audio_and_report_the_fault() {
    let r = rig_with(Duration::from_millis(100), Vec::new());
    r.device.open(AudioSource::Mic).unwrap();
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end && alive(&r, 0) {
        feed(&r.host, 0, &[0.0; 160]);
        thread::sleep(Duration::from_millis(5));
    }
    wait_until("zero-buffer fault", || !r.faults.lock().unwrap().is_empty());
    assert_eq!(
        *r.faults.lock().unwrap(),
        [(AudioSource::Mic, MicFault::ZeroBuffers)]
    );
    assert!(!alive(&r, 0), "the stream must stop");
    // No transcript: the sink is told to drop what it holds.
    assert!(got(&r).contains(&Got::Event(SinkEvent::Discard)));
    // The next user open builds a fresh stream.
    r.device.open(AudioSource::Mic).unwrap();
    assert!(alive(&r, 1));
}

#[test]
fn cap_17b_a_stream_that_gives_no_buffers_at_all_is_stopped() {
    let r = rig_with(Duration::from_millis(100), Vec::new());
    r.device.open(AudioSource::Mic).unwrap();
    wait_until("zero-buffer fault", || !r.faults.lock().unwrap().is_empty());
    assert_eq!(
        *r.faults.lock().unwrap(),
        [(AudioSource::Mic, MicFault::ZeroBuffers)]
    );
    assert!(!alive(&r, 0));
}

#[test]
fn cap_17b_sound_keeps_the_mic_open() {
    let r = rig_with(Duration::from_millis(100), Vec::new());
    r.device.open(AudioSource::Mic).unwrap();
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        feed(&r.host, 0, &[0.01; 160]);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(r.faults.lock().unwrap().is_empty());
    assert!(alive(&r, 0));
    r.device.close(AudioSource::Mic);
    assert!(!alive(&r, 0));
}

#[test]
fn cap_17b_capture_core_stops_every_mic_lease_with_zero_buffers() {
    let events = Arc::new(RecordingCaptureEvents::default());
    let core = CaptureCore::new(
        Box::new(FakeAudioDevice::new()),
        Arc::new(SimClock::new()),
        events.clone(),
    );
    core.start(CaptureRequest {
        mog_id: "mog-1".to_string(),
        mode: CaptureMode::Ptt,
        others_track: false,
    })
    .unwrap();
    core.mic_silent();
    let stop = events.last_stop().expect("a stop");
    assert_eq!(stop.reason, StopReason::ZeroBuffers);
    assert_eq!(stop.reason.code(), "zero_buffers");
    // No transcript: the segment pipeline drops the audio of this lease.
    assert!(stop.reason.discards_audio());
    assert!(!core.status().recording);
}

// ---------------------------------------------------------------------------------------------
// CAP-18: a default device change reopens in 500 ms or less with a gap marker.

#[test]
fn cap_18_an_invalidated_stream_reopens_on_the_new_default_with_a_gap_marker() {
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    feed(&r.host, 0, &[0.25; 4]);
    let t0 = Instant::now();
    fault(&r.host, 0, StreamFault::Invalidated);
    wait_until("the reopen", || alive(&r, 1));
    let took = t0.elapsed();
    assert!(took <= Duration::from_millis(500), "reopen took {took:?}");
    assert!(!alive(&r, 0), "the old stream must be dropped");
    assert_eq!(log(&r)[3..], ["prefer_built_in false", "build 1", "play 1"]);
    feed(&r.host, 1, &[0.5; 4]);
    wait_until("the new frame", || got(&r).len() == 3);
    let g = got(&r);
    // No buffered audio goes: the frame before the change is still there, then the marker.
    assert_eq!(g[0], Got::Frame(vec![0.25; 4]));
    match &g[1] {
        Got::Event(SinkEvent::Gap { gap_ms }) => assert!(*gap_ms <= 500, "gap {gap_ms}"),
        other => panic!("expected a gap marker, got {other:?}"),
    }
    assert_eq!(g[2], Got::Frame(vec![0.5; 4]));
    assert!(
        r.faults.lock().unwrap().is_empty(),
        "a reopen is not a fault"
    );
}

#[test]
fn cap_18_a_rerouted_stream_keeps_running_and_gets_a_gap_marker() {
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    fault(&r.host, 0, StreamFault::Rerouted);
    wait_until("the gap marker", || {
        got(&r)
            .iter()
            .any(|g| matches!(g, Got::Event(SinkEvent::Gap { .. })))
    });
    assert!(alive(&r, 0));
    assert_eq!(log(&r), ["prefer_built_in false", "build 0", "play 0"]);
    feed(&r.host, 0, &[0.5; 2]);
    wait_until("the frame", || got(&r).len() == 2);
    assert!(r.faults.lock().unwrap().is_empty());
}

#[test]
fn cap_18_a_late_fault_of_the_old_stream_does_not_touch_the_new_one() {
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    fault(&r.host, 0, StreamFault::Invalidated);
    wait_until("the reopen", || alive(&r, 1));
    fault(&r.host, 0, StreamFault::Fatal);
    feed(&r.host, 0, &[0.9; 2]);
    thread::sleep(Duration::from_millis(50));
    assert!(alive(&r, 1));
    assert!(r.faults.lock().unwrap().is_empty());
    assert!(
        !got(&r).contains(&Got::Frame(vec![0.9; 2])),
        "old stream frames are dropped"
    );
}

#[test]
fn cap_18_a_failed_reopen_reports_the_device_lost_after_the_grace() {
    let r = rig_full(
        MicConfig {
            zero_window: Duration::from_secs(60),
            lost_grace: Duration::from_millis(100),
            ..MicConfig::default()
        },
        vec![1, 2],
        None,
    );
    r.device.open(AudioSource::Mic).unwrap();
    fault(&r.host, 0, StreamFault::Invalidated);
    wait_until("the lost fault", || !r.faults.lock().unwrap().is_empty());
    assert_eq!(
        *r.faults.lock().unwrap(),
        [(AudioSource::Mic, MicFault::Lost)]
    );
    assert!(!alive(&r, 0));
    // Two tries (at once and 250 ms later) failed; the slot is closed, so the next open
    // builds again.
    r.device.open(AudioSource::Mic).unwrap();
    assert!(alive(&r, 3));
}

#[test]
fn cap_18_close_after_a_reopen_drops_the_new_stream() {
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    fault(&r.host, 0, StreamFault::Invalidated);
    wait_until("the reopen", || alive(&r, 1));
    r.device.close(AudioSource::Mic);
    assert!(!alive(&r, 1));
}

// ---------------------------------------------------------------------------------------------
// CAP-22: prefer the built-in mic when a Bluetooth headset connects.

#[test]
fn cap_22_a_bluetooth_default_gives_way_to_the_built_in_mic_when_preferred() {
    use InputChoice::{BuiltIn, Default};
    assert_eq!(
        choose_input(true, Some(InputKind::Bluetooth), true),
        BuiltIn
    );
    assert_eq!(
        choose_input(false, Some(InputKind::Bluetooth), true),
        Default
    );
    assert_eq!(
        choose_input(true, Some(InputKind::Bluetooth), false),
        Default
    );
    assert_eq!(choose_input(true, Some(InputKind::BuiltIn), true), Default);
    assert_eq!(choose_input(true, Some(InputKind::Other), true), Default);
    assert_eq!(choose_input(true, None, true), BuiltIn);
}

#[test]
fn cap_22_the_preference_reaches_the_host_before_each_build() {
    let r = rig();
    r.device.set_prefer_built_in(true);
    r.device.open(AudioSource::Mic).unwrap();
    r.device.close(AudioSource::Mic);
    r.device.set_prefer_built_in(false);
    r.device.open(AudioSource::Mic).unwrap();
    assert_eq!(
        log(&r),
        [
            "prefer_built_in true",
            "build 0",
            "play 0",
            "prefer_built_in false",
            "build 1",
            "play 1"
        ]
    );
}

#[test]
fn cap_17a_the_macos_bundle_has_the_mic_usage_text_for_the_prompt() {
    let plist = include_str!("../../../src-tauri/Info.plist");
    assert!(plist.contains("<key>NSMicrophoneUsageDescription</key>"));
    assert!(plist.contains("only while you hold the talk key"));
}

// ---------------------------------------------------------------------------------------------
// #453: the mic fault in `capture.state` (CAP-17b) and the app setting hook (CAP-22).

fn ptt() -> CaptureRequest {
    CaptureRequest {
        mog_id: "mog-1".to_string(),
        mode: CaptureMode::Ptt,
        others_track: false,
    }
}

fn core_with(device: FakeAudioDevice) -> (CaptureCore, Arc<RecordingCaptureEvents>) {
    let events = Arc::new(RecordingCaptureEvents::default());
    let core = CaptureCore::new(Box::new(device), Arc::new(SimClock::new()), events.clone());
    (core, events)
}

#[test]
fn cap_17b_an_idle_core_reports_no_mic_reading() {
    let (core, _) = core_with(FakeAudioDevice::new());
    assert_eq!(core.status().mic, None);
}

#[test]
fn cap_17b_zero_buffers_put_the_mic_fault_in_capture_state() {
    let (core, events) = core_with(FakeAudioDevice::new());
    let lease = core.start(ptt()).unwrap();
    assert_eq!(
        core.status().mic,
        Some(MicHealth::Ok),
        "a granted lease is ok"
    );
    core.mic_silent();
    let last = events.states().last().cloned().expect("a capture.state");
    assert!(!last.recording);
    assert_eq!(last.mic, Some(MicHealth::ZeroBuffers));
    assert_eq!(core.status().mic, Some(MicHealth::ZeroBuffers));
    drop(lease);
}

#[test]
fn cap_17b_the_next_granted_lease_resets_the_mic_to_ok() {
    let (core, _) = core_with(FakeAudioDevice::new());
    core.start(ptt()).unwrap();
    core.mic_silent();
    core.start(ptt()).unwrap();
    assert_eq!(core.status().mic, Some(MicHealth::Ok));
}

#[test]
fn cap_17b_a_lost_mic_reports_no_device() {
    let (core, _) = core_with(FakeAudioDevice::new());
    core.start(ptt()).unwrap();
    core.device_failed(AudioSource::Mic);
    assert_eq!(core.status().mic, Some(MicHealth::NoDevice));
}

#[test]
fn cap_17b_a_refused_mic_open_puts_its_code_in_capture_state() {
    for (code, health) in [
        ("permission_denied", MicHealth::PermissionDenied),
        ("no_device", MicHealth::NoDevice),
    ] {
        let device = FakeAudioDevice::new();
        device.log().fail_next_open = Some((Some(AudioSource::Mic), code));
        let (core, events) = core_with(device);
        assert!(core.start(ptt()).is_err(), "{code}");
        assert_eq!(core.status().mic, Some(health), "{code}");
        assert_eq!(
            events.states().last().and_then(|s| s.mic),
            Some(health),
            "{code}: the brain got the fault"
        );
    }
}

#[test]
fn cap_22_the_app_setting_reaches_the_mic_device_at_the_next_build() {
    let r = polled(Vec::new(), Duration::from_secs(10));
    let settings = mog_core::app_settings::AppSettings::default();
    settings.set_mic_preference(Arc::new(r.device.preference()));
    settings.write(mog_core::types::AppSetting {
        key: mog_core::types::AppSettingKey::CapturePreferBuiltInMic,
        value: true,
    });
    r.device.open(AudioSource::Mic).unwrap();
    assert_eq!(log(&r)[..1], ["prefer_built_in true"]);
}
