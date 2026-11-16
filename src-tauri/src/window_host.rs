//! R11 WindowHost: makes and closes the `mog-<id>` and `card-<id>` windows at run time (ARC-3).
//!
//! `WindowLedger` is the pure part: the cap of 8 active MOGs, label-safe IDs and one card per
//! open MOG. `TauriWindowHost` applies the ledger to real windows. Every label it makes matches
//! exactly one capability glob (`mog-*`, `card-*`), so the window gets only `view_emit`.
//!
//! TODO(#285): the gateway does not route `window_mog_open`, `window_mog_close`,
//! `window_card_show` and `window_card_hide` to this host yet (contract change).
//! TODO(#78): the hidden-card reaper (`CARD_HIDDEN_TTL`) and the key-capable panel.

use crate::windows::{self, WindowKind, MAX_ACTIVE_MOGS};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Mutex;

/// Why the host refused a window operation. No variant carries the caller's ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    /// The MOG ID is not label-safe (1-64 characters of `[A-Za-z0-9-]`).
    InvalidId,
    /// `cap` MOG windows are open already (ARC-3, MB-03b).
    CapReached { cap: usize },
    /// No MOG window with this ID is open.
    UnknownMog,
    /// The window system refused to make or change the window.
    HostFailed,
}

impl WindowError {
    /// The stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            WindowError::InvalidId => "invalid_id",
            WindowError::CapReached { .. } => "mog_cap_reached",
            WindowError::UnknownMog => "unknown_mog",
            WindowError::HostFailed => "host_failed",
        }
    }
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowError::InvalidId => write!(f, "the MOG ID is not a valid window label"),
            WindowError::CapReached { cap } => write!(f, "{cap} MOG windows are open already"),
            WindowError::UnknownMog => write!(f, "no MOG window with this ID is open"),
            WindowError::HostFailed => write!(f, "the window system refused the operation"),
        }
    }
}

impl std::error::Error for WindowError {}

/// The result of `open_mog`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// A new MOG window must be made with this kind.
    Opened(WindowKind),
    /// The MOG window is open already; nothing changes.
    AlreadyOpen,
}

/// Which MOG and card windows are open. `BTreeMap` value: the MOG has a card window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowLedger {
    cap: usize,
    mogs: BTreeMap<String, bool>,
}

impl Default for WindowLedger {
    fn default() -> Self {
        Self::new(MAX_ACTIVE_MOGS)
    }
}

impl WindowLedger {
    /// A ledger that allows `cap` active MOG windows.
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            mogs: BTreeMap::new(),
        }
    }

    /// Reserve a MOG window. Idempotent for a MOG that is open already.
    pub fn open_mog(&mut self, mog_id: &str) -> Result<OpenOutcome, WindowError> {
        let kind = WindowKind::mog(mog_id).ok_or(WindowError::InvalidId)?;
        if self.mogs.contains_key(mog_id) {
            return Ok(OpenOutcome::AlreadyOpen);
        }
        if self.mogs.len() >= self.cap {
            return Err(WindowError::CapReached { cap: self.cap });
        }
        self.mogs.insert(mog_id.to_owned(), false);
        Ok(OpenOutcome::Opened(kind))
    }

    /// Release a MOG window and its card. `false` if it was not open.
    pub fn close_mog(&mut self, mog_id: &str) -> bool {
        self.mogs.remove(mog_id).is_some()
    }

    /// Record the card window of an open MOG (made on the first show).
    pub fn show_card(&mut self, mog_id: &str) -> Result<WindowKind, WindowError> {
        let kind = WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        let card = self.mogs.get_mut(mog_id).ok_or(WindowError::UnknownMog)?;
        *card = true;
        Ok(kind)
    }

    /// True if the MOG has a card window.
    pub fn has_card(&self, mog_id: &str) -> bool {
        self.mogs.get(mog_id).copied().unwrap_or(false)
    }

    /// Follow a window that closed outside the host (crash, OS close). A `mog-<id>` frees its
    /// slot; a `card-<id>` is no longer open. Other labels change nothing.
    pub fn window_destroyed(&mut self, label: &str) {
        match WindowKind::from_label(label) {
            Some(WindowKind::Mog(id)) => {
                self.mogs.remove(&id);
            }
            Some(WindowKind::Card(id)) => {
                if let Some(card) = self.mogs.get_mut(&id) {
                    *card = false;
                }
            }
            _ => {}
        }
    }

    /// The number of open MOG windows.
    pub fn active_mogs(&self) -> usize {
        self.mogs.len()
    }

    /// The IDs of the open MOG windows.
    pub fn open_ids(&self) -> BTreeSet<String> {
        self.mogs.keys().cloned().collect()
    }
}

