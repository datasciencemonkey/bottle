//! The recording tap: the frame sinks that feed an active recording (ADR 0103).
//!
//! The host tees the mic's frames to [`RecordingTap::mic_sink`] (next to the level stream and
//! the PTT buffer, ADR 0078) and gives [`RecordingTap::system_sink`] to the system audio
//! device. While no recording is armed, the sinks drop every frame, so no audio is kept
//! before a start or after a stop.
//!
//! The sinks run on real-time audio threads. They never block: they `try_lock` the arm slot
//! and `try_send` a copy of the block into a bounded queue. A full queue drops the block and
//! counts it ([`RecordingTap::dropped`]), so memory stays bounded if the writer falls behind.
//!
//! **Mic gate (ADR 0118).** While a talk key is down or a PTT overlay lease is live, and for
//! [`GATE_TAIL_MS`] after both end, the mic sink sends zeros of the same length to the mixer
//! (the timeline stays aligned) and marks the block `gated`. The PTT buffer and R4 are
//! siblings in the host's tee and still get the real frames. Only Rust calls the gate API.
//!
//! The system sink also stamps the last block with sound on the injected clock. The session
//! reads it for the CAP-10a 5 s zero-buffer check.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

use crate::audio_device::{FrameSink, MicFrame};
use crate::clock::MonotonicClock;

use super::mixer::Track;

/// How many blocks wait for the writer at most (about 2.5 s of 10 ms blocks per track).
pub const QUEUE_BLOCKS: usize = 512;

/// How long the mic gate stays closed after the talk key and the overlay lease have ended.
pub const GATE_TAIL_MS: u64 = 250;

/// One copied block of mono audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The source track.
    pub track: Track,
    /// The block's sample rate in Hz.
    pub rate: u32,
    /// Mono samples. All zero when `gated`.
    pub samples: Vec<f32>,
    /// The mic gate was closed: the real samples never left the sink (ADR 0118).
    pub gated: bool,
}

struct Shared {
    tx: Mutex<Option<SyncSender<Block>>>,
    clock: Arc<dyn MonotonicClock>,
    queue_blocks: usize,
    last_system_sound_ms: AtomicU64,
    dropped: AtomicU64,
    /// The talk key is physically down.
    key_down: AtomicBool,
    /// A PTT overlay lease on the recording is live.
    lease_live: AtomicBool,
    /// The gate stays closed until this clock time (ms): the 250 ms tail.
    closed_until_ms: AtomicU64,
}

/// The arm slot and the sinks. Clones share one slot.
#[derive(Clone)]
pub struct RecordingTap {
    shared: Arc<Shared>,
}

impl RecordingTap {
    /// A disarmed tap whose silence stamps use `clock`.
    pub fn new(clock: Arc<dyn MonotonicClock>) -> Self {
        Self::with_queue_blocks(clock, QUEUE_BLOCKS)
    }

    /// As [`RecordingTap::new`], with a queue of `queue_blocks` blocks. Tests that feed audio
    /// faster than real time use a long queue, so the writer thread never makes them drop.
    pub fn with_queue_blocks(clock: Arc<dyn MonotonicClock>, queue_blocks: usize) -> Self {
        Self {
            shared: Arc::new(Shared {
                tx: Mutex::new(None),
                clock,
                queue_blocks,
                last_system_sound_ms: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                key_down: AtomicBool::new(false),
                lease_live: AtomicBool::new(false),
                closed_until_ms: AtomicU64::new(0),
            }),
        }
    }

    /// The sink for the mic's frames.
    pub fn mic_sink(&self) -> TapSink {
        TapSink {
            shared: Arc::clone(&self.shared),
            track: Track::Mic,
        }
    }

    /// The sink for the system audio frames.
    pub fn system_sink(&self) -> TapSink {
        TapSink {
            shared: Arc::clone(&self.shared),
            track: Track::System,
        }
    }

    /// Arm the tap for a new recording: from now on frames go to the returned receiver. The
    /// silence stamp starts at now. An earlier arm is replaced (its receiver ends).
    pub fn arm(&self) -> Receiver<Block> {
        let (tx, rx) = mpsc::sync_channel(self.shared.queue_blocks);
        self.shared
            .last_system_sound_ms
            .store(now_ms(&*self.shared.clock), Ordering::SeqCst);
        self.shared.dropped.store(0, Ordering::SeqCst);
        // Only the tail resets. The flags are live state: an overlay lease that was granted
        // between the session lease and this arm, or a key that is down now, must keep the
        // gate closed (a stale `true` only silences the mic; a stale `false` would leak).
        // `lease_live` clears on that lease's stop. The host syncs the key with `resync_key`.
        self.shared.closed_until_ms.store(0, Ordering::SeqCst);
        *self
            .shared
            .tx
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(tx);
        rx
    }

