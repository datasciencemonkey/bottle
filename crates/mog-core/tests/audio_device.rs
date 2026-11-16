//! Tests for the mic `AudioDevice` backend (#388): the audio thread, open and close as
//! messages, close before return, the open/close race and the failure path.
//!
//! A fake host and a fake stream stand in for cpal. The audio is a fixture WAV file. No test
//! opens the user's mic.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use mog_core::audio_device::{
    codes, mix_to_mono, DataFn, FailureLink, FaultFn, FrameSink, Interleaved, MicDevice, MicFrame,
    MicHost, StreamFault,
};
use mog_core::capture::{
    AudioDevice, AudioSource, CaptureCore, CaptureRequest, CaptureService, DeviceError, StopReason,
};
use mog_core::clock::SimClock;
use mog_core::testkit::capture::RecordingCaptureEvents;
use mog_core::types::CaptureMode;

const WAIT: Duration = Duration::from_secs(5);

// ---------- the fixture WAV (stereo, 16-bit PCM, 48 kHz, 480 frames) ----------

struct Wav {
    channels: u16,
    sample_rate: u32,
    samples: Vec<f32>,
}

fn fixture_wav() -> Wav {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/audio/stereo-48k-10ms.wav"
    );
    let bytes = std::fs::read(path).expect("fixture wav");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let (mut channels, mut sample_rate, mut samples) = (0, 0, Vec::new());
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32_at(at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " {
            assert_eq!(u16_at(body), 1, "PCM");
            channels = u16_at(body + 2);
            sample_rate = u32_at(body + 4);
            assert_eq!(u16_at(body + 14), 16, "16-bit");
        } else if id == b"data" {
            samples = bytes[body..body + len]
                .chunks_exact(2)
                .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
                .collect();
        }
        at = body + len + (len & 1);
    }
    Wav {
        channels,
        sample_rate,
        samples,
    }
}

// ---------- the fake host ----------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ev {
    Built(u64),
    Played(u64),
    Dropped(u64),
}

/// A pause point inside the fake host: the host tells the test it got there, then waits.
struct Gate {
    entered: Sender<()>,
    release: Receiver<()>,
}

struct GateCtl {
    entered: Receiver<()>,
    release: Sender<()>,
}

impl GateCtl {
    fn wait_entered(&self) {
        self.entered
            .recv_timeout(WAIT)
            .expect("the host reached the gate");
    }
    fn release(&self) {
        self.release.send(()).expect("release the gate");
    }
}

fn gate() -> (Gate, GateCtl) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    (
        Gate {
            entered: entered_tx,
            release: release_rx,
        },
        GateCtl {
            entered: entered_rx,
            release: release_tx,
        },
    )
}

#[derive(Default)]
struct Shared {
    events: Mutex<Vec<(Ev, ThreadId)>>,
    host_thread: Mutex<Option<ThreadId>>,
    /// The callbacks of every stream ever built. They stay after the drop, like a callback
    /// that is in flight when a stream closes.
    data: Mutex<HashMap<u64, DataFn>>,
    faults: Mutex<HashMap<u64, FaultFn>>,
    next_id: AtomicUsize,
    build_error: Mutex<Option<DeviceError>>,
    play_error: Mutex<Option<DeviceError>>,
    build_gate: Mutex<Option<Gate>>,
    play_gate: Mutex<Option<Gate>>,
    panic_on_build: AtomicBool,
}

#[derive(Clone, Default)]
struct Probe(Arc<Shared>);

impl Probe {
    fn log(&self, ev: Ev) {
        self.0
            .events
            .lock()
            .unwrap()
            .push((ev, thread::current().id()));
    }
    fn events(&self) -> Vec<Ev> {
        self.0
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|(e, _)| e.clone())
            .collect()
    }
    fn threads(&self) -> Vec<ThreadId> {
        self.0
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|(_, t)| *t)
            .collect()
    }
    fn host_thread(&self) -> Option<ThreadId> {
        *self.0.host_thread.lock().unwrap()
    }
    fn hold_build(&self) -> GateCtl {
        let (g, ctl) = gate();
        *self.0.build_gate.lock().unwrap() = Some(g);
        ctl
    }
    fn hold_play(&self) -> GateCtl {
        let (g, ctl) = gate();
        *self.0.play_gate.lock().unwrap() = Some(g);
        ctl
    }
    fn fail_build(&self, e: DeviceError) {
        *self.0.build_error.lock().unwrap() = Some(e);
    }
    fn fail_play(&self, e: DeviceError) {
        *self.0.play_error.lock().unwrap() = Some(e);
    }
    /// Run the data callback of stream `id`, as the host's real-time thread does.
    fn push(&self, id: u64, samples: &[f32], channels: u16, sample_rate: u32) {
        let mut data = self.0.data.lock().unwrap();
        let cb = data.get_mut(&id).expect("a built stream");
        cb(Interleaved {
            samples,
            channels,
            sample_rate,
        });
    }
    /// Run the error callback of stream `id`, as a host thread does.
    fn fault(&self, id: u64, fault: StreamFault) {
        let mut faults = self.0.faults.lock().unwrap();
        let cb = faults.get_mut(&id).expect("a built stream");
        cb(fault);
    }
}

