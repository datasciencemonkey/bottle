//! R4 SttEngine (section 4.3).
//!
//! Responsibilities: Parakeet TDT 0.6B v3 int8 on the official `sherpa-onnx` crate, CPU EP, one worker thread. Packs audio for cloud ASR.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R4";

/// Local speech to text. It gets segment IDs from R2, not raw audio from callers.
pub trait SttEngine: Send + Sync {
    /// Load the local model into memory.
    fn load_model(&self) -> CoreResult<()>;
    /// Transcribe one segment that R2 made. Returns the text.
    fn transcribe(&self, segment_id: &str) -> CoreResult<String>;
}

/// The R4 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubSttEngine;

impl SttEngine for StubSttEngine {
    fn load_model(&self) -> CoreResult<()> {
        // TODO(#46): model residency and preload on key-down.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn transcribe(&self, _segment_id: &str) -> CoreResult<String> {
        // TODO(#46): streaming VAD decode on one worker thread.
        Err(CoreError::NotImplemented { component: ID })
    }
}
