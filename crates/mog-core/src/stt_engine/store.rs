//! The `stt_transcribe` side of R4 (#46, ADR 0082): the finished transcripts wait here until
//! the brain asks for them by `segmentId`.
//!
//! The brain gets `capture.segment` after a capture, then calls `stt_transcribe` with its
//! `segmentId`. [`TranscriptStore`] keeps each result until that call, and hands it out once.
//! It keeps at most [`TranscriptStore::CAPACITY`] results; the oldest goes first. The text is
//! in RAM only. The host routes `stt_transcribe` here (#514). TODO(#514): the host emits
//! `capture.segment`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use super::{EngineParts, LocalSttEngine, RecognizerLoader, SttDone, TranscriptSink};
use crate::error::{CoreError, CoreResult};
use crate::stt::SttEngine;
use crate::types::Transcript;

const ID: &str = "R4";

/// The finished transcripts by `segmentId`. It also passes each result on to `next`.
pub struct TranscriptStore {
    items: Mutex<VecDeque<(String, Result<Transcript, &'static str>)>>,
    next: Arc<dyn TranscriptSink>,
}

impl std::fmt::Debug for TranscriptStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No text in a log.
        f.debug_struct("TranscriptStore")
            .field("items", &self.lock().len())
            .finish_non_exhaustive()
    }
}

impl TranscriptStore {
    /// The most results that wait at once.
    pub const CAPACITY: usize = 32;

    /// An empty store that passes each result on to `next`.
    pub fn new(next: Arc<dyn TranscriptSink>) -> Self {
        Self {
            items: Mutex::default(),
            next,
        }
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, VecDeque<(String, Result<Transcript, &'static str>)>> {
        self.items.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Take the result of `segment_id`. `None` if it is not here (unknown, or taken).
    pub fn take(&self, segment_id: &str) -> Option<Result<Transcript, &'static str>> {
        let mut items = self.lock();
        let at = items.iter().position(|(id, _)| id == segment_id)?;
        items.remove(at).map(|(_, r)| r)
    }
}

impl TranscriptSink for TranscriptStore {
    fn done(&self, done: SttDone) {
        {
            let mut items = self.lock();
            if items.len() >= Self::CAPACITY {
                items.pop_front();
            }
            items.push_back((super::segment_id(done.lease_id), done.result.clone()));
        }
        self.next.done(done);
    }
}

/// R4 as the [`SttEngine`] interface: the engine worker and the transcript store.
#[derive(Debug, Clone)]
pub struct LocalStt {
    engine: LocalSttEngine,
    store: Arc<TranscriptStore>,
}

impl LocalStt {
    /// Start R4 with `loader`. Each result goes to the store, then to `next` (the host, which
    /// emits `capture.segment`). `configure` can change the parts first (a test clock, ...).
    pub fn new(
        loader: Arc<dyn RecognizerLoader>,
        next: Arc<dyn TranscriptSink>,
        configure: impl FnOnce(&mut EngineParts),
    ) -> Self {
        let store = Arc::new(TranscriptStore::new(next));
        let mut parts = EngineParts::new(loader, Arc::clone(&store) as Arc<dyn TranscriptSink>);
        configure(&mut parts);
        Self {
            engine: LocalSttEngine::spawn(parts),
            store,
        }
    }

    /// The engine handle (preload, the mic sink, the R2 events).
    pub fn engine(&self) -> &LocalSttEngine {
        &self.engine
    }
}

impl SttEngine for LocalStt {
    fn load_model(&self) -> CoreResult<()> {
        self.engine.preload();
        self.engine.sync();
        let status = self.engine.status();
        if status.loaded {
            return Ok(());
        }
        Err(CoreError::Unavailable {
            component: ID,
            reason: status.last_error.unwrap_or("stt_model_missing"),
        })
    }

    fn transcribe(&self, segment_id: &str) -> CoreResult<Transcript> {
        match self.store.take(segment_id) {
            Some(Ok(t)) => Ok(t),
            Some(Err(reason)) => Err(CoreError::Unavailable {
                component: ID,
                reason,
            }),
            None => Err(CoreError::Unavailable {
                component: ID,
                reason: "stt_segment_unknown",
            }),
        }
    }
}