/// A fake stream. Like a cpal stream it is not `Send`; dropping it stops it.
struct FakeStream {
    id: u64,
    probe: Probe,
    _not_send: PhantomData<*const ()>,
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.probe.log(Ev::Dropped(self.id));
    }
}

/// A fake host. Like a cpal host it is made on the audio thread and is not `Send`.
struct FakeHost {
    probe: Probe,
    _not_send: PhantomData<*const ()>,
}

fn wait_at(slot: &Mutex<Option<Gate>>) {
    let g = slot.lock().unwrap().take();
    if let Some(g) = g {
        g.entered.send(()).unwrap();
        g.release
            .recv_timeout(WAIT)
            .expect("the test releases the gate");
    }
}

impl MicHost for FakeHost {
    type Stream = FakeStream;

    fn build(&mut self, on_data: DataFn, on_fault: FaultFn) -> Result<FakeStream, DeviceError> {
        assert!(
            !self.probe.0.panic_on_build.load(Ordering::SeqCst),
            "fake host panics in build"
        );
        wait_at(&self.probe.0.build_gate);
        if let Some(e) = self.probe.0.build_error.lock().unwrap().take() {
            return Err(e);
        }
        let id = self.probe.0.next_id.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        self.probe.0.data.lock().unwrap().insert(id, on_data);
        self.probe.0.faults.lock().unwrap().insert(id, on_fault);
        self.probe.log(Ev::Built(id));
        Ok(FakeStream {
            id,
            probe: self.probe.clone(),
            _not_send: PhantomData,
        })
    }

    fn play(&mut self, stream: &FakeStream) -> Result<(), DeviceError> {
        if let Some(e) = self.probe.0.play_error.lock().unwrap().take() {
            return Err(e);
        }
        self.probe.log(Ev::Played(stream.id));
        wait_at(&self.probe.0.play_gate);
        Ok(())
    }
}

/// One frame the sink got: mono samples and the sample rate.
type Got = (Vec<f32>, u32);

/// An in-RAM test sink: it keeps every frame it gets.
#[derive(Clone, Default)]
struct RamSink {
    frames: Arc<Mutex<Vec<Got>>>,
}

impl RamSink {
    fn frames(&self) -> Vec<Got> {
        self.frames.lock().unwrap().clone()
    }
}

impl FrameSink for RamSink {
    fn frame(&mut self, frame: MicFrame<'_>) {
        self.frames
            .lock()
            .unwrap()
            .push((frame.samples.to_vec(), frame.sample_rate));
    }
}

struct Rig {
    device: MicDevice,
    link: FailureLink,
    probe: Probe,
    sink: RamSink,
}

fn rig() -> Rig {
    let probe = Probe::default();
    let sink = RamSink::default();
    let host_probe = probe.clone();
    let (device, link) = MicDevice::spawn(
        move || {
            *host_probe.0.host_thread.lock().unwrap() = Some(thread::current().id());
            FakeHost {
                probe: host_probe,
                _not_send: PhantomData,
            }
        },
        Box::new(sink.clone()),
    )
    .expect("spawn the audio thread");
    Rig {
        device,
        link,
        probe,
        sink,
    }
}

/// Run `f` on a helper thread and fail the test if it does not return within `limit`.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).expect("returned in time")
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

// ---------- MOG-09: nothing opens until open; close stops the stream before it returns ----------

#[test]
fn mog_09_no_stream_exists_before_open() {
    let r = rig();
    wait_until("the host on the audio thread", || {
        r.probe.host_thread().is_some()
    });
    assert_eq!(r.probe.events(), vec![]);
    // Closing a closed stream does nothing.
    r.device.close(AudioSource::Mic);
    assert_eq!(r.probe.events(), vec![]);
}

