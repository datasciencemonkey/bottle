//! Call recording, capture half (ADR 0103, Refs #65 #107 #108): the mixer, the FLAC encoder,
//! the 55 min part chunker, the system audio device and the recording session.
//!
//! Every test is deterministic: time comes from `SimClock` / `SimWallClock`, audio from fake
//! devices, and files go to a temp dir. No test opens the user's mic or system audio.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::audio_device::system::{
    RecordingDevice, SystemAudioAvailability, SystemAudioDevice, SystemAudioStatus,
};
use mog_core::audio_device::{
    DataFn, FaultFn, FrameSink, Interleaved, MicFrame, MicHost, StreamFault,
};
use mog_core::capture::{
    AudioDevice, AudioSource, CaptureCore, CaptureEvents, CaptureService, DeviceError, StopReason,
    Stopped,
};
use mog_core::clock::{SimClock, SimWallClock};
use mog_core::recording_capture::chunker::{part_file_name, PartWriter};
use mog_core::recording_capture::flac::FlacWriter;
use mog_core::recording_capture::mixer::{soft_clip, Mixer, Track};
use mog_core::recording_capture::permissions::{ask_on_enable, PermissionAsker};
use mog_core::recording_capture::session::{
    FinishedRecording, PartsHandoff, RecordingCaptureError, RecordingSession, SessionConfig,
    Snapshot, StartOptions, StopCause, SystemAudioState,
};
use mog_core::recording_capture::tap::RecordingTap;
use mog_core::types::CaptureState;

// ---------- helpers ----------

