//! The one shared per-profile rate bucket of the app (#467, #138, ADR 0075; G-09a, ARC-F3).
//!
//! `rate_bucket.rs:4` says "R7 and R8 share one per process". `lib.rs` builds that one
//! [`mog_core::http_transport::rate_bucket::AimdBucket`] and manages it as [`SharedRateBucket`],
//! before `setup` runs any `*_host::install`. Every R7 or R8 in the app takes a clone of this Arc
//! from app state, so the AIMD cut and the `Retry-After` hold of a profile are shared. Never build
//! a second bucket in the app (#511: `model_invoke` and `dbx_http` must take this one too).

use std::sync::Arc;

use mog_core::http_transport::rate_bucket::RateBucket;

/// The app-managed shared bucket. Clone the inner Arc; never build a new bucket.
#[derive(Clone)]
pub struct SharedRateBucket(pub Arc<dyn RateBucket>);

impl std::fmt::Debug for SharedRateBucket {
    /// Shows nothing: no profile names.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedRateBucket").finish_non_exhaustive()
    }
}