#[test]
fn mog_09_close_drops_the_stream_before_it_returns() {
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    r.device.close(AudioSource::Mic);
    assert_eq!(
        r.probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
    // A second close does nothing.
    r.device.close(AudioSource::Mic);
    assert_eq!(r.probe.events().len(), 3);
}

#[test]
fn mog_09_close_during_a_slow_build_returns_at_once_and_the_stream_never_plays() {
    let r = rig();
    let held = r.probe.hold_build();
    let device = Arc::new(r.device);
    let opener = {
        let device = Arc::clone(&device);
        thread::spawn(move || device.open(AudioSource::Mic))
    };
    held.wait_entered();
    // The open is stuck in the host. close must not wait for it (MB-05).
    let closer = Arc::clone(&device);
    within(Duration::from_secs(1), move || {
        closer.close(AudioSource::Mic)
    });
    held.release();
    let opened = opener.join().unwrap();
    assert_eq!(opened, Err(codes::OPEN_CANCELLED));
    assert_eq!(r.probe.events(), vec![Ev::Built(1), Ev::Dropped(1)]);
    // R2 closes again after the open returns: nothing is left to close.
    device.close(AudioSource::Mic);
    assert_eq!(r.probe.events().len(), 2);
    // The next open works.
    device.open(AudioSource::Mic).unwrap();
    assert_eq!(r.probe.events()[2..], [Ev::Built(2), Ev::Played(2)]);
}

#[test]
fn mog_09_close_during_play_drops_the_stream_before_the_open_returns() {
    let r = rig();
    let held = r.probe.hold_play();
    let device = Arc::new(r.device);
    let opener = {
        let device = Arc::clone(&device);
        thread::spawn(move || device.open(AudioSource::Mic))
    };
    held.wait_entered();
    let closer = Arc::clone(&device);
    within(Duration::from_secs(1), move || {
        closer.close(AudioSource::Mic)
    });
    held.release();
    assert_eq!(opener.join().unwrap(), Err(codes::OPEN_CANCELLED));
    assert_eq!(
        r.probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
}

#[test]
fn mog_09_dropping_the_device_stops_an_open_stream() {
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    drop(r.device);
    assert_eq!(
        r.probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
}

// ---------- CAP-01a: one owner, one audio thread ----------

#[test]
fn cap_01a_the_audio_thread_makes_the_host_and_owns_every_stream() {
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    r.device.close(AudioSource::Mic);
    let audio = r.probe.host_thread().expect("the host thread");
    assert_ne!(audio, thread::current().id());
    assert_eq!(r.probe.threads(), vec![audio, audio, audio]);
}

#[test]
fn cap_01a_a_failed_build_or_play_leaves_no_stream() {
    let r = rig();
    r.probe.fail_build(codes::PERMISSION_DENIED);
    assert_eq!(
        r.device.open(AudioSource::Mic),
        Err(codes::PERMISSION_DENIED)
    );
    assert_eq!(r.probe.events(), vec![]);
    r.probe.fail_play(codes::DEVICE_BUSY);
    assert_eq!(r.device.open(AudioSource::Mic), Err(codes::DEVICE_BUSY));
    assert_eq!(r.probe.events(), vec![Ev::Built(1), Ev::Dropped(1)]);
    r.device.open(AudioSource::Mic).unwrap();
    assert_eq!(r.probe.events()[2..], [Ev::Built(2), Ev::Played(2)]);
}

#[test]
fn cap_01a_system_audio_is_not_in_this_backend() {
    let r = rig();
    assert_eq!(
        r.device.open(AudioSource::SystemAudio),
        Err(codes::SYSTEM_AUDIO_UNSUPPORTED)
    );
    r.device.close(AudioSource::SystemAudio);
    assert_eq!(r.probe.events(), vec![]);
}

#[test]
fn cap_01a_a_panicking_host_gives_audio_thread_gone() {
    let r = rig();
    r.probe.0.panic_on_build.store(true, Ordering::SeqCst);
    assert_eq!(
        r.device.open(AudioSource::Mic),
        Err(codes::AUDIO_THREAD_GONE)
    );
    within(Duration::from_secs(1), move || {
        r.device.close(AudioSource::Mic);
        r.device.open(AudioSource::Mic)
    })
    .expect_err("the thread is gone");
}

// ---------- CAP-01b: frames stay in the in-RAM sink ----------

#[test]
fn cap_01b_fixture_frames_reach_the_sink_as_mono() {
    let wav = fixture_wav();
    assert_eq!((wav.channels, wav.sample_rate), (2, 48_000));
    assert_eq!(wav.samples.len(), 960);
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    for block in wav.samples.chunks(320) {
        r.probe.push(1, block, wav.channels, wav.sample_rate);
    }
    let frames = r.sink.frames();
    assert_eq!(frames.len(), 3);
    let mono: Vec<f32> = frames.iter().flat_map(|(s, _)| s.clone()).collect();
    assert!(frames.iter().all(|(_, rate)| *rate == 48_000));
    let expected: Vec<f32> = wav
        .samples
        .chunks_exact(2)
        .map(|f| (f[0] + f[1]) / 2.0)
        .collect();
    assert_eq!(mono, expected);
}

#[test]
fn cap_01b_no_frame_reaches_the_sink_after_close() {
    let r = rig();
    r.device.open(AudioSource::Mic).unwrap();
    r.probe.push(1, &[0.5, 0.5], 1, 16_000);
    r.device.close(AudioSource::Mic);
    // A callback still in flight after close delivers nothing.
    r.probe.push(1, &[0.5, 0.5], 1, 16_000);
    assert_eq!(r.sink.frames().len(), 1);
    // Nor does the old stream once a new one runs.
    r.device.open(AudioSource::Mic).unwrap();
    r.probe.push(1, &[0.25], 1, 16_000);
    r.probe.push(2, &[0.75], 1, 16_000);
    let frames = r.sink.frames();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].0, vec![0.75]);
}

#[test]
fn cap_01b_device_errors_are_content_free_codes() {
    for e in [
        codes::PERMISSION_DENIED,
        codes::NO_DEVICE,
        codes::DEVICE_BUSY,
        codes::UNSUPPORTED_FORMAT,
        codes::DEVICE_FAILED,
        codes::OPEN_CANCELLED,
        codes::AUDIO_THREAD_GONE,
        codes::SYSTEM_AUDIO_UNSUPPORTED,
    ] {
        assert!(
            !e.code.is_empty() && e.code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{}",
            e.code
        );
    }
}

#[test]
fn cap_01b_mix_to_mono_averages_each_frame() {
    let mut out = vec![9.0];
    mix_to_mono(&[0.25, 0.75, -1.0, 1.0, 0.5], 2, &mut out);
    assert_eq!(out, vec![0.5, 0.0]);
    mix_to_mono(&[0.1, 0.2], 1, &mut out);
    assert_eq!(out, vec![0.1, 0.2]);
    mix_to_mono(&[0.1, 0.2], 0, &mut out);
    assert_eq!(out, vec![0.1, 0.2]);
    assert!(MicFrame {
        samples: &[0.0, 0.0],
        sample_rate: 16_000
    }
    .is_digital_silence());
    assert!(!MicFrame {
        samples: &[0.0, 1e-6],
        sample_rate: 16_000
    }
    .is_digital_silence());
}

// ---------- the failure path ----------

#[test]
fn cap_01a_a_fatal_fault_stops_the_stream_and_reports_it_off_the_audio_thread() {
    let r = rig();
    let calls = Arc::new(Mutex::new(Vec::new()));
    {
        let calls = Arc::clone(&calls);
        r.link
            .connect_fn(move |s| calls.lock().unwrap().push((s, thread::current().id())));
    }
    r.device.open(AudioSource::Mic).unwrap();
    // The fault comes from a host thread (here the test thread).
    r.probe.fault(1, StreamFault::Fatal);
    wait_until("the failure report", || !calls.lock().unwrap().is_empty());
    let (source, on) = calls.lock().unwrap()[0];
    assert_eq!(source, AudioSource::Mic);
    assert_ne!(Some(on), r.probe.host_thread());
    assert_eq!(
        r.probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
    // The next open builds a fresh stream (Handy's needs_reopen).
    r.device.open(AudioSource::Mic).unwrap();
    assert_eq!(r.probe.events()[3..], [Ev::Built(2), Ev::Played(2)]);
}

#[test]
fn cap_01a_a_transient_fault_keeps_the_stream() {
    let r = rig();
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = Arc::clone(&calls);
        r.link.connect_fn(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
        });
    }
    r.device.open(AudioSource::Mic).unwrap();
    r.probe.fault(1, StreamFault::Transient);
    r.probe.push(1, &[0.5], 1, 16_000);
    assert_eq!(r.sink.frames().len(), 1);
    // A later fatal fault is the only report.
    r.probe.fault(1, StreamFault::Fatal);
    wait_until("the failure report", || calls.load(Ordering::SeqCst) == 1);
    assert_eq!(
        r.probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
}

#[test]
fn cap_01a_a_late_fault_of_an_old_stream_is_ignored() {
    let r = rig();
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = Arc::clone(&calls);
        r.link.connect_fn(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
        });
    }
    r.device.open(AudioSource::Mic).unwrap();
    r.device.close(AudioSource::Mic);
    r.device.open(AudioSource::Mic).unwrap();
    // The old stream faults late, then the new one faults. Faults reach the audio thread in
    // order, so when the second is reported the first was handled.
    r.probe.fault(1, StreamFault::Fatal);
    r.probe.fault(2, StreamFault::Fatal);
    wait_until("the failure report", || calls.load(Ordering::SeqCst) >= 1);
    thread::sleep(Duration::from_millis(50));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.probe.events(),
        vec![
            Ev::Built(1),
            Ev::Played(1),
            Ev::Dropped(1),
            Ev::Built(2),
            Ev::Played(2),
            Ev::Dropped(2)
        ]
    );
}