fn temp_dir(tag: &str) -> PathBuf {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("rng");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let dir = std::env::temp_dir().join(format!("bottle-rec-test-{tag}-{hex}"));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Decode a FLAC file with an independent decoder (claxon) into 16-bit samples.
fn decode(path: &Path) -> (u32, u32, u32, Vec<i32>) {
    let mut reader = claxon::FlacReader::open(path).expect("valid FLAC");
    let info = reader.streaminfo();
    let samples: Vec<i32> = reader.samples().map(|s| s.expect("sample")).collect();
    (
        info.sample_rate,
        info.channels,
        info.bits_per_sample,
        samples,
    )
}

fn tone(rate: u32, hz: f32, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
        .collect()
}

// ---------- FLAC ----------

#[test]
fn adr_0103_flac_round_trips_16k_mono_16_bit() {
    let dir = temp_dir("flac");
    let path = dir.join("t.flac");
    let input = tone(16_000, 440.0, 0.5, 16_000 * 3 + 123);
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut w = FlacWriter::new(file).unwrap();
        // Odd block sizes: the writer re-blocks.
        for chunk in input.chunks(1_000) {
            w.write(chunk).unwrap();
        }
        assert_eq!(w.samples(), input.len() as u64);
        w.finish().unwrap();
    }
    let (rate, channels, bits, samples) = decode(&path);
    assert_eq!((rate, channels, bits), (16_000, 1, 16));
    assert_eq!(samples.len(), input.len());
    for (got, want) in samples.iter().zip(&input) {
        let want = (want.clamp(-1.0, 1.0) * 32_767.0).round() as i32;
        assert_eq!(*got, want);
    }
    // Compressed: a tone codes well below the 2 bytes per sample of raw PCM.
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size < (input.len() as u64) * 2 / 2, "size {size}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn adr_0103_flac_codes_silence_noise_and_clipping() {
    let dir = temp_dir("flac2");
    let path = dir.join("n.flac");
    // Silence (constant subframes), full-scale noise (verbatim fallback) and out-of-range
    // values (clamped).
    let mut input = vec![0.0f32; 5_000];
    let mut x: u32 = 12_345;
    for _ in 0..9_000 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        input.push(((x >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0);
    }
    input.extend([2.0, -2.0, 1.0, -1.0, f32::NAN]);
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut w = FlacWriter::new(file).unwrap();
        w.write(&input).unwrap();
        w.finish().unwrap();
    }
    let (_, _, _, samples) = decode(&path);
    assert_eq!(samples.len(), input.len());
    assert!(samples[..5_000].iter().all(|s| *s == 0));
    let tail = &samples[samples.len() - 5..];
    assert_eq!(tail, &[32_767, -32_767, 32_767, -32_767, 0]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn adr_0103_flac_with_no_samples_is_still_valid() {
    let dir = temp_dir("flac3");
    let path = dir.join("e.flac");
    {
        let file = std::fs::File::create(&path).unwrap();
        FlacWriter::new(file).unwrap().finish().unwrap();
    }
    let (rate, _, _, samples) = decode(&path);
    assert_eq!(rate, 16_000);
    assert!(samples.is_empty());
    std::fs::remove_dir_all(dir).ok();
}

// ---------- mixer ----------

#[test]
fn adr_0103_soft_clip_is_linear_below_the_knee_and_bounded_above() {
    for x in [0.0f32, 0.1, -0.3, 0.7, -0.74] {
        assert_eq!(soft_clip(x), x);
    }
    for x in [0.9f32, 1.5, 4.0, 100.0] {
        let y = soft_clip(x);
        assert!(y > 0.75 && y < 1.0, "{x} -> {y}");
        assert_eq!(soft_clip(-x), -y);
    }
    // Monotonic.
    let mut last = -1.0f32;
    for i in 0..400 {
        let y = soft_clip(-4.0 + i as f32 * 0.02);
        assert!(y >= last);
        last = y;
    }
    assert_eq!(soft_clip(f32::NAN), 0.0);
}

#[test]
fn adr_0103_mixer_sums_both_tracks_at_16k() {
    let mut m = Mixer::new(1_600);
    let mut out = Vec::new();
    // 48 kHz mic and 16 kHz system audio, both constant.
    m.push(Track::Mic, &vec![0.25; 4_800], 48_000);
    m.push(Track::System, &vec![0.25; 1_600], 16_000);
    m.drain(&mut out);
    // 100 ms at 16 kHz from each, summed.
    assert!(out.len() >= 1_590 && out.len() <= 1_600, "{}", out.len());
    assert!(out[10..].iter().all(|s| (*s - 0.5).abs() < 1e-3));
}

#[test]
fn adr_0103_mixer_goes_on_with_one_track_absent() {
    let mut m = Mixer::new(1_600);
    let mut out = Vec::new();
    for _ in 0..10 {
        m.push(Track::Mic, &vec![0.1; 1_600], 16_000);
        m.drain(&mut out);
    }
    // No system audio at all: the mic still comes out, at most `max_lag` behind.
    assert!(out.len() >= 16_000 - 1_600, "{}", out.len());
    assert!(m.buffered(Track::Mic) <= 1_600);
    m.flush(&mut out);
    assert_eq!(out.len(), 16_000);
    assert!(out.iter().all(|s| (*s - 0.1).abs() < 1e-6));
}

#[test]
fn adr_0103_mixer_buffers_stay_bounded_under_clock_drift() {
    let mut m = Mixer::new(1_600);
    let mut out = Vec::new();
    // System audio runs 1 % fast against the mic for 60 s.
    for _ in 0..600 {
        m.push(Track::Mic, &vec![0.0; 160], 16_000);
        m.push(Track::System, &vec![0.0; 162], 16_000);
        m.drain(&mut out);
        assert!(m.buffered(Track::Mic) <= 1_600 + 162);
        assert!(m.buffered(Track::System) <= 1_600 + 162);
    }
    m.flush(&mut out);
    // No audio is lost: the output is the longer track.
    assert_eq!(out.len(), 600 * 162);
}

#[test]
fn adr_0103_mixer_soft_clips_a_loud_sum() {
    let mut m = Mixer::new(160);
    let mut out = Vec::new();
    m.push(Track::Mic, &vec![0.9; 160], 16_000);
    m.push(Track::System, &vec![0.9; 160], 16_000);
    m.flush(&mut out);
    assert_eq!(out.len(), 160);
    assert!(out.iter().all(|s| *s < 1.0 && *s > 0.75));
}

// ---------- chunker ----------

#[test]
fn adr_0103_parts_roll_at_the_part_limit() {
    let dir = temp_dir("parts");
    // A 1 s part limit stands in for 55 min.
    let mut w = PartWriter::new(&dir, "2026-11-19_0930", 16_000).unwrap();
    for _ in 0..25 {
        w.write(&vec![0.2; 1_000]).unwrap();
    }
    let parts = w.finish().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].file_name, "2026-11-19_0930.part01.flac");
    assert_eq!(parts[1].file_name, "2026-11-19_0930.part02.flac");
    assert_eq!(parts[0].samples, 16_000);
    assert_eq!(parts[1].samples, 9_000);
    assert_eq!(parts[0].offset_ms, 0);
    assert_eq!(parts[1].offset_ms, 1_000);
    assert_eq!(parts[1].duration_ms(), 562);
    for p in &parts {
        assert_eq!(p.path, dir.join(&p.file_name));
        let (_, _, _, s) = decode(&p.path);
        assert_eq!(s.len() as u64, p.samples);
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn adr_0103_part_names_have_two_digits() {
    assert_eq!(part_file_name("a", 1), "a.part01.flac");
    assert_eq!(part_file_name("a", 12), "a.part12.flac");
    assert_eq!(part_file_name("a", 100), "a.part100.flac");
}

#[test]
fn adr_0103_default_part_limit_is_55_minutes() {
    assert_eq!(
        mog_core::recording_capture::PART_MAX_SAMPLES,
        55 * 60 * 16_000
    );
}

#[cfg(unix)]
#[test]
fn adr_0103_temp_audio_is_user_only() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_dir("perm");
    let dir = root.join("rec");
    mog_core::recording_capture::chunker::create_private_dir(&dir).unwrap();
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    let mut w = PartWriter::new(&dir, "n", 16_000).unwrap();
    w.write(&[0.1; 100]).unwrap();
    let parts = w.finish().unwrap();
    let mode = std::fs::metadata(&parts[0].path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    std::fs::remove_dir_all(root).ok();
}

// ---------- system audio device ----------

/// A fake audio host: `build` hands the data callback to the test, which plays frames.
#[derive(Clone, Default)]
struct FakeHost {
    data: Arc<Mutex<Option<DataFn>>>,
    fault: Arc<Mutex<Option<FaultFn>>>,
    fail: Option<DeviceError>,
    builds: Arc<AtomicUsize>,
    live: Arc<AtomicUsize>,
    hang: Option<Duration>,
}

struct FakeStream(Arc<AtomicUsize>);
impl Drop for FakeStream {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl MicHost for FakeHost {
    type Stream = FakeStream;
    fn build(&mut self, on_data: DataFn, on_fault: FaultFn) -> Result<FakeStream, DeviceError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        if let Some(d) = self.hang {
            std::thread::sleep(d);
        }
        if let Some(e) = self.fail {
            return Err(e);
        }
        *self.data.lock().unwrap() = Some(on_data);
        *self.fault.lock().unwrap() = Some(on_fault);
        self.live.fetch_add(1, Ordering::SeqCst);
        Ok(FakeStream(Arc::clone(&self.live)))
    }
    fn play(&mut self, _stream: &FakeStream) -> Result<(), DeviceError> {
        Ok(())
    }
}

impl FakeHost {
    fn play(&self, samples: &[f32], channels: u16, rate: u32) -> bool {
        let mut g = self.data.lock().unwrap();
        match g.as_mut() {
            Some(f) => {
                f(Interleaved {
                    samples,
                    channels,
                    sample_rate: rate,
                });
                true
            }
            None => false,
        }
    }
}

/// A sink that records what it got.
#[derive(Clone, Default)]
struct Collect(Arc<Mutex<Vec<f32>>>);
impl FrameSink for Collect {
    fn frame(&mut self, frame: MicFrame<'_>) {
        self.0.lock().unwrap().extend_from_slice(frame.samples);
    }
}

#[test]
fn adr_0103_system_audio_opens_on_its_own_thread_and_mixes_to_mono() {
    let host = FakeHost::default();
    let got = Collect::default();
    let h = host.clone();
    let (dev, status) =
        SystemAudioDevice::spawn(move || h, Box::new(got.clone()), Duration::from_secs(5)).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Off);
    dev.open(AudioSource::SystemAudio).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Live);
    assert!(host.play(&[0.2, 0.4, 0.6, 0.8], 2, 48_000));
    let v = got.0.lock().unwrap().clone();
    assert_eq!(v.len(), 2);
    assert!((v[0] - 0.3).abs() < 1e-6 && (v[1] - 0.7).abs() < 1e-6);
    dev.close(AudioSource::SystemAudio);
    assert_eq!(
        host.live.load(Ordering::SeqCst),
        0,
        "stream dropped at close"
    );
    assert_eq!(status.get(), SystemAudioAvailability::Off);
    // The mic is not this device's source.
    assert!(dev.open(AudioSource::Mic).is_err());
}

