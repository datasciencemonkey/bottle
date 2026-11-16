//! R11 WindowHost (section 4.3).
//!
//! Responsibilities: Hub, MOG windows, cards, tray, native recording indicator (fail-closed, never `contentProtected`), the native window of the consent dialog (the logic is T12), cap of 8 active MOGs, per-display positions, `insert_text` paste.
//!
//! `mog-core` has no Tauri dependency: the app crate (`src-tauri`) implements this trait.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R11";

/// Native windows. The Tauri shell (or a later shell) implements it; the core only calls it.
pub trait WindowHost: Send + Sync {
    /// Open the MOG window of `mog_id` (cap of 8 active MOGs).
    fn open_mog_window(&self, mog_id: &str) -> CoreResult<()>;
    /// Close the MOG window of `mog_id`.
    fn close_mog_window(&self, mog_id: &str) -> CoreResult<()>;
    /// Show the card window of `mog_id`, made on the first open.
    fn show_card(&self, mog_id: &str) -> CoreResult<()>;
}

/// The R11 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubWindowHost;

impl WindowHost for StubWindowHost {
    fn open_mog_window(&self, _mog_id: &str) -> CoreResult<()> {
        // TODO(#8): Tauri window model in src-tauri.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn close_mog_window(&self, _mog_id: &str) -> CoreResult<()> {
        // TODO(#8): Tauri window model in src-tauri.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn show_card(&self, _mog_id: &str) -> CoreResult<()> {
        // TODO(#78): card window, lazy create.
        Err(CoreError::NotImplemented { component: ID })
    }
}
