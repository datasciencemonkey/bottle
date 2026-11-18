//! CAP-34 and section 9.2 (#21, ADR 0078): the PTT buffer wipes its audio before the memory
//! is freed.
//!
//! This test binary has its own global allocator. It wraps the system allocator, and while a
//! test arms it on its own thread, it reads each freed block of the armed size before it
//! frees it. A block that holds any non-zero byte is "dirty". So a removed wipe in
//! `Chunk::drop` or `Utterance::drop` makes these tests fail. The allocator itself never
//! allocates, and it reads only blocks that are still allocated.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::{Arc, Mutex};

use mog_core::audio_device::ptt_buffer::{PttBuffer, Utterance, UtteranceSink, CHUNK_SAMPLES};
use mog_core::audio_device::{FrameSink, MicFrame};
use mog_core::capture::{StopReason, Stopped};
use mog_core::types::{CaptureMode, CaptureState, OthersTrack};

thread_local! {
    /// The block size in bytes to check on this thread; 0 is off.
    static WATCH: Cell<usize> = const { Cell::new(0) };
    static CHECKED: Cell<usize> = const { Cell::new(0) };
    static DIRTY: Cell<usize> = const { Cell::new(0) };
}

struct WipeProbe;

unsafe impl GlobalAlloc for WipeProbe {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller keeps the `GlobalAlloc` contract; we forward it.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let watch = WATCH.try_with(Cell::get).unwrap_or(0);
        if watch != 0 && layout.size() == watch {
            // SAFETY: `ptr` is a live block of `layout.size()` bytes until the call below.
            let bytes = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
            let dirty = bytes.iter().any(|b| *b != 0);
            let _ = CHECKED.try_with(|c| c.set(c.get() + 1));
            if dirty {
                let _ = DIRTY.try_with(|c| c.set(c.get() + 1));
            }
        }
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static PROBE: WipeProbe = WipeProbe;

/// Check the freed blocks of `bytes` bytes on this thread while `f` runs. Returns (checked,
/// dirty).
fn watch(bytes: usize, f: impl FnOnce()) -> (usize, usize) {
    CHECKED.with(|c| c.set(0));
    DIRTY.with(|c| c.set(0));
    WATCH.with(|w| w.set(bytes));
    f();
    WATCH.with(|w| w.set(0));
    (CHECKED.with(Cell::get), DIRTY.with(Cell::get))
}

const MOG_A: &str = "00000000-0000-4000-8000-0000000000a1";
const RATE: u32 = 48_000;
/// Not a multiple of the chunk size, so the utterance block has its own size.
const UTTERANCE_SAMPLES: usize = 12_345;

#[derive(Default)]
struct Kept(Mutex<Vec<Utterance>>);

impl UtteranceSink for Kept {
    fn utterance(&self, utterance: Utterance) {
        self.0.lock().unwrap().push(utterance);
    }
}

fn recording() -> CaptureState {
    CaptureState {
        owner: Some(MOG_A.into()),
        mode: Some(CaptureMode::Ptt),
        lease_id: Some("1".into()),
        recording: true,
        paused: false,
        session_id: None,
        started_at: Some(1),
        directed_to: None,
        others_track: OthersTrack::Off,
        mic: None,
    }
}

fn stop(reason: StopReason) -> Stopped {
    Stopped {
        lease_id: 1,
        mog_id: MOG_A.into(),
        mode: CaptureMode::Ptt,
        reason,
    }
}

/// Loud audio with no zero sample.
fn record(buf: &PttBuffer, samples: usize) {
    let audio = vec![0.5f32; samples];
    let mut sink = buf.sink();
    buf.on_state(&recording());
    for block in audio.chunks(480) {
        sink.frame(MicFrame {
            samples: block,
            sample_rate: RATE,
        });
    }
}

const CHUNK_BYTES: usize = CHUNK_SAMPLES * std::mem::size_of::<f32>();

#[test]
fn cap_34_discarded_take_chunks_are_wiped_before_free() {
    let kept = Arc::new(Kept::default());
    let buf = PttBuffer::new(kept.clone());
    let (checked, dirty) = watch(CHUNK_BYTES, || {
        record(&buf, CHUNK_SAMPLES * 3);
        buf.on_stopped(&stop(StopReason::Discarded));
    });
    assert!(
        checked >= 3,
        "the 3 chunks were freed and checked: {checked}"
    );
    assert_eq!(dirty, 0, "a chunk was freed with audio in it");
}

#[test]
fn cap_34_kept_take_chunks_are_wiped_after_the_copy() {
    let kept = Arc::new(Kept::default());
    let buf = PttBuffer::new(kept.clone());
    let (checked, dirty) = watch(CHUNK_BYTES, || {
        record(&buf, UTTERANCE_SAMPLES);
        buf.on_stopped(&stop(StopReason::Released));
    });
    assert!(
        checked >= 4,
        "the 4 chunks were freed and checked: {checked}"
    );
    assert_eq!(dirty, 0, "a chunk was freed with audio in it");
    assert_eq!(kept.0.lock().unwrap().len(), 1);
}

#[test]
fn cap_34_a_dropped_utterance_is_wiped_before_free() {
    let kept = Arc::new(Kept::default());
    let buf = PttBuffer::new(kept.clone());
    record(&buf, UTTERANCE_SAMPLES);
    buf.on_stopped(&stop(StopReason::Released));
    let utterance = kept.0.lock().unwrap().pop().expect("one utterance");
    assert_eq!(utterance.samples().len(), UTTERANCE_SAMPLES);
    assert!(utterance.samples().iter().all(|s| *s != 0.0));
    let bytes = UTTERANCE_SAMPLES * std::mem::size_of::<f32>();
    let (checked, dirty) = watch(bytes, || drop(utterance));
    assert_eq!(checked, 1, "the utterance block was freed and checked");
    assert_eq!(dirty, 0, "the utterance was freed with audio in it");
}