#[test]
fn adr_0103_a_failed_system_audio_open_is_tolerated_as_unavailable() {
    let host = FakeHost {
        fail: Some(DeviceError {
            code: "permission_denied",
        }),
        ..FakeHost::default()
    };
    let h = host.clone();
    let (dev, status) = SystemAudioDevice::spawn(
        move || h,
        Box::new(Collect::default()),
        Duration::from_secs(5),
    )
    .unwrap();
    // A recording goes on mic-only: open does not fail the lease.
    dev.open(AudioSource::SystemAudio).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Unavailable);
    dev.close(AudioSource::SystemAudio);
}

#[test]
fn adr_0103_a_slow_system_audio_open_times_out_and_cleans_up() {
    let host = FakeHost {
        hang: Some(Duration::from_millis(400)),
        ..FakeHost::default()
    };
    let h = host.clone();
    let (dev, status) = SystemAudioDevice::spawn(
        move || h,
        Box::new(Collect::default()),
        Duration::from_millis(50),
    )
    .unwrap();
    dev.open(AudioSource::SystemAudio).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Unavailable);
    dev.close(AudioSource::SystemAudio);
    // The late stream is dropped when its build returns.
    for _ in 0..100 {
        if host.builds.load(Ordering::SeqCst) == 1 && host.live.load(Ordering::SeqCst) == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(450));
    assert_eq!(host.live.load(Ordering::SeqCst), 0);
}

