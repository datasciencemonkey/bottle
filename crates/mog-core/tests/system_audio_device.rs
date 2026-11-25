//! `SystemAudioDevice` open/adopt/drop races (CAP-10c, #593 follow-up): a build whose `play`
//! blocks past `open`'s own timeout must end up either adopted (tracked, `status` live) or
//! reliably dropped — never a live stream that `status` does not know about. A fake `MicHost`
//! stands in for the real Core Audio tap; no test opens real system audio.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::audio_device::system::{SystemAudioAvailability, SystemAudioDevice};
use mog_core::audio_device::{DataFn, FaultFn, FrameSink, Interleaved, MicFrame, MicHost};
use mog_core::capture::{AudioDevice, AudioSource, DeviceError};

const WAIT: Duration = Duration::from_secs(5);

/// A frame sink that only counts what it got (no audio is kept).
#[derive(Clone, Default)]
struct CountSink(Arc<AtomicUsize>);
impl FrameSink for CountSink {
    fn frame(&mut self, _frame: MicFrame<'_>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeStream {
    on_data: Mutex<DataFn>,
    dropped: Arc<AtomicUsize>,
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

/// A host whose `play` takes `play_delay` before it succeeds (simulating a slow
/// `AudioDeviceStart`, review of #593's evidence of an ~8.6 s start). `build` is instant.
#[derive(Clone)]
struct SlowPlayHost {
    play_delay: Duration,
    builds: Arc<AtomicUsize>,
    plays: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

impl SlowPlayHost {
    fn new(play_delay: Duration) -> Self {
        Self {
            play_delay,
            builds: Arc::new(AtomicUsize::new(0)),
            plays: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl MicHost for SlowPlayHost {
    type Stream = FakeStream;

    fn build(&mut self, on_data: DataFn, _on_fault: FaultFn) -> Result<FakeStream, DeviceError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        Ok(FakeStream {
            on_data: Mutex::new(on_data),
            dropped: Arc::clone(&self.dropped),
        })
    }

    fn play(&mut self, stream: &FakeStream) -> Result<(), DeviceError> {
        self.plays.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.play_delay);
        // The real device starts delivering frames only once it is really running.
        let samples = [0.1_f32, -0.1_f32];
        (stream.on_data.lock().unwrap())(Interleaved {
            samples: &samples,
            channels: 1,
            sample_rate: 16_000,
        });
        Ok(())
    }
}

/// Poll `f` until it is true or `WAIT` runs out.
fn wait_for(mut f: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !f() {
        assert!(
            start.elapsed() < WAIT,
            "timed out waiting for the condition"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn rec_a_late_build_that_succeeds_is_adopted_not_left_live_and_untracked() {
    let host = SlowPlayHost::new(Duration::from_millis(150));
    let got = Arc::new(AtomicUsize::new(0));
    let sink = CountSink(Arc::clone(&got));
    let h = host.clone();
    // A short open timeout on both: `play` (150 ms) outlives it (30 ms), like the bug's ~8.6 s
    // `AudioDeviceStart` outliving the review's 8 s timeout.
    let (dev, status) =
        SystemAudioDevice::spawn(move || h, Box::new(sink), Duration::from_millis(30)).unwrap();

    dev.open(AudioSource::SystemAudio).unwrap();
    // The caller gives up at once: mic-only for now, never left unresolved.
    assert_eq!(status.get(), SystemAudioAvailability::Unavailable);

    // The late build+play succeeds; the worker adopts it and corrects `status` to `Live`,
    // and the diagnostics say it was a timed-out resolution.
    wait_for(|| status.get() == SystemAudioAvailability::Live);
    assert!(
        status.timed_out(),
        "the resolution took longer than its timeout"
    );
    assert!(
        status.open_ms() >= 150,
        "open_ms should cover the real build+play time, got {}",
        status.open_ms()
    );

    // The frame that arrived once the stream was genuinely live reached the sink: the tap is
    // tracked, not a live stream nobody knows about.
    wait_for(|| got.load(Ordering::SeqCst) > 0);

    dev.close(AudioSource::SystemAudio);
    assert_eq!(status.get(), SystemAudioAvailability::Off);
    assert_eq!(
        host.dropped.load(Ordering::SeqCst),
        1,
        "the stream was dropped exactly once"
    );
}

#[test]
fn rec_a_close_during_a_late_build_reliably_drops_it_and_never_adopts() {
    let host = SlowPlayHost::new(Duration::from_millis(150));
    let got = Arc::new(AtomicUsize::new(0));
    let sink = CountSink(Arc::clone(&got));
    let h = host.clone();
    let (dev, status) =
        SystemAudioDevice::spawn(move || h, Box::new(sink), Duration::from_millis(30)).unwrap();

    dev.open(AudioSource::SystemAudio).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Unavailable);
    // Close right away, well before the 150 ms play() resolves. `close` waits for the worker
    // (within its own CLOSE_WAIT), so by the time it returns the late build has already been
    // cancelled rather than adopted.
    dev.close(AudioSource::SystemAudio);

    assert_eq!(
        status.get(),
        SystemAudioAvailability::Off,
        "a close must win over a late build: never left on Live or a stale Unavailable"
    );
    wait_for(|| host.dropped.load(Ordering::SeqCst) == 1);
    // Give any stray late worker activity a moment, then check status did not flip again.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(status.get(), SystemAudioAvailability::Off);
}

#[test]
fn rec_a_close_that_gives_up_early_still_drops_the_late_build_and_stays_off() {
    // `close_wait` (30 ms) is well short of the build (300 ms): `close` must not block on it,
    // and the late build must still end up dropped with `status` left at `Off`.
    let host = SlowPlayHost::new(Duration::from_millis(300));
    let got = Arc::new(AtomicUsize::new(0));
    let sink = CountSink(Arc::clone(&got));
    let h = host.clone();
    let (dev, status) = SystemAudioDevice::spawn_with_close_wait(
        move || h,
        Box::new(sink),
        Duration::from_millis(30),
        Duration::from_millis(30),
    )
    .unwrap();

    dev.open(AudioSource::SystemAudio).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Unavailable);
    let before = std::time::Instant::now();
    dev.close(AudioSource::SystemAudio);
    assert!(
        before.elapsed() < Duration::from_millis(300),
        "close must give up at close_wait, not wait for the build"
    );
    assert_eq!(status.get(), SystemAudioAvailability::Off);

    // The build resolves later regardless; it must be dropped, and `status` must stay `Off`.
    wait_for(|| host.dropped.load(Ordering::SeqCst) == 1);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(status.get(), SystemAudioAvailability::Off);
}

#[test]
fn rec_open_close_open_while_the_first_play_is_still_blocked() {
    let host = SlowPlayHost::new(Duration::from_millis(300));
    let got = Arc::new(AtomicUsize::new(0));
    let sink = CountSink(Arc::clone(&got));
    let h = host.clone();
    let (dev, status) = SystemAudioDevice::spawn_with_close_wait(
        move || h,
        Box::new(sink),
        Duration::from_millis(30),
        Duration::from_millis(30),
    )
    .unwrap();

    // Generation 1: open, then close while its `play` is still blocked.
    dev.open(AudioSource::SystemAudio).unwrap();
    dev.close(AudioSource::SystemAudio);
    assert_eq!(status.get(), SystemAudioAvailability::Off);

    // Generation 2: open again right away, while generation 1's `play` is still running.
    dev.open(AudioSource::SystemAudio).unwrap();
    assert_eq!(status.get(), SystemAudioAvailability::Unavailable);

    // Generation 1's late build resolves and must be dropped, never adopted (`wanted` now
    // names generation 2).
    wait_for(|| host.dropped.load(Ordering::SeqCst) >= 1);
    assert_ne!(
        status.get(),
        SystemAudioAvailability::Live,
        "generation 1's late build must never be adopted once generation 2 is wanted"
    );

    // Generation 2's own build is still pending; only once it resolves may `status` go live.
    wait_for(|| status.get() == SystemAudioAvailability::Live);
    dev.close(AudioSource::SystemAudio);
}