/// The R11 host for the Tauri shell. The app keeps one instance as managed state.
pub struct TauriWindowHost<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    ledger: Mutex<WindowLedger>,
}

impl<R: tauri::Runtime> TauriWindowHost<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self {
            app,
            ledger: Mutex::new(WindowLedger::default()),
        }
    }

    fn ledger(&self) -> std::sync::MutexGuard<'_, WindowLedger> {
        self.ledger.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn window(&self, kind: &WindowKind) -> Option<tauri::WebviewWindow<R>> {
        use tauri::Manager;
        self.app.get_webview_window(&kind.label())
    }

    /// Open the MOG window of `mog_id`. A second call shows the window, or makes it again if
    /// it is gone. The ledger lock is not held while the window is built.
    pub fn open_mog_window(&self, mog_id: &str) -> Result<(), WindowError> {
        let outcome = self.ledger().open_mog(mog_id)?;
        let kind = match outcome {
            OpenOutcome::Opened(kind) => kind,
            OpenOutcome::AlreadyOpen => {
                let kind = WindowKind::Mog(mog_id.to_owned());
                if let Some(w) = self.window(&kind) {
                    return w.show().map_err(|_| WindowError::HostFailed);
                }
                kind
            }
        };
        if windows::build(&self.app, &kind.spec()).is_err() {
            self.ledger().close_mog(mog_id);
            return Err(WindowError::HostFailed);
        }
        Ok(())
    }

    /// Close the MOG window of `mog_id` and its card window. The slot is freed only after
    /// both windows are gone.
    pub fn close_mog_window(&self, mog_id: &str) -> Result<(), WindowError> {
        let mog = WindowKind::mog(mog_id).ok_or(WindowError::InvalidId)?;
        if !self.ledger().open_ids().contains(mog_id) {
            return Err(WindowError::UnknownMog);
        }
        for kind in [WindowKind::Card(mog_id.to_owned()), mog] {
            if let Some(w) = self.window(&kind) {
                w.destroy().map_err(|_| WindowError::HostFailed)?;
            }
        }
        self.ledger().close_mog(mog_id);
        Ok(())
    }

    /// The number of open MOG windows in the ledger.
    pub fn active_mogs(&self) -> usize {
        self.ledger().active_mogs()
    }

    /// Call from the app's `WindowEvent::Destroyed` handler, so the ledger follows the windows.
    pub fn window_destroyed(&self, label: &str) {
        self.ledger().window_destroyed(label);
    }

    /// Show the card window of an open MOG; make it on the first show.
    pub fn show_card(&self, mog_id: &str) -> Result<(), WindowError> {
        let kind = self.ledger().show_card(mog_id)?;
        // The ledger lock is released before the window is built.
        let w = windows::build(&self.app, &kind.spec()).map_err(|_| WindowError::HostFailed)?;
        w.show().map_err(|_| WindowError::HostFailed)
    }

    /// Hide the card window of `mog_id`. The reaper of #78 destroys it later.
    pub fn hide_card(&self, mog_id: &str) -> Result<(), WindowError> {
        let kind = WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        match self.window(&kind) {
            Some(w) => w.hide().map_err(|_| WindowError::HostFailed),
            None => Err(WindowError::UnknownMog),
        }
    }
}