#[test]
fn adr_0103_a_fatal_system_stream_error_marks_it_unavailable() {
    let host = FakeHost::default();
    let h = host.clone();
    let (dev, status) = SystemAudioDevice::spawn(
        move || h,
        Box::new(Collect::default()),
        Duration::from_secs(5),
    )
    .unwrap();
    dev.open(AudioSource::SystemAudio).unwrap();
    (host.fault.lock().unwrap().as_mut().unwrap())(StreamFault::Fatal);
    for _ in 0..200 {
        if status.get() == SystemAudioAvailability::Unavailable {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(status.get(), SystemAudioAvailability::Unavailable);
    dev.close(AudioSource::SystemAudio);
}

#[test]
fn adr_0103_no_backend_means_unavailable_system_audio() {
    let (dev, status) = SystemAudioDevice::unavailable();
    dev.open(AudioSource::SystemAudio).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Unavailable);
}

/// A fake mic device that counts its open streams.
#[derive(Clone, Default)]
struct FakeMic {
    open: Arc<AtomicUsize>,
    opens: Arc<AtomicUsize>,
}
impl AudioDevice for FakeMic {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        assert_eq!(source, AudioSource::Mic);
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.open.store(1, Ordering::SeqCst);
        Ok(())
    }
    fn close(&self, source: AudioSource) {
        assert_eq!(source, AudioSource::Mic);
        self.open.store(0, Ordering::SeqCst);
    }
}