    /// Disarm: no frame goes anywhere after this returns, and the receiver ends once it has
    /// read what is queued.
    pub fn disarm(&self) {
        self.shared
            .tx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    /// The talk key went down (ADR 0118). The host calls it at key-down, before the brain hears
    /// the press, so no first syllable reaches the recording. It never blocks.
    pub fn talk_key_down(&self) {
        self.shared.key_down.store(true, Ordering::SeqCst);
    }

    /// The talk key came up. The gate stays closed for [`GATE_TAIL_MS`] more, and longer while
    /// an overlay lease is live.
    pub fn talk_key_up(&self) {
        self.end_hold(&self.shared.key_down);
    }

    /// Set the physical state of the talk key from the host, for example at `arm` time or
    /// after an event tap was disabled and enabled again. `false` acts as [`Self::talk_key_up`]
    /// (with the tail), `true` as [`Self::talk_key_down`].
    pub fn resync_key(&self, down: bool) {
        if down {
            self.talk_key_down();
        } else {
            self.talk_key_up();
        }
    }

    /// A PTT overlay lease on the recording went live (`true`) or ended (`false`). The gate
    /// stays closed [`GATE_TAIL_MS`] after the end, unless the key is still down.
    pub fn overlay_lease(&self, live: bool) {
        if live {
            self.shared.lease_live.store(true, Ordering::SeqCst);
        } else {
            self.end_hold(&self.shared.lease_live);
        }
    }

    /// The tail is set before the flag clears, so no frame can see both open (fail closed).
    fn end_hold(&self, flag: &AtomicBool) {
        let until = now_ms(&*self.shared.clock).saturating_add(GATE_TAIL_MS);
        self.shared
            .closed_until_ms
            .fetch_max(until, Ordering::SeqCst);
        flag.store(false, Ordering::SeqCst);
    }

    /// The mic gate is closed now: the recording gets zeros for the mic.
    pub fn mic_gated(&self) -> bool {
        self.shared.gated()
    }

    /// The clock time (ms) of the last system audio block with sound, or of the arm.
    pub fn last_system_sound_ms(&self) -> u64 {
        self.shared.last_system_sound_ms.load(Ordering::SeqCst)
    }

    /// Blocks dropped since the arm because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::SeqCst)
    }
}

impl Shared {
    fn gated(&self) -> bool {
        self.key_down.load(Ordering::SeqCst)
            || self.lease_live.load(Ordering::SeqCst)
            || now_ms(&*self.clock) < self.closed_until_ms.load(Ordering::SeqCst)
    }
}

fn now_ms(clock: &dyn MonotonicClock) -> u64 {
    u64::try_from(clock.now().as_millis()).unwrap_or(u64::MAX)
}

/// A [`FrameSink`] of one track. See [`RecordingTap`].
pub struct TapSink {
    shared: Arc<Shared>,
    track: Track,
}

impl FrameSink for TapSink {
    fn frame(&mut self, frame: MicFrame<'_>) {
        let guard = match self.shared.tx.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            // An arm or disarm holds the slot for a moment: drop this block.
            Err(TryLockError::WouldBlock) => return,
        };
        let Some(tx) = guard.as_ref() else {
            return;
        };
        if self.track == Track::System && !frame.is_digital_silence() {
            self.shared
                .last_system_sound_ms
                .fetch_max(now_ms(&*self.shared.clock), Ordering::SeqCst);
        }
        // The gate is read on the mic track only. A closed gate sends zeros, never the samples.
        let gated = self.track == Track::Mic && self.shared.gated();
        let samples = if gated {
            vec![0.0; frame.samples.len()]
        } else {
            frame.samples.to_vec()
        };
        let block = Block {
            track: self.track,
            rate: frame.sample_rate,
            samples,
            gated,
        };
        match tx.try_send(block) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// How many ended overlay leases the gate wrapper remembers, so a late `overlay_granted` of a
/// lease that has stopped (the events come after the locks are released) cannot close the gate
/// for good.
const ENDED_MEMORY: usize = 16;

#[derive(Default)]
struct OverlayState {
    live: Option<crate::capture::LeaseNo>,
    ended: std::collections::VecDeque<crate::capture::LeaseNo>,
}

/// Wraps [`CaptureEvents`] and drives the mic gate from the overlay lease (ADR 0118): the
/// internal `overlay_granted` hook closes it, and the stop of that lease starts the tail.
/// The key-down side is the host's [`RecordingTap::talk_key_down`] call. The event goes on to
/// the inner sink. Put it outermost in the host chain, so the gate closes before any other
/// sink hears of the lease.
pub struct GateCaptureEvents<E> {
    inner: E,
    tap: RecordingTap,
    overlay: Mutex<OverlayState>,
}

impl<E: crate::capture::CaptureEvents> GateCaptureEvents<E> {
    /// Wrap `inner`.
    pub fn new(inner: E, tap: RecordingTap) -> Self {
        Self {
            inner,
            tap,
            overlay: Mutex::new(OverlayState::default()),
        }
    }

    /// The wrapped sink.
    pub fn inner(&self) -> &E {
        &self.inner
    }
}

impl<E: crate::capture::CaptureEvents> crate::capture::CaptureEvents for GateCaptureEvents<E> {
    fn state(&self, state: &crate::types::CaptureState) {
        self.inner.state(state);
    }

    fn stopped(&self, stopped: &crate::capture::Stopped) {
        {
            let mut o = self.overlay.lock().unwrap_or_else(PoisonError::into_inner);
            if !o.ended.contains(&stopped.lease_id) {
                if o.ended.len() >= ENDED_MEMORY {
                    o.ended.pop_front();
                }
                o.ended.push_back(stopped.lease_id);
            }
            if o.live == Some(stopped.lease_id) {
                o.live = None;
                self.tap.overlay_lease(false);
            }
        }
        self.inner.stopped(stopped);
    }

    fn overlay_granted(&self, lease: &crate::capture::Lease) {
        {
            let mut o = self.overlay.lock().unwrap_or_else(PoisonError::into_inner);
            // A grant that arrives after the stop of its lease must not close the gate.
            if !o.ended.contains(&lease.id) {
                o.live = Some(lease.id);
                self.tap.overlay_lease(true);
            }
        }
        self.inner.overlay_granted(lease);
    }
}