// ---------- with the real CaptureCore ----------

fn core_rig() -> (Arc<CaptureCore>, Probe, Arc<RecordingCaptureEvents>) {
    let r = rig();
    let events = Arc::new(RecordingCaptureEvents::default());
    let core = Arc::new(CaptureCore::new(
        Box::new(r.device),
        Arc::new(SimClock::new()),
        events.clone(),
    ));
    r.link.connect(&core);
    (core, r.probe, events)
}

fn req(mode: CaptureMode) -> CaptureRequest {
    CaptureRequest {
        mog_id: "mog-1".to_string(),
        mode,
        others_track: false,
    }
}

#[test]
fn cap_13b_pause_closes_the_stream_before_pause_returns() {
    let (core, probe, _) = core_rig();
    let lease = core.start(req(CaptureMode::Session)).unwrap();
    assert_eq!(probe.events(), vec![Ev::Built(1), Ev::Played(1)]);
    let state = core.pause(lease.id).unwrap();
    assert!(state.paused);
    assert_eq!(probe.events()[2..], [Ev::Dropped(1)]);
    core.resume(lease.id).unwrap();
    assert_eq!(probe.events()[3..], [Ev::Built(2), Ev::Played(2)]);
}

#[test]
fn mog_09_stop_and_stop_all_close_the_stream_before_they_return() {
    let (core, probe, _) = core_rig();
    let lease = core.start(req(CaptureMode::Ptt)).unwrap();
    core.stop(lease.id).unwrap();
    assert_eq!(
        probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
    core.start(req(CaptureMode::HandsFree)).unwrap();
    core.stop_all();
    assert_eq!(
        probe.events()[3..],
        [Ev::Built(2), Ev::Played(2), Ev::Dropped(2)]
    );
}

#[test]
fn mog_09_stop_all_during_a_slow_open_is_quick_and_leaves_the_mic_closed() {
    let (core, probe, _) = core_rig();
    let held = probe.hold_build();
    let starter = {
        let core = Arc::clone(&core);
        thread::spawn(move || core.start(req(CaptureMode::Ptt)))
    };
    held.wait_entered();
    let panic = Arc::clone(&core);
    within(Duration::from_millis(200), move || panic.stop_all());
    held.release();
    assert!(starter.join().unwrap().is_err());
    // R2 runs one device call at a time, so the open finishes and the start's thread closes
    // the stream before `start` returns.
    assert_eq!(
        probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
    assert!(!core.status().recording);
}

#[test]
fn cap_01a_a_fatal_fault_stops_the_lease_with_device_lost() {
    let (core, probe, events) = core_rig();
    core.start(req(CaptureMode::Ptt)).unwrap();
    probe.fault(1, StreamFault::Fatal);
    wait_until("the device_lost stop", || events.last_stop().is_some());
    assert_eq!(events.last_stop().unwrap().reason, StopReason::DeviceLost);
    assert!(!core.status().recording);
    assert_eq!(
        probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
    // The next start opens a fresh stream.
    core.start(req(CaptureMode::Ptt)).unwrap();
    assert_eq!(probe.events()[3..], [Ev::Built(2), Ev::Played(2)]);
}

#[test]
fn cap_01a_the_failure_link_does_not_keep_the_core_alive() {
    let (core, probe, _) = core_rig();
    core.start(req(CaptureMode::Ptt)).unwrap();
    drop(core);
    // Dropping the core closes the mic (CaptureCore's Drop) and ends the audio thread.
    assert_eq!(
        probe.events(),
        vec![Ev::Built(1), Ev::Played(1), Ev::Dropped(1)]
    );
}