#[test]
fn cap_01a_recording_device_routes_each_source_to_its_backend() {
    let mic = FakeMic::default();
    let host = FakeHost::default();
    let h = host.clone();
    let (sys, status) = SystemAudioDevice::spawn(
        move || h,
        Box::new(Collect::default()),
        Duration::from_secs(5),
    )
    .unwrap();
    let dev = RecordingDevice::new(Box::new(mic.clone()), Box::new(sys));
    dev.open(AudioSource::Mic).unwrap();
    assert_eq!(mic.open.load(Ordering::SeqCst), 1);
    assert_eq!(status.get(), SystemAudioAvailability::Off);
    dev.open(AudioSource::SystemAudio).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Live);
    dev.close(AudioSource::SystemAudio);
    dev.close(AudioSource::Mic);
    assert_eq!(mic.open.load(Ordering::SeqCst), 0);
    assert_eq!(host.live.load(Ordering::SeqCst), 0);
}

// ---------- recording session ----------

#[derive(Default)]
struct Handoff(Mutex<Vec<FinishedRecording>>);
impl PartsHandoff for Handoff {
    fn finished(&self, rec: FinishedRecording) {
        self.0.lock().unwrap().push(rec);
    }
}

/// Forwards `capture.stopped` to the recording session, as the host chain does.
#[derive(Default)]
struct Events(Mutex<Option<Arc<RecordingSession>>>);
impl CaptureEvents for Events {
    fn state(&self, _state: &CaptureState) {}
    fn stopped(&self, stopped: &Stopped) {
        let s = self.0.lock().unwrap().clone();
        if let Some(s) = s {
            s.lease_stopped(stopped.lease_id, stopped.reason);
        }
    }
}

