//! Live checks of the call recording capture on a real Mac (ADR 0103). Ignored by default:
//! they open the real system audio tap and play speech with `say`.
//!
//! Run by hand on macOS 14.4+:
//!
//! ```text
//! cargo test -p mog-core --test recording_capture_live -- --ignored --nocapture
//! ```
//!
//! The first run may show the macOS "System Audio Recording" prompt for the terminal app.
//! If it is denied, the tap gives digital silence and the test fails with "silent": that is
//! the CAP-10a case, not a test bug.

#![cfg(target_os = "macos")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::audio_device::system::{SystemAudioAvailability, SystemAudioDevice};
use mog_core::audio_device::system_macos::TapHost;
use mog_core::audio_device::{FrameSink, MicFrame};
use mog_core::capture::{AudioDevice, AudioSource};
use mog_core::recording_capture::chunker::{PartFormat, PartWriter};
use mog_core::recording_capture::mixer::{Mixer, Track};

#[derive(Clone, Default)]
struct Collect(
    Arc<Mutex<(Vec<f32>, u32)>>,
    Arc<Mutex<Vec<(std::time::Instant, usize)>>>,
);

impl FrameSink for Collect {
    fn frame(&mut self, frame: MicFrame<'_>) {
        let mut g = self.0.lock().unwrap();
        g.0.extend_from_slice(frame.samples);
        g.1 = frame.sample_rate;
        self.1
            .lock()
            .unwrap()
            .push((std::time::Instant::now(), frame.samples.len()));
    }
}

#[test]
#[ignore = "opens the real system audio tap; run by hand on a Mac"]
fn adr_0103_live_tap_records_say_into_a_non_silent_flac() {
    let got = Collect::default();
    let (dev, status) = SystemAudioDevice::spawn(
        TapHost::new,
        Box::new(got.clone()),
        Duration::from_secs(10),
        Duration::from_secs(10),
    )
    .unwrap();
    let t0 = std::time::Instant::now();
    dev.open(AudioSource::SystemAudio).unwrap();
    eprintln!("open took {:?}", t0.elapsed());
    assert_eq!(
        status.get(),
        SystemAudioAvailability::Live,
        "tap did not open"
    );
    let mut say = std::process::Command::new("say")
        .arg("Bottle records the other side of the call. One two three four five.")
        .spawn()
        .expect("say");
    std::thread::sleep(Duration::from_secs(5));
    dev.close(AudioSource::SystemAudio);
    let _ = say.wait();

    let (samples, rate) = got.0.lock().unwrap().clone();
    {
        let cb = got.1.lock().unwrap();
        if let (Some(a), Some(b)) = (cb.first(), cb.last()) {
            eprintln!(
                "{} callbacks, first after {:?}, span {:?}, first len {}",
                cb.len(),
                a.0 - t0,
                b.0 - a.0,
                a.1
            );
        }
    }
    eprintln!("tap rate {rate} Hz, {} mono samples", samples.len());
    assert!(rate > 0 && !samples.is_empty(), "no frames from the tap");

    let mut mixer = Mixer::new(4_000);
    mixer.push(Track::System, &samples, rate);
    let mut out = Vec::new();
    mixer.flush(&mut out);
    let dir = std::env::temp_dir().join(format!("bottle-live-tap-{}", std::process::id()));
    let mut parts = PartWriter::new(&dir, "live", 55 * 60 * 16_000).unwrap();
    parts.write(&out).unwrap();
    let parts = parts.finish().unwrap();
    eprintln!("wrote {}", parts[0].path.display());
    // The reference decoder, when it is installed (Homebrew `flac`).
    if let Ok(o) = std::process::Command::new("flac")
        .args(["-t", "-s"])
        .arg(&parts[0].path)
        .output()
    {
        eprintln!(
            "flac -t: {}",
            if o.status.success() { "ok" } else { "FAILED" }
        );
        assert!(
            o.status.success(),
            "flac -t: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    let mut reader = claxon::FlacReader::open(&parts[0].path).unwrap();
    let decoded: Vec<i32> = reader.samples().map(|s| s.unwrap()).collect();
    let loud = decoded.iter().filter(|s| s.abs() > 300).count();
    let peak = decoded.iter().map(|s| s.abs()).max().unwrap_or(0);
    eprintln!(
        "decoded {} samples at 16 kHz, {loud} above -40 dBFS, peak {peak}",
        decoded.len()
    );
    // The tap's IO starts about 1 s after the open (tap auto start), so 5 s give 3.5-4 s.
    assert!(decoded.len() >= 16_000 * 3, "too short");
    assert!(
        loud > 1_600,
        "silent: System Audio Recording denied or no output"
    );

    // The decided codec: the same audio as one Ogg Opus file, checked by ffmpeg if installed.
    let mut opus = PartWriter::with_format(&dir, "live", u64::MAX, PartFormat::Opus).unwrap();
    opus.write(&out).unwrap();
    let opus = opus.finish().unwrap();
    let size = std::fs::metadata(&opus[0].path).unwrap().len();
    eprintln!("wrote {} ({size} bytes)", opus[0].path.display());
    if let Ok(o) = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&opus[0].path)
        .args(["-f", "null", "-"])
        .output()
    {
        let ok = o.status.success() && o.stderr.is_empty();
        eprintln!("ffmpeg decode: {}", if ok { "ok" } else { "FAILED" });
        assert!(ok, "ffmpeg: {}", String::from_utf8_lossy(&o.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
