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
//! The system sink also stamps the last block with sound on the injected clock. The session
//! reads it for the CAP-10a 5 s zero-buffer check.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

use crate::audio_device::{FrameSink, MicFrame};
use crate::clock::MonotonicClock;

use super::mixer::Track;

/// How many blocks wait for the writer at most (about 2.5 s of 10 ms blocks per track).
pub const QUEUE_BLOCKS: usize = 512;

/// One copied block of mono audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The source track.
    pub track: Track,
    /// The block's sample rate in Hz.
    pub rate: u32,
    /// Mono samples.
    pub samples: Vec<f32>,
}

struct Shared {
    tx: Mutex<Option<SyncSender<Block>>>,
    clock: Arc<dyn MonotonicClock>,
    last_system_sound_ms: AtomicU64,
    dropped: AtomicU64,
}

/// The arm slot and the sinks. Clones share one slot.
#[derive(Clone)]
pub struct RecordingTap {
    shared: Arc<Shared>,
}

impl RecordingTap {
    /// A disarmed tap whose silence stamps use `clock`.
    pub fn new(clock: Arc<dyn MonotonicClock>) -> Self {
        Self {
            shared: Arc::new(Shared {
                tx: Mutex::new(None),
                clock,
                last_system_sound_ms: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
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
        let (tx, rx) = mpsc::sync_channel(QUEUE_BLOCKS);
        self.shared
            .last_system_sound_ms
            .store(now_ms(&*self.shared.clock), Ordering::SeqCst);
        self.shared.dropped.store(0, Ordering::SeqCst);
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

    /// The clock time (ms) of the last system audio block with sound, or of the arm.
    pub fn last_system_sound_ms(&self) -> u64 {
        self.shared.last_system_sound_ms.load(Ordering::SeqCst)
    }

    /// Blocks dropped since the arm because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::SeqCst)
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
        let block = Block {
            track: self.track,
            rate: frame.sample_rate,
            samples: frame.samples.to_vec(),
        };
        match tx.try_send(block) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
