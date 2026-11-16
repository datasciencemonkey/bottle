//! R11 WindowHost (section 4.3).
//!
//! Responsibilities: Hub, MOG windows, cards, tray, native recording indicator (fail-closed, never `contentProtected`), the native window of the consent dialog (the logic is T12), cap of 8 active MOGs, per-display positions, `insert_text` paste.
//!
//! `mog-core` has no Tauri dependency: the app crate (`src-tauri`) implements this trait.
//! The Tauri host routes the `window_*` commands of 4.5 to it (#285, ADR 0040). A refusal is
//! [`CoreError::IpcRefused`] with a code from `ipc_gateway::ERROR_CODES`.

use crate::error::{CoreError, CoreResult};
use crate::ipc::Geometry;

const ID: &str = "R11";

/// Native windows. The Tauri shell (or a later shell) implements it; the core only calls it.
pub trait WindowHost: Send + Sync {
    /// Open the MOG window of `mog_id` (cap of 8 active MOGs). `geometry` is the logical
    /// position and size; `None` keeps the default.
    fn open_mog_window(&self, mog_id: &str, geometry: Option<&Geometry>) -> CoreResult<()>;
    /// Close the MOG window of `mog_id`.
    fn close_mog_window(&self, mog_id: &str) -> CoreResult<()>;
    /// Show the card window of `mog_id`, made on the first open.
    fn show_card(&self, mog_id: &str) -> CoreResult<()>;
    /// Hide the card window of `mog_id`. The host destroys it later (MOG-18b).
    fn hide_card(&self, mog_id: &str) -> CoreResult<()>;
}

/// The R11 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubWindowHost;

impl WindowHost for StubWindowHost {
    fn open_mog_window(&self, _mog_id: &str, _geometry: Option<&Geometry>) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn close_mog_window(&self, _mog_id: &str) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn show_card(&self, _mog_id: &str) -> CoreResult<()> {
        // The Tauri shell implements R11 (`TauriWindowHost`); this stub stays for other hosts.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn hide_card(&self, _mog_id: &str) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }
}