struct Rig {
    session: Arc<RecordingSession>,
    core: Arc<CaptureCore>,
    clock: Arc<SimClock>,
    wall: SimWallClock,
    tap: RecordingTap,
    mic: FakeMic,
    sys_host: FakeHost,
    status: SystemAudioStatus,
    handoff: Arc<Handoff>,
    enabled: Arc<std::sync::atomic::AtomicBool>,
    root: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn rig_with(sys_fail: bool, part_max_samples: u64) -> Rig {
    let clock = Arc::new(SimClock::new());
    let wall = SimWallClock::at(1_763_544_600_000);
    let tap = RecordingTap::new(clock.clone());
    let mic = FakeMic::default();
    let sys_host = FakeHost {
        fail: sys_fail.then_some(DeviceError {
            code: "permission_denied",
        }),
        ..FakeHost::default()
    };
    let h = sys_host.clone();
    let (sys, status) = SystemAudioDevice::spawn(
        move || h,
        Box::new(tap.system_sink()),
        Duration::from_secs(5),
    )
    .unwrap();
    let events = Arc::new(Events::default());
    let core = Arc::new(CaptureCore::new(
        Box::new(RecordingDevice::new(Box::new(mic.clone()), Box::new(sys))),
        clock.clone(),
        events.clone(),
    ));
    let handoff = Arc::new(Handoff::default());
    let enabled = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let root = temp_dir("session");
    let en = Arc::clone(&enabled);
    let session = RecordingSession::new(
        core.clone(),
        tap.clone(),
        status.clone(),
        clock.clone(),
        Arc::new(wall.clone()),
        Arc::new(move || en.load(Ordering::SeqCst)),
        handoff.clone(),
        SessionConfig {
            temp_root: root.clone(),
            part_max_samples,
            ..SessionConfig::default()
        },
    );
    *events.0.lock().unwrap() = Some(Arc::clone(&session));
    Rig {
        session,
        core,
        clock,
        wall,
        tap,
        mic,
        sys_host,
        status,
        handoff,
        enabled,
        root,
    }
}

fn rig() -> Rig {
    rig_with(false, mog_core::recording_capture::PART_MAX_SAMPLES)
}

fn opts(name: &str, length_min: u32) -> StartOptions {
    StartOptions {
        name: name.to_string(),
        length_min,
    }
}

/// Play `ms` of audio on both tracks: the mic through the tap's mic sink, system audio
/// through the fake system host.
fn play(r: &Rig, ms: u64, mic_amp: f32, sys_amp: f32) {
    let mut mic = r.tap.mic_sink();
    for _ in 0..(ms / 10) {
        let m = vec![mic_amp; 480];
        mic.frame(MicFrame {
            samples: &m,
            sample_rate: 48_000,
        });
        let s = vec![sys_amp; 960];
        r.sys_host.play(&s, 2, 48_000);
        r.clock.advance(Duration::from_millis(10));
    }
}

#[test]
fn adr_0103_knob_off_starts_no_capture_path() {
    let r = rig();
    r.enabled.store(false, Ordering::SeqCst);
    let err = r.session.start(opts("n", 60)).unwrap_err();
    assert_eq!(err, RecordingCaptureError::Disabled);
    assert_eq!(err.code(), "recording_disabled");
    assert_eq!(r.mic.opens.load(Ordering::SeqCst), 0, "no mic open");
    assert_eq!(r.sys_host.builds.load(Ordering::SeqCst), 0, "no tap");
    assert_eq!(r.session.snapshot(), Snapshot::Idle);
    assert!(
        std::fs::read_dir(&r.root).unwrap().next().is_none(),
        "no temp file"
    );
}

#[test]
fn adr_0103_record_mixes_mic_and_system_audio_into_a_flac_part() {
    let r = rig();
    let info = r.session.start(opts("2026-11-19_0930", 60)).unwrap();
    assert_eq!(info.name, "2026-11-19_0930");
    assert_eq!(info.started_at_ms, 1_763_544_600_000);
    assert_eq!(info.ends_at_ms, 1_763_544_600_000 + 60 * 60_000);
    assert_eq!(info.system_audio, SystemAudioState::Ok);
    assert_eq!(r.mic.open.load(Ordering::SeqCst), 1);
    assert_eq!(r.status.get(), SystemAudioAvailability::Live);
    play(&r, 2_000, 0.1, 0.2);
    r.wall.advance_ms(2_000);
    let done = r.session.stop(StopCause::User).unwrap();
    assert_eq!(r.mic.open.load(Ordering::SeqCst), 0, "mic closed");
    assert_eq!(r.sys_host.live.load(Ordering::SeqCst), 0, "tap closed");
    assert_eq!(done.cause, StopCause::User);
    assert_eq!(done.parts.len(), 1);
    assert_eq!(done.duration_ms, 2_000);
    assert_eq!(done.parts[0].file_name, "2026-11-19_0930.part01.flac");
    let (rate, ch, bits, samples) = decode(&done.parts[0].path);
    assert_eq!((rate, ch, bits), (16_000, 1, 16));
    assert!(
        (31_000..=32_000).contains(&samples.len()),
        "{}",
        samples.len()
    );
    // 0.1 + 0.2 = 0.3 full scale in the steady part.
    let mid = samples[16_000];
    assert!((mid - (0.3 * 32_767.0) as i32).abs() < 50, "{mid}");
    // The cloud side got the same parts, once.
    let h = r.handoff.0.lock().unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].parts, done.parts);
    assert_eq!(r.session.snapshot(), Snapshot::Idle);
}

#[test]
fn adr_0103_one_recording_at_a_time() {
    let r = rig();
    r.session.start(opts("a", 60)).unwrap();
    let err = r.session.start(opts("b", 60)).unwrap_err();
    assert_eq!(err, RecordingCaptureError::Busy);
    assert_eq!(err.code(), "recording_busy");
    r.session.stop(StopCause::User).unwrap();
    assert_eq!(
        r.session.stop(StopCause::User).unwrap_err().code(),
        "not_recording"
    );
}

#[test]
fn adr_0103_length_must_be_5_to_180_minutes() {
    let r = rig();
    for bad in [0, 4, 181] {
        assert_eq!(
            r.session.start(opts("n", bad)).unwrap_err(),
            RecordingCaptureError::InvalidLength
        );
    }
    assert_eq!(r.mic.opens.load(Ordering::SeqCst), 0);
}

