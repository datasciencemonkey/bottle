//! The cloud half of call recording (ADR 0103, PR C): Settings persistence, storage in a Unity
//! Catalog volume (My Files probe, an existing volume, or `bottle_audio` created after the user
//! confirms), upload with the Files API, the transcript and the notes from two model calls on
//! the Unity Gateway chat route (NFR-21a), the job manifest, list, detail, retry, and the
//! Catalog Explorer link with the OS opener.
//!
//! Everything runs on the device with the home-profile token (R6 `with_token`, inside R7). No
//! token, transcript, instructions or notes text is logged. No compute is started: no SQL
//! warehouse, no job, no statement (ADR 0103, UPDATE 4).

pub mod cloud;
pub mod errors;
pub mod http;
pub mod job;
pub mod layout;
pub mod model;
pub mod opener;
pub mod settings_store;
pub mod transcript;
