//! Call recording, capture half (ADR 0103, Refs #65 #107 #108): the mixer, the FLAC encoder,
//! the 10 min part chunker, the system audio device and the recording session.
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
use mog_core::recording_capture::chunker::{part_file_name, PartFormat, PartWriter};
use mog_core::recording_capture::flac::FlacWriter;
use mog_core::recording_capture::mixer::{soft_clip, Mixer, Track};
use mog_core::recording_capture::opus::OpusWriter;
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
    // A 1 s part limit stands in for 10 min.
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
fn adr_0103_default_cuts_a_part_every_five_minutes() {
    // Upload in parts during the call: one constant sets the part length.
    assert_eq!(mog_core::recording_capture::PART_MAX_MINUTES, Some(5));
    assert_eq!(
        mog_core::recording_capture::PART_MAX_SAMPLES,
        Some(5 * 60 * 16_000)
    );
    assert_eq!(
        SessionConfig::default().part_max_samples,
        Some(5 * 60 * 16_000)
    );
    assert_eq!(SessionConfig::default().part_format, PartFormat::Opus);
    assert_eq!(
        mog_core::recording_capture::part_max_samples(Some(10)),
        Some(10 * 60 * 16_000)
    );
}

#[test]
fn adr_0103_the_part_encoder_swaps_to_wav() {
    assert_eq!(PartFormat::Flac.extension(), "flac");
    assert_eq!(PartFormat::Wav.extension(), "wav");
    let dir = temp_dir("wav");
    let mut w = PartWriter::with_format(&dir, "n", 16_000, PartFormat::Wav).unwrap();
    w.write(&vec![0.5; 20_000]).unwrap();
    let parts = w.finish().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].file_name, "n.part01.wav");
    let bytes = std::fs::read(&parts[0].path).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    assert_eq!(u16_at(20), 1, "PCM");
    assert_eq!(u16_at(22), 1, "mono");
    assert_eq!(u32_at(24), 16_000);
    assert_eq!(u16_at(34), 16);
    assert_eq!(&bytes[36..40], b"data");
    assert_eq!(u32_at(40), 32_000);
    assert_eq!(u32_at(4) as usize, bytes.len() - 8);
    assert_eq!(bytes.len(), 44 + 32_000);
    assert_eq!(i16::from_le_bytes([bytes[44], bytes[45]]), 16_384);
    assert_eq!(parts[1].samples, 4_000);
    std::fs::remove_dir_all(dir).ok();
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
fn rec_a_late_adopted_system_open_goes_live_and_records_its_diagnostics() {
    // The build (and so the whole open) takes longer than the open timeout, like the review's
    // ~8.6 s `AudioDeviceStart` outliving an 8 s timeout. It still succeeds, so the worker
    // adopts it instead of leaving a live stream that `status` does not know about.
    let host = FakeHost {
        hang: Some(Duration::from_millis(150)),
        ..FakeHost::default()
    };
    let h = host.clone();
    let clock = Arc::new(SimClock::new());
    let wall = SimWallClock::at(1_763_544_600_000);
    let tap = RecordingTap::new(clock.clone());
    let mic = FakeMic::default();
    let (sys, status) = SystemAudioDevice::spawn(
        move || h,
        Box::new(tap.system_sink()),
        Duration::from_millis(30),
    )
    .unwrap();
    let events = Arc::new(Events::default());
    let core = Arc::new(CaptureCore::new(
        Box::new(RecordingDevice::new(Box::new(mic.clone()), Box::new(sys))),
        clock.clone(),
        events.clone(),
    ));
    let handoff = Arc::new(Handoff::default());
    let root = temp_dir("session-cap10c");
    let session = RecordingSession::new(
        core.clone(),
        tap.clone(),
        status.clone(),
        clock.clone(),
        Arc::new(wall.clone()),
        Arc::new(|| true),
        handoff.clone(),
        SessionConfig {
            temp_root: root.clone(),
            ..SessionConfig::default()
        },
    );
    *events.0.lock().unwrap() = Some(Arc::clone(&session));
    let info = session.start(opts("cap-10c", 60)).unwrap();
    // The open already answered `Unavailable` (mic-only, for now); the late build+play has not
    // resolved yet.
    assert_eq!(info.system_audio, SystemAudioState::Unavailable);
    for _ in 0..100 {
        if status.get() == SystemAudioAvailability::Live {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        status.get(),
        SystemAudioAvailability::Live,
        "the late build was adopted"
    );
    // CAP-10c: `system_state` has no latch, so the session follows the device's own late
    // adoption on the very next tick (the host's ticker calls this every 250 ms); it must not
    // stay stuck at `unavailable` (the owner's exact symptom). No sound was ever fed to the
    // tap and the sim clock never advanced, so once live this reads `ok`, not
    // `silent_or_denied`.
    for _ in 0..100 {
        session.tick();
        if matches!(
            session.snapshot(),
            Snapshot::Recording(ref i) if i.system_audio == SystemAudioState::Ok
        ) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        matches!(
            session.snapshot(),
            Snapshot::Recording(ref i) if i.system_audio == SystemAudioState::Ok
        ),
        "a late-adopted tap must leave `unavailable` once it is really live: got {:?}",
        session.snapshot()
    );
    let done = session.stop(StopCause::User).unwrap();
    assert_eq!(
        done.system_audio,
        SystemAudioState::Ok,
        "`FinishedRecording.system_audio` must reflect the late adoption too, not stay stuck"
    );
    assert!(
        done.system_open_timed_out,
        "the resolution took longer than its timeout"
    );
    assert!(
        done.system_open_ms >= 150,
        "system_open_ms should cover the real build time, got {}",
        done.system_open_ms
    );
    let _ = std::fs::remove_dir_all(root);
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

/// The finished recordings, and a log of the hand-off calls in order.
#[derive(Default)]
struct Handoff(Mutex<Vec<FinishedRecording>>, Mutex<Vec<String>>);
impl PartsHandoff for Handoff {
    fn started(&self, rec: &mog_core::recording_capture::session::StartedRecording) {
        self.1.lock().unwrap().push(format!("started:{}", rec.name));
    }
    fn part_ready(&self, _id: &str, part: &mog_core::recording_capture::chunker::PartFile) {
        // The part is closed when it is announced: it must be a whole Ogg Opus file.
        let bytes = std::fs::read(&part.path).unwrap();
        assert!(!bytes.is_empty());
        self.1
            .lock()
            .unwrap()
            .push(format!("part:{}", part.file_name));
    }
    fn finished(&self, rec: FinishedRecording) {
        self.1.lock().unwrap().push("finished".to_owned());
        self.0.lock().unwrap().push(rec);
    }
}

/// Forwards `capture.stopped` to the recording session, as the host chain does.
#[derive(Default)]
struct Events(Mutex<Option<Arc<RecordingSession>>>);
impl CaptureEvents for Events {
    fn overlay_granted(&self, _lease: &mog_core::capture::Lease) {}
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

fn rig_with(sys_fail: bool, part_max_samples: Option<u64>) -> Rig {
    rig_full(sys_fail, part_max_samples, PartFormat::Flac)
}

/// FLAC in most session tests: claxon checks the content.
fn rig_full(sys_fail: bool, part_max_samples: Option<u64>, part_format: PartFormat) -> Rig {
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
            part_format,
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
    // One part: most tests look at one file.
    rig_with(false, None)
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
    // One part (no cap): `<name>.<ext>`, no part number.
    assert_eq!(done.parts[0].file_name, "2026-11-19_0930.flac");
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
fn adr_0103_length_must_be_5_to_120_minutes() {
    let r = rig();
    for bad in [0, 4, 121, 181] {
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
fn adr_0103_a_90_minute_recording_gives_nine_10_minute_parts() {
    // 10 ms stands in for 1 min: 100 ms parts, a 900 ms recording.
    let r = rig_with(false, Some(10 * 160));
    r.session.start(opts("n", 90)).unwrap();
    play(&r, 90 * 10, 0.1, 0.0);
    let done = r.session.stop(StopCause::User).unwrap();
    assert_eq!(done.parts.len(), 9);
    assert_eq!(done.parts[0].samples, 10 * 160);
    assert_eq!(done.parts[1].offset_ms, 100);
    assert_eq!(done.parts[8].file_name, "n.part09.flac");
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
fn cap_10a_denied_system_audio_still_records_the_mic_and_never_stops_the_call() {
    // System Audio Recording not allowed: the tap gives zeros. The notice state is set, but the
    // recording runs on and its part holds the whole mic track.
    let r = rig();
    r.session.start(opts("n", 60)).unwrap();
    play(&r, 8_000, 0.2, 0.0);
    r.session.tick();
    let Snapshot::Recording(info) = r.session.snapshot() else {
        panic!("still recording")
    };
    assert_eq!(info.system_audio, SystemAudioState::SilentOrDenied);
    let done = r.session.stop(StopCause::User).unwrap();
    assert_eq!(done.cause, StopCause::User);
    assert_eq!(done.parts.len(), 1);
    let (_, _, _, s) = decode(&done.parts[0].path);
    assert!(s.len() >= 8 * 16_000 - 1_600, "{} samples", s.len());
    assert!(
        (s[64_000] - (0.2 * 32_767.0) as i32).abs() < 50,
        "the mic is in it"
    );
}

#[test]
fn rec_capture_counts_say_whether_mic_frames_reached_the_writer() {
    // A stop with no mic frame at all: no part, and the counts show it (no samples kept).
    let r = rig();
    r.session.start(opts("n", 60)).unwrap();
    let none = r.session.stop(StopCause::User).unwrap();
    assert!(none.parts.is_empty());
    assert_eq!(none.capture.mic_blocks, 0);
    assert_eq!(none.capture.first_mic_ms, None);
    // A normal call: every block counts, from the arm on.
    r.session.start(opts("m", 60)).unwrap();
    r.clock.advance(Duration::from_millis(30));
    play(&r, 500, 0.1, 0.1);
    let done = r.session.stop(StopCause::User).unwrap();
    assert_eq!(done.capture.mic_blocks, 50);
    assert_eq!(done.capture.gated_mic_blocks, 0);
    assert!(done.capture.system_blocks > 0);
    assert_eq!(done.capture.dropped, 0);
    assert_eq!(done.capture.first_mic_ms, Some(30));
}

#[test]
fn adr_0103_no_system_audio_backend_records_mic_only() {
    let r = rig_with(true, None);
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

/// Captures, from inside the hand-off's `finished()` call (which `stop()` makes synchronously,
/// after it has already notified `Snapshot::Idle`), whether the session still reports the
/// recording's name as `finishing` at that exact moment. Review of #592 (the finding on
/// `session.rs`'s `stop`): the cloud-side scan and Retry must never see a window where the
/// name is neither `Snapshot::Recording` nor `finishing`, yet the writer thread has not
/// necessarily finished flushing the last part to disk.
#[derive(Default)]
struct FinishingWitness {
    session: Mutex<Option<Arc<RecordingSession>>>,
    seen: Mutex<Option<(bool, Option<String>)>>,
}

impl PartsHandoff for FinishingWitness {
    fn finished(&self, _rec: FinishedRecording) {
        if let Some(s) = self.session.lock().unwrap().clone() {
            let idle = matches!(s.snapshot(), Snapshot::Idle);
            *self.seen.lock().unwrap() = Some((idle, s.finishing_name()));
        }
    }
}

#[test]
fn rec_finishing_name_covers_the_window_snapshot_already_reads_idle() {
    let clock = Arc::new(SimClock::new());
    let wall = SimWallClock::at(1_763_544_600_000);
    let tap = RecordingTap::new(clock.clone());
    let mic = FakeMic::default();
    let (sys, status) = SystemAudioDevice::spawn(
        FakeHost::default,
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
    let root = temp_dir("session-finishing");
    let witness = Arc::new(FinishingWitness::default());
    let session = RecordingSession::new(
        core.clone(),
        tap.clone(),
        status.clone(),
        clock.clone(),
        Arc::new(wall.clone()),
        Arc::new(|| true),
        witness.clone(),
        SessionConfig {
            temp_root: root.clone(),
            ..SessionConfig::default()
        },
    );
    *events.0.lock().unwrap() = Some(Arc::clone(&session));
    *witness.session.lock().unwrap() = Some(Arc::clone(&session));

    session.start(opts("2026-11-19_0905", 60)).unwrap();
    assert_eq!(
        session.finishing_name(),
        None,
        "not finishing before a stop"
    );
    session.stop(StopCause::User).unwrap();

    let (was_idle_in_handoff, finishing_in_handoff) = witness.seen.lock().unwrap().take().unwrap();
    assert!(
        was_idle_in_handoff,
        "snapshot() already reads Idle by the hand-off"
    );
    assert_eq!(
        finishing_in_handoff,
        Some("2026-11-19_0905".to_owned()),
        "finishing_name() must still name the recording at that exact moment"
    );
    assert_eq!(
        session.finishing_name(),
        None,
        "cleared once stop() has returned (the guard dropped)"
    );
    std::fs::remove_dir_all(root).ok();
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

// ---------- Opus (the decided codec) ----------

/// The Ogg packets of an Opus file.
fn ogg_packets(bytes: &[u8]) -> Vec<ogg::Packet> {
    let mut r = ogg::PacketReader::new(std::io::Cursor::new(bytes.to_vec()));
    let mut out = Vec::new();
    while let Some(p) = r.read_packet().expect("valid Ogg") {
        out.push(p);
    }
    out
}

/// Decode the audio packets with the same libopus port, at 16 kHz.
fn opus_decode(packets: &[ogg::Packet]) -> Vec<i16> {
    let mut err = 0;
    let dec = unsafe { unsafe_libopus::opus_decoder_create(16_000, 1, &mut err) };
    assert_eq!(err, 0);
    let mut pcm = Vec::new();
    let mut buf = vec![0i16; 5_760];
    for p in packets {
        let n = unsafe {
            unsafe_libopus::opus_decode(
                dec,
                p.data.as_ptr(),
                p.data.len() as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
                0,
            )
        };
        assert!(n > 0, "decode error {n}");
        pcm.extend_from_slice(&buf[..n as usize]);
    }
    unsafe { unsafe_libopus::opus_decoder_destroy(dec) };
    pcm
}

fn rms_i16(s: &[i16]) -> f64 {
    (s.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / s.len().max(1) as f64).sqrt()
}

#[test]
fn adr_0103_opus_part_is_ogg_with_head_tags_and_exact_granule() {
    assert_eq!(PartFormat::Opus.extension(), "opus");
    assert_eq!(PartFormat::Opus.mime(), "audio/ogg");
    let input = tone(16_000, 300.0, 0.4, 16_000 * 3 + 77);
    let mut w = OpusWriter::new(std::io::Cursor::new(Vec::new())).unwrap();
    for c in input.chunks(700) {
        w.write(c).unwrap();
    }
    assert_eq!(w.samples(), input.len() as u64);
    let bytes = w.finish().unwrap().into_inner();
    assert_eq!(&bytes[..4], b"OggS");
    let packets = ogg_packets(&bytes);
    // OpusHead: version 1, mono, pre-skip, 16 kHz input, gain 0, mapping 0.
    let head = &packets[0].data;
    assert_eq!(&head[..8], b"OpusHead");
    assert_eq!(head.len(), 19);
    assert_eq!(head[8], 1);
    assert_eq!(head[9], 1);
    let pre_skip = u64::from(u16::from_le_bytes([head[10], head[11]]));
    assert!(pre_skip > 0);
    assert_eq!(
        u32::from_le_bytes([head[12], head[13], head[14], head[15]]),
        16_000
    );
    assert_eq!(&head[16..19], &[0, 0, 0]);
    assert_eq!(&packets[1].data[..8], b"OpusTags");
    // The last page's granule (48 kHz) is pre-skip plus the input length: the decoder trims
    // the padding of the last frame.
    let last = packets.last().unwrap();
    assert!(last.last_in_stream());
    assert_eq!(last.absgp_page(), pre_skip + input.len() as u64 * 3);
    // Round trip: about the same loudness after the pre-skip.
    let pcm = opus_decode(&packets[2..]);
    assert!(pcm.len() as u64 >= input.len() as u64 + pre_skip / 3);
    let skip = (pre_skip / 3) as usize;
    let want: Vec<i16> = input.iter().map(|v| (v * 32_767.0) as i16).collect();
    let (a, b) = (
        rms_i16(&pcm[skip + 1_600..skip + 40_000]),
        rms_i16(&want[1_600..40_000]),
    );
    assert!((a / b - 1.0).abs() < 0.25, "rms {a} vs {b}");
}

#[test]
fn adr_0103_an_empty_opus_part_is_still_valid() {
    let w = OpusWriter::new(std::io::Cursor::new(Vec::new())).unwrap();
    let bytes = w.finish().unwrap().into_inner();
    let packets = ogg_packets(&bytes);
    assert_eq!(&packets[0].data[..8], b"OpusHead");
    assert!(packets.last().unwrap().last_in_stream());
}

#[test]
fn adr_0103_sixty_minutes_of_opus_fit_the_24_mb_base64_budget() {
    // 20 s of a speech-like signal (a tone under noise bursts), scaled to 60 min.
    let mut x: u32 = 7;
    let input: Vec<f32> = (0..16_000 * 20)
        .map(|i| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = ((x >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0;
            let burst = if (i / 4_000) % 3 == 0 { 0.0 } else { 1.0 };
            burst * (0.3 * (i as f32 * 0.07).sin() + 0.1 * noise)
        })
        .collect();
    let mut w = OpusWriter::new(std::io::Cursor::new(Vec::new())).unwrap();
    w.write(&input).unwrap();
    let bytes = w.finish().unwrap().into_inner();
    let per_hour = bytes.len() as f64 * (3_600.0 / 20.0);
    let base64 = per_hour * 4.0 / 3.0;
    assert!(base64 < 24_000_000.0, "{base64} bytes of base64 per hour");
    // About 16 kbps: well under 3 KB/s.
    assert!(bytes.len() < 20 * 3_000, "{}", bytes.len());
}

#[test]
fn adr_0103_the_default_recording_is_one_opus_file() {
    let r = rig_full(false, None, PartFormat::Opus);
    r.session.start(opts("2026-11-19_0930", 60)).unwrap();
    play(&r, 1_000, 0.1, 0.2);
    let done = r.session.stop(StopCause::User).unwrap();
    assert_eq!(done.parts.len(), 1);
    assert_eq!(done.parts[0].file_name, "2026-11-19_0930.opus");
    let bytes = std::fs::read(&done.parts[0].path).unwrap();
    assert_eq!(&bytes[..4], b"OggS");
    assert!(bytes.windows(8).any(|w| w == b"OpusHead"));
}

// ---------- the ADR 0103 contract seam ----------

#[test]
fn adr_0103_the_session_is_the_contract_recording_capture() {
    use mog_core::recording::RecordingCapture;
    use mog_core::types::{RecordingId, RecordingPhase, SystemAudio};
    let r = rig_full(false, None, PartFormat::Opus);
    let cap: &dyn RecordingCapture = &*r.session;
    assert_eq!(cap.state().state, RecordingPhase::Idle);
    assert_eq!(cap.state().system_audio, SystemAudio::Off);
    let id = RecordingId::try_from("rec-abc".to_owned()).unwrap();
    let dir = r.root.join("rec-abc");
    let st = cap.start(&id, &dir, Duration::from_secs(90 * 60)).unwrap();
    assert_eq!(st.state, RecordingPhase::Recording);
    assert_eq!(st.id, Some(id.clone()));
    assert_eq!(st.started_at_ms, Some(1_763_544_600_000));
    assert_eq!(st.ends_at_ms, Some(1_763_544_600_000 + 90 * 60_000));
    assert_eq!(st.system_audio, SystemAudio::Ok);
    let name = st.name.unwrap();
    assert_eq!(name.len(), "2026-11-19_0930".len(), "{name}");
    assert_eq!(
        cap.start(&id, &dir, Duration::from_secs(600))
            .unwrap_err()
            .ipc_code(),
        "recording_busy"
    );
    play(&r, 1_000, 0.1, 0.1);
    let parts = cap.stop().unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].index, 1);
    assert_eq!(parts[0].path, dir.join(format!("{name}.opus")));
    assert_eq!(parts[0].offset_s, 0.0);
    assert!(parts[0].duration_s > 0.9 && parts[0].duration_s <= 1.0);
    assert_eq!(cap.state().state, RecordingPhase::Idle);
    // A second stop is a no-op; the hand-off got the recording once.
    assert!(cap.stop().unwrap().is_empty());
    assert_eq!(r.handoff.0.lock().unwrap().len(), 1);
}

#[test]
fn adr_0103_knob_off_maps_to_recording_off() {
    use mog_core::recording::RecordingCapture;
    let r = rig();
    r.enabled.store(false, Ordering::SeqCst);
    let id = mog_core::types::RecordingId::try_from("rec-1".to_owned()).unwrap();
    let e = (&*r.session as &dyn RecordingCapture)
        .start(&id, &r.root.join("x"), Duration::from_secs(600))
        .unwrap_err();
    assert_eq!(e.ipc_code(), "recording_off");
}

/// A fake capture for the component routing.
#[derive(Default)]
struct FakeCapture {
    calls: Mutex<Vec<String>>,
}
impl mog_core::recording::RecordingCapture for FakeCapture {
    fn start(
        &self,
        id: &mog_core::types::RecordingId,
        dir: &Path,
        max_len: Duration,
    ) -> mog_core::recording::RecordingResult<mog_core::types::RecordingState> {
        assert!(dir.ends_with(id.as_str()));
        self.calls
            .lock()
            .unwrap()
            .push(format!("start {}", max_len.as_secs()));
        Ok(mog_core::types::RecordingState::idle())
    }
    fn stop(&self) -> mog_core::recording::RecordingResult<Vec<mog_core::recording::FinishedPart>> {
        self.calls.lock().unwrap().push("stop".into());
        Ok(Vec::new())
    }
    fn state(&self) -> mog_core::types::RecordingState {
        mog_core::types::RecordingState::idle()
    }
}

#[test]
fn adr_0103_the_component_routes_start_stop_state_and_the_knob() {
    use mog_core::ipc_gateway::GatewayComponents;
    use mog_core::recording::{CaptureLink, MemorySettingsStore, RecordingComponent};
    use mog_core::types::RecordingSettings;
    use serde_json::json;
    let store = Arc::new(MemorySettingsStore::new(RecordingSettings {
        enabled: true,
        consent_ack: true,
        ..RecordingSettings::default()
    }));
    let c = RecordingComponent::new(store);
    let fake = Arc::new(FakeCapture::default());
    let knob = Arc::new(Mutex::new(Vec::new()));
    let k2 = Arc::clone(&knob);
    let ctx = Arc::new(Mutex::new(Vec::new()));
    c.connect_capture(CaptureLink {
        capture: fake.clone(),
        temp_root: std::env::temp_dir().join("bottle-rec-component"),
        on_knob: Arc::new(move |on| k2.lock().unwrap().push(on)),
        on_start: {
            let ctx = Arc::clone(&ctx);
            Arc::new(move |id, profile, instructions| {
                ctx.lock().unwrap().push((
                    id.as_str().to_owned(),
                    profile.clone(),
                    instructions.map(|i| i.as_str().to_owned()),
                ))
            })
        },
    });
    // The stored knob reaches the host at the connect.
    assert_eq!(*knob.lock().unwrap(), vec![true]);
    // `profile` is required on the wire (ADR 0103 contract).
    assert!(c.call("recording_start", json!({})).is_err());
    let st = c
        .call(
            "recording_start",
            json!({"profile": "home", "instructions": "Only action items."}),
        )
        .unwrap();
    assert_eq!(st["state"], json!("idle"));
    {
        let ctx = ctx.lock().unwrap();
        assert_eq!(ctx.len(), 1);
        assert!(ctx[0].0.starts_with("rec-"));
        assert_eq!(ctx[0].1, "home");
        assert_eq!(ctx[0].2.as_deref(), Some("Only action items."));
    }
    c.call("recording_stop", json!({})).unwrap();
    c.call("recording_state", json!({})).unwrap();
    // The default length preset is 60 min.
    assert_eq!(*fake.calls.lock().unwrap(), vec!["start 3600", "stop"]);
    // A settings write that turns the knob off reaches the host and closes the gate.
    c.call(
        "recording_settings_write",
        json!({"patch": {"enabled": false}}),
    )
    .unwrap();
    assert_eq!(*knob.lock().unwrap(), vec![true, false]);
    let e = c
        .call("recording_start", json!({"profile": "home"}))
        .unwrap_err();
    assert!(matches!(
        e,
        mog_core::error::CoreError::IpcRefused {
            code: "recording_off"
        }
    ));
    assert_eq!(
        fake.calls.lock().unwrap().len(),
        2,
        "no capture start with the knob off"
    );
}

// ---------- a muted mic does not end a recording ----------

#[test]
fn adr_0103_the_mic_zero_watch_holds_while_a_recording_runs() {
    use mog_core::audio_device::{MicConfig, MicDevice, MicFault};
    let host = FakeHost::default();
    let h = host.clone();
    let (mic, link) = MicDevice::spawn_with(
        move || h,
        Box::new(Collect::default()),
        MicConfig {
            zero_window: Duration::from_millis(60),
            ..MicConfig::default()
        },
    )
    .unwrap();
    let faults = Arc::new(Mutex::new(Vec::new()));
    let f2 = Arc::clone(&faults);
    link.connect_fault_fn(move |_, fault| f2.lock().unwrap().push(fault));
    let hold = link.silence_hold();
    hold.set(true);
    mic.open(AudioSource::Mic).unwrap();
    // A muted mic: only digital silence, far past the zero window.
    for _ in 0..30 {
        host.play(&[0.0; 160], 1, 16_000);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        host.live.load(Ordering::SeqCst),
        1,
        "the mic stream stays open"
    );
    assert!(faults.lock().unwrap().is_empty(), "no zero-buffer fault");
    // Without the hold (no recording), CAP-17b applies again.
    hold.set(false);
    for _ in 0..100 {
        if !faults.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*faults.lock().unwrap(), vec![MicFault::ZeroBuffers]);
    mic.close(AudioSource::Mic);
}

#[test]
fn adr_0103_the_session_holds_the_mic_zero_watch_while_it_records() {
    let r = rig();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = Arc::clone(&seen);
    r.session
        .set_active_hook(Arc::new(move |on| s2.lock().unwrap().push(on)));
    assert!(!r.session.is_recording());
    r.session.start(opts("n", 60)).unwrap();
    assert!(r.session.is_recording());
    r.session.stop(StopCause::User).unwrap();
    assert!(!r.session.is_recording());
    // A refused start releases the hold too.
    r.enabled.store(false, Ordering::SeqCst);
    let _ = r.session.start(opts("n", 60));
    assert_eq!(*seen.lock().unwrap(), vec![true, false]);
    // A zero-buffer stop of the lease would no longer be a mic loss: it never comes, and a
    // muted mic records as silence next to the system audio.
    r.enabled.store(true, Ordering::SeqCst);
    r.session.start(opts("m", 60)).unwrap();
    play(&r, 500, 0.0, 0.3);
    let done = r.session.stop(StopCause::User).unwrap();
    let (_, _, _, s) = decode(&done.parts[0].path);
    assert!((s[4_000] - (0.3 * 32_767.0) as i32).abs() < 50);
}

#[test]
fn adr_0103_the_job_context_goes_out_with_the_parts() {
    let r = rig();
    let info_id = "rec-ctx1";
    r.session
        .set_job(info_id, Some("home".into()), Some("Notes, please.".into()));
    r.session
        .start_in(opts("n", 60), Some(info_id.into()), None)
        .unwrap();
    let done = r.session.stop(StopCause::User).unwrap();
    assert_eq!(done.profile.as_deref(), Some("home"));
    assert_eq!(done.instructions.as_deref(), Some("Notes, please."));
    assert_eq!(
        r.handoff.0.lock().unwrap()[0].profile.as_deref(),
        Some("home")
    );
    // A recording with no context hands off `None`s.
    r.session.start(opts("m", 60)).unwrap();
    let done = r.session.stop(StopCause::User).unwrap();
    assert_eq!((done.profile, done.instructions), (None, None));
}

#[test]
fn adr_0103_a_start_during_a_stop_is_busy() {
    // Review note 1: a start inside the stop window must not be disarmed by the old stop.
    let r = rig();
    let session = Arc::clone(&r.session);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = Arc::clone(&seen);
    let weak = Arc::downgrade(&session);
    r.session.set_active_hook(Arc::new(move |on| {
        if !on {
            if let Some(s) = weak.upgrade() {
                // The old stop is still between its take and its release.
                s2.lock()
                    .unwrap()
                    .push((s.is_recording(), s.start(opts("x", 60)).err()));
            }
        }
    }));
    session.start(opts("n", 60)).unwrap();
    session.stop(StopCause::User).unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(true, Some(RecordingCaptureError::Busy))]
    );
    assert!(!session.is_recording());
    session.start(opts("y", 60)).unwrap();
}

/// Wait (no latency assert) until the writer thread has announced `n` log entries.
fn wait_log(r: &Rig, n: usize) -> Vec<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let log = r.handoff.1.lock().unwrap().clone();
        if log.len() >= n || std::time::Instant::now() > deadline {
            return log;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn rec_parts_rotation_follows_the_audio_clock() {
    // The part length counts 16 kHz samples of the mixed stream, and the simulated clock moves
    // with the audio, so the cut is exact and no real time passes. Scale: 10 ms stands in for
    // one minute (as in the 90 minute test), so the 5 minute cap is 50 ms = 800 samples.
    let r = rig_full(false, Some(5 * 160), PartFormat::Flac);
    r.session.start(opts("2026-11-20_0900", 60)).unwrap();
    assert_eq!(
        r.handoff.1.lock().unwrap().as_slice(),
        ["started:2026-11-20_0900"]
    );
    // Just under "5 minutes" (40 ms of 50 ms): no part is announced yet.
    play(&r, 40, 0.1, 0.0);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        r.handoff.1.lock().unwrap().len(),
        1,
        "no part before the cap"
    );
    play(&r, 20, 0.1, 0.0);
    let log = wait_log(&r, 2);
    assert_eq!(log[1], "part:2026-11-20_0900.part01.flac");
    play(&r, 50, 0.1, 0.0);
    let log = wait_log(&r, 3);
    assert_eq!(log[2], "part:2026-11-20_0900.part02.flac");
    play(&r, 10, 0.1, 0.0);
    let done = r.session.stop(StopCause::User).unwrap();
    // Three parts at the stop; the last one is not announced, `finished` carries it.
    assert_eq!(done.parts.len(), 3);
    assert_eq!(done.parts[0].samples, 5 * 160);
    assert_eq!(done.parts[1].offset_ms, 50);
    assert_eq!(done.parts[2].offset_ms, 100);
    let log = r.handoff.1.lock().unwrap().clone();
    assert_eq!(log.last().unwrap(), "finished");
    assert_eq!(log.iter().filter(|l| l.starts_with("part:")).count(), 2);
}