#[test]
fn adr_0103_recording_stops_by_itself_at_the_length_preset() {
    let r = rig();
    let info = r.session.start(opts("n", 5)).unwrap();
    assert_eq!(info.ends_at_ms - info.started_at_ms, 5 * 60_000);
    play(&r, 1_000, 0.1, 0.1);
    // Short of the length: still recording.
    r.clock.advance(Duration::from_millis(5 * 60_000 - 1_001));
    assert!(r.session.tick().is_some());
    assert!(matches!(r.session.snapshot(), Snapshot::Recording(_)));
    r.clock.advance(Duration::from_millis(1));
    r.session.tick();
    assert_eq!(r.session.snapshot(), Snapshot::Idle);
    let h = r.handoff.0.lock().unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].cause, StopCause::LengthReached);
    assert_eq!(h[0].cause.code(), "length_reached");
    assert_eq!(r.mic.open.load(Ordering::SeqCst), 0);
}

#[test]
fn adr_0103_a_90_minute_recording_gives_two_parts() {
    // 10 ms stands in for 1 min: 550 ms parts, a 900 ms recording.
    let r = rig_with(false, 55 * 160);
    r.session.start(opts("n", 90)).unwrap();
    play(&r, 90 * 10, 0.1, 0.0);
    let done = r.session.stop(StopCause::User).unwrap();
    assert_eq!(done.parts.len(), 2);
    assert_eq!(done.parts[0].samples, 55 * 160);
    assert_eq!(done.parts[1].offset_ms, 550);
}

#[test]
fn cap_10a_five_seconds_of_zero_system_audio_is_silent_or_denied() {
    let r = rig();
    r.session.start(opts("n", 60)).unwrap();
    // Mic sound, but system audio is all zeros (a denied System Audio Recording permission).
    play(&r, 4_990, 0.1, 0.0);
    r.session.tick();
    let Snapshot::Recording(info) = r.session.snapshot() else {
        panic!("recording")
    };
    assert_eq!(info.system_audio, SystemAudioState::Ok);
    play(&r, 10, 0.1, 0.0);
    r.session.tick();
    let Snapshot::Recording(info) = r.session.snapshot() else {
        panic!("recording")
    };
    assert_eq!(info.system_audio, SystemAudioState::SilentOrDenied);
    assert_eq!(info.system_audio.code(), "silent_or_denied");
    // Sound comes back: the state clears. The recording never stopped.
    play(&r, 100, 0.1, 0.3);
    r.session.tick();
    let Snapshot::Recording(info) = r.session.snapshot() else {
        panic!("recording")
    };
    assert_eq!(info.system_audio, SystemAudioState::Ok);
    r.session.stop(StopCause::User).unwrap();
}

#[test]
fn adr_0103_no_system_audio_backend_records_mic_only() {
    let r = rig_with(true, mog_core::recording_capture::PART_MAX_SAMPLES);
    let info = r.session.start(opts("n", 60)).unwrap();
    assert_eq!(info.system_audio, SystemAudioState::Unavailable);
    assert_eq!(info.system_audio.code(), "unavailable");
    play(&r, 1_000, 0.2, 0.0);
    r.clock.advance(Duration::from_secs(10));
    r.session.tick();
    let Snapshot::Recording(info) = r.session.snapshot() else {
        panic!("recording")
    };
    assert_eq!(info.system_audio, SystemAudioState::Unavailable);
    let done = r.session.stop(StopCause::User).unwrap();
    let (_, _, _, s) = decode(&done.parts[0].path);
    assert!(s.len() >= 15_000);
    assert!((s[8_000] - (0.2 * 32_767.0) as i32).abs() < 50);
}

#[test]
fn adr_0103_mic_loss_finalizes_the_part_and_hands_it_off() {
    let r = rig();
    r.session.start(opts("n", 60)).unwrap();
    play(&r, 500, 0.1, 0.1);
    r.core.device_failed(AudioSource::Mic);
    assert_eq!(r.session.snapshot(), Snapshot::Idle);
    let h = r.handoff.0.lock().unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].cause, StopCause::MicLost);
    assert_eq!(h[0].parts.len(), 1);
    assert!(decode(&h[0].parts[0].path).3.len() >= 7_000);
    assert_eq!(r.sys_host.live.load(Ordering::SeqCst), 0, "tap closed");
}

#[test]
fn mb_05_panic_stop_finalizes_the_recording() {
    let r = rig();
    r.session.start(opts("n", 60)).unwrap();
    play(&r, 200, 0.1, 0.1);
    r.core.stop_all();
    assert_eq!(r.session.snapshot(), Snapshot::Idle);
    assert_eq!(r.handoff.0.lock().unwrap()[0].cause, StopCause::Panic);
    assert_eq!(r.mic.open.load(Ordering::SeqCst), 0);
}

#[test]
fn adr_0103_sleep_stops_and_finalizes() {
    let r = rig();
    r.session.start(opts("n", 60)).unwrap();
    play(&r, 200, 0.1, 0.1);
    r.session
        .session_event(mog_core::token_events::SessionEvent::Sleep);
    assert_eq!(r.session.snapshot(), Snapshot::Idle);
    assert_eq!(r.handoff.0.lock().unwrap()[0].cause, StopCause::Sleep);
    // A screen lock does not stop a recording.
    r.session.start(opts("m", 60)).unwrap();
    r.session
        .session_event(mog_core::token_events::SessionEvent::ScreenLock);
    assert!(matches!(r.session.snapshot(), Snapshot::Recording(_)));
    r.session.stop(StopCause::Quit).unwrap();
    assert_eq!(r.handoff.0.lock().unwrap()[1].cause, StopCause::Quit);
}

#[test]
fn adr_0103_ptt_audio_before_the_recording_is_not_recorded() {
    let r = rig();
    // Frames before the start go nowhere.
    let mut mic = r.tap.mic_sink();
    mic.frame(MicFrame {
        samples: &[0.9; 4_800],
        sample_rate: 48_000,
    });
    r.session.start(opts("n", 60)).unwrap();
    play(&r, 100, 0.1, 0.0);
    let done = r.session.stop(StopCause::User).unwrap();
    let (_, _, _, s) = decode(&done.parts[0].path);
    assert!(s
        .iter()
        .all(|v| (*v - (0.1 * 32_767.0) as i32).abs() < 50 || *v == 0));
}

#[test]
fn adr_0103_state_observer_sees_start_and_stop() {
    let r = rig();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = Arc::clone(&seen);
    r.session.set_observer(Arc::new(move |snap: &Snapshot| {
        s2.lock().unwrap().push(snap.clone())
    }));
    r.session.start(opts("n", 60)).unwrap();
    r.session.stop(StopCause::User).unwrap();
    let seen = seen.lock().unwrap();
    assert!(matches!(seen.first(), Some(Snapshot::Recording(_))));
    assert_eq!(seen.last(), Some(&Snapshot::Idle));
}

#[test]
fn adr_0103_a_recording_mic_stop_reason_maps_to_a_cause() {
    assert_eq!(
        StopCause::from_lease_stop(StopReason::DeviceLost),
        StopCause::MicLost
    );
    assert_eq!(
        StopCause::from_lease_stop(StopReason::ZeroBuffers),
        StopCause::MicLost
    );
    assert_eq!(
        StopCause::from_lease_stop(StopReason::Panic),
        StopCause::Panic
    );
    assert_eq!(
        StopCause::from_lease_stop(StopReason::Preempted),
        StopCause::MicLost
    );
}

// ---------- permission ask on knob ON ----------

#[derive(Default)]
struct Asker(Mutex<Vec<&'static str>>);
impl PermissionAsker for Asker {
    fn ask_mic(&self) {
        self.0.lock().unwrap().push("mic");
    }
    fn ask_system_audio(&self) {
        self.0.lock().unwrap().push("system_audio");
    }
}

#[test]
fn adr_0103_turning_the_knob_on_asks_for_mic_and_system_audio() {
    let a = Asker::default();
    ask_on_enable(false, true, &a);
    assert_eq!(*a.0.lock().unwrap(), vec!["mic", "system_audio"]);
    // On to on, on to off and off to off ask nothing.
    for (was, now) in [(true, true), (true, false), (false, false)] {
        let a = Asker::default();
        ask_on_enable(was, now, &a);
        assert!(a.0.lock().unwrap().is_empty());
    }
}
