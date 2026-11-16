//! R11 WindowHost: makes and closes the `mog-<id>` and `card-<id>` windows at run time (ARC-3).
//!
//! `WindowLedger` is the pure part: the cap of 8 active MOGs, label-safe IDs and one card per
//! open MOG. `TauriWindowHost` applies the ledger to real windows. Every label it makes matches
//! exactly one capability glob (`mog-*`, `card-*`), so the window gets only `view_emit`.
//!
//! TODO(#285): the gateway does not route `window_mog_open`, `window_mog_close`,
//! `window_card_show` and `window_card_hide` to this host yet (contract change).
//!
//! The card window (#78, ADR 0022) is made on the first show (MOG-18a). `spawn_card_reaper`
//! destroys a card that is hidden for `CARD_HIDDEN_TTL` (MOG-18b). A window that the host
//! destroys stays "closing" until its `Destroyed` event comes, so a late event never frees the
//! slot of a new window with the same label, and the new window never adopts the dying one.

use crate::windows::{self, WindowKind, CARD_HIDDEN_TTL, CARD_REAP_TICK, MAX_ACTIVE_MOGS};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    /// The old window with this label is still closing; try again after it is gone.
    Closing,
}

impl WindowError {
    /// The stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            WindowError::InvalidId => "invalid_id",
            WindowError::CapReached { .. } => "mog_cap_reached",
            WindowError::UnknownMog => "unknown_mog",
            WindowError::HostFailed => "host_failed",
            WindowError::Closing => "window_closing",
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
            WindowError::Closing => write!(f, "the old window is still closing"),
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

/// The card window of one open MOG (ARC-3, MOG-18a, MOG-18b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardState {
    /// No card WebView exists. The next show makes it.
    Absent,
    /// The card is on screen.
    Shown,
    /// The card is hidden since this time; the reaper destroys it after `CARD_HIDDEN_TTL`.
    Hidden(Instant),
    /// The reaper destroys the card; a show waits until its `Destroyed` event comes.
    Closing,
}

/// Which MOG and card windows are open, and which windows the host destroyed and still waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowLedger {
    cap: usize,
    mogs: BTreeMap<String, CardState>,
    /// Labels of windows the host destroyed, with the number of `Destroyed` events still due.
    closing: BTreeMap<String, usize>,
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
            closing: BTreeMap::new(),
        }
    }

    fn is_closing(&self, kind: &WindowKind) -> bool {
        self.closing.contains_key(&kind.label())
    }

    /// Reserve a MOG window. Idempotent for a MOG that is open already.
    pub fn open_mog(&mut self, mog_id: &str) -> Result<OpenOutcome, WindowError> {
        let kind = WindowKind::mog(mog_id).ok_or(WindowError::InvalidId)?;
        if self.mogs.contains_key(mog_id) {
            return Ok(OpenOutcome::AlreadyOpen);
        }
        if self.is_closing(&kind) {
            return Err(WindowError::Closing);
        }
        if self.mogs.len() >= self.cap {
            return Err(WindowError::CapReached { cap: self.cap });
        }
        self.mogs.insert(mog_id.to_owned(), CardState::Absent);
        Ok(OpenOutcome::Opened(kind))
    }

    /// Release a MOG window and its card. `false` if it was not open.
    pub fn close_mog(&mut self, mog_id: &str) -> bool {
        self.mogs.remove(mog_id).is_some()
    }

    /// Mark the card of an open MOG as shown. The caller makes the window if the state was
    /// `Absent` (MOG-18a).
    pub fn show_card(&mut self, mog_id: &str) -> Result<WindowKind, WindowError> {
        let kind = WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        let closing = self.is_closing(&kind);
        let card = self.mogs.get_mut(mog_id).ok_or(WindowError::UnknownMog)?;
        if closing || *card == CardState::Closing {
            return Err(WindowError::Closing);
        }
        *card = CardState::Shown;
        Ok(kind)
    }

    /// Mark the card of `mog_id` as hidden at `now`. A second hide keeps the first time; a hide
    /// of a closing card changes nothing. A MOG without a card gives `UnknownMog`.
    pub fn hide_card(&mut self, mog_id: &str, now: Instant) -> Result<WindowKind, WindowError> {
        let kind = WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        let card = self.mogs.get_mut(mog_id).ok_or(WindowError::UnknownMog)?;
        match *card {
            CardState::Absent => return Err(WindowError::UnknownMog),
            CardState::Shown => *card = CardState::Hidden(now),
            CardState::Hidden(_) | CardState::Closing => {}
        }
        Ok(kind)
    }

    /// The card state of an open MOG, or `None` if the MOG is not open.
    pub fn card_state(&self, mog_id: &str) -> Option<CardState> {
        self.mogs.get(mog_id).copied()
    }

    /// True if the MOG has a card window (shown, hidden or closing).
    pub fn has_card(&self, mog_id: &str) -> bool {
        self.card_state(mog_id)
            .is_some_and(|c| c != CardState::Absent)
    }

    /// The cards hidden for `CARD_HIDDEN_TTL` or more at `now` (MOG-18b). They become `Closing`,
    /// so each one is returned once.
    pub fn take_due_cards(&mut self, now: Instant) -> Vec<WindowKind> {
        let mut due = Vec::new();
        for (id, card) in &mut self.mogs {
            if let CardState::Hidden(since) = *card {
                if now.saturating_duration_since(since) >= CARD_HIDDEN_TTL {
                    *card = CardState::Closing;
                    due.push(WindowKind::Card(id.clone()));
                }
            }
        }
        due
    }

    /// A card that the reaper could not destroy is hidden again from `now`.
    pub fn reap_failed(&mut self, mog_id: &str, now: Instant) {
        if let Some(card) = self.mogs.get_mut(mog_id) {
            if *card == CardState::Closing {
                *card = CardState::Hidden(now);
            }
        }
    }

    /// A card that failed to open is not open: the next show makes it again.
    pub fn card_failed(&mut self, mog_id: &str) {
        if let Some(card) = self.mogs.get_mut(mog_id) {
            *card = CardState::Absent;
        }
    }

    /// The host destroyed the window `label`; its `Destroyed` event is due.
    pub fn expect_destroyed(&mut self, label: &str) {
        *self.closing.entry(label.to_owned()).or_default() += 1;
    }

    /// Follow a `Destroyed` event. An event the host expects (it destroyed the window) only
    /// ends the closing state. Otherwise the window closed outside the host (crash, OS close):
    /// a `mog-<id>` frees its slot and returns its card, which the caller must destroy; a
    /// `card-<id>` is no longer open. Other labels change nothing.
    pub fn window_destroyed(&mut self, label: &str) -> Vec<WindowKind> {
        let kind = WindowKind::from_label(label);
        if let Some(n) = self.closing.get_mut(label) {
            *n -= 1;
            if *n == 0 {
                self.closing.remove(label);
            }
            if let Some(WindowKind::Card(id)) = &kind {
                if self.card_state(id) == Some(CardState::Closing) {
                    self.card_failed(id);
                }
            }
            return Vec::new();
        }
        match kind {
            Some(WindowKind::Mog(id)) => match self.mogs.remove(&id) {
                Some(CardState::Shown | CardState::Hidden(_)) => vec![WindowKind::Card(id)],
                _ => Vec::new(),
            },
            Some(WindowKind::Card(id)) => {
                self.card_failed(&id);
                Vec::new()
            }
            _ => Vec::new(),
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

/// The platform step that turns a built window into its native kind and shows it.
/// On macOS, `macos_panel::MacPanelStyle` makes each MOG window a non-activating NSPanel
/// (MOG-02). Tests use a double, because the Tauri mock runtime has no real window.
pub trait NativeWindowStyle<R: tauri::Runtime>: Send + Sync {
    /// Called once, after the window is built and before it shows. Off the main thread an
    /// implementation may queue the native step and return `Ok` before it runs; it must then
    /// destroy the window itself if the step fails (the Destroyed event frees the slot).
    fn style(&self, window: &tauri::WebviewWindow<R>, kind: &WindowKind)
        -> Result<(), WindowError>;
    /// Show the window. It must not activate Bottle or take focus for a MOG window.
    fn show(&self, window: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError>;
}

/// No native step: Tauri shows the window. Used on platforms without a native style yet.
/// TODO(#80): the Windows `WS_EX_NOACTIVATE` style.
pub struct PlainStyle;

impl<R: tauri::Runtime> NativeWindowStyle<R> for PlainStyle {
    fn style(&self, _: &tauri::WebviewWindow<R>, _: &WindowKind) -> Result<(), WindowError> {
        Ok(())
    }

    fn show(&self, window: &tauri::WebviewWindow<R>, _: &WindowKind) -> Result<(), WindowError> {
        window.show().map_err(|_| WindowError::HostFailed)
    }
}

/// The time source of the host. Tests move a fake clock; the app uses `Instant::now`.
pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// The R11 host for the Tauri shell. The app keeps one instance as managed state.
pub struct TauriWindowHost<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    ledger: Mutex<WindowLedger>,
    native: Box<dyn NativeWindowStyle<R>>,
    clock: Clock,
}

impl<R: tauri::Runtime> TauriWindowHost<R> {
    /// A host with no native window step (`PlainStyle`).
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self::with_style(app, Box::new(PlainStyle))
    }

    /// A host that applies `native` to every window it makes.
    pub fn with_style(app: tauri::AppHandle<R>, native: Box<dyn NativeWindowStyle<R>>) -> Self {
        Self {
            app,
            ledger: Mutex::new(WindowLedger::default()),
            native,
            clock: Arc::new(Instant::now),
        }
    }

    /// Use `clock` for the hidden-card timer (MOG-18b).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Build a window hidden if it is a panel, apply the native step, then show it.
    fn make(&self, kind: &WindowKind) -> Result<tauri::WebviewWindow<R>, WindowError> {
        let w =
            windows::build(&self.app, &kind.build_spec()).map_err(|_| WindowError::HostFailed)?;
        // A MOG window that is not a panel could take focus: do not keep a half-made window.
        if let Err(e) = self
            .native
            .style(&w, kind)
            .and_then(|()| self.native.show(&w, kind))
        {
            if w.destroy().is_ok() {
                self.ledger().expect_destroyed(&kind.label());
            }
            return Err(e);
        }
        Ok(w)
    }

    fn ledger(&self) -> std::sync::MutexGuard<'_, WindowLedger> {
        self.ledger.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn window(&self, kind: &WindowKind) -> Option<tauri::WebviewWindow<R>> {
        use tauri::Manager;
        self.app.get_webview_window(&kind.label())
    }

    /// Destroy the window of `kind` if it exists and is not closing already. Its `Destroyed`
    /// event is then expected.
    fn destroy(&self, kind: &WindowKind) -> Result<(), WindowError> {
        let label = kind.label();
        if self.ledger().closing.contains_key(&label) {
            return Ok(());
        }
        if let Some(w) = self.window(kind) {
            w.destroy().map_err(|_| WindowError::HostFailed)?;
            self.ledger().expect_destroyed(&label);
        }
        Ok(())
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
                    return self.native.show(&w, &kind);
                }
                kind
            }
        };
        if let Err(e) = self.make(&kind) {
            self.ledger().close_mog(mog_id);
            return Err(e);
        }
        Ok(())
    }

    /// Close the MOG window of `mog_id` and its card window. The slot is freed only after
    /// both windows are destroyed; the label is used again after their `Destroyed` events.
    pub fn close_mog_window(&self, mog_id: &str) -> Result<(), WindowError> {
        let mog = WindowKind::mog(mog_id).ok_or(WindowError::InvalidId)?;
        if !self.ledger().open_ids().contains(mog_id) {
            return Err(WindowError::UnknownMog);
        }
        for kind in [WindowKind::Card(mog_id.to_owned()), mog] {
            self.destroy(&kind)?;
        }
        self.ledger().close_mog(mog_id);
        Ok(())
    }

    /// The number of open MOG windows in the ledger.
    pub fn active_mogs(&self) -> usize {
        self.ledger().active_mogs()
    }

    /// True if the MOG has a card window in the ledger.
    pub fn has_card(&self, mog_id: &str) -> bool {
        self.ledger().has_card(mog_id)
    }

    /// The card state of an open MOG, or `None` if the MOG is not open.
    pub fn card_state(&self, mog_id: &str) -> Option<CardState> {
        self.ledger().card_state(mog_id)
    }

    /// Call from the app's `WindowEvent::Destroyed` handler, so the ledger follows the windows.
    /// The card of a MOG window that closed outside the host is destroyed too.
    pub fn window_destroyed(&self, label: &str) {
        let orphans = self.ledger().window_destroyed(label);
        for card in orphans {
            // If the destroy fails, the card's own `Destroyed` event never comes; nothing waits.
            let _ = self.destroy(&card);
        }
    }

    /// Show the card window of an open MOG; make it on the first show (MOG-18a). The card
    /// shows through the native step only, which never activates Bottle (MOG-03b).
    pub fn show_card(&self, mog_id: &str) -> Result<(), WindowError> {
        let (was, kind) = {
            let mut ledger = self.ledger();
            let was = ledger.card_state(mog_id);
            (was, ledger.show_card(mog_id)?)
        };
        // The ledger lock is released before the window is built.
        let existing = match was {
            Some(CardState::Absent) | None => None,
            _ => self.window(&kind),
        };
        match existing {
            Some(w) => self.native.show(&w, &kind),
            None => self.make(&kind).map(|_| ()).inspect_err(|_| {
                // The card did not open: clear its mark, so the next show makes it again.
                self.ledger().card_failed(mog_id);
            }),
        }
    }

    /// Hide the card window of `mog_id`. The ledger marks it hidden only after the window
    /// hid, so a failed hide leaves no half state (UX-19b). The reaper destroys it later.
    pub fn hide_card(&self, mog_id: &str) -> Result<(), WindowError> {
        let kind = WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        match self.ledger().card_state(mog_id) {
            None | Some(CardState::Absent) => return Err(WindowError::UnknownMog),
            Some(CardState::Closing) => return Ok(()),
            Some(CardState::Shown | CardState::Hidden(_)) => {}
        }
        let w = self.window(&kind).ok_or(WindowError::UnknownMog)?;
        w.hide().map_err(|_| WindowError::HostFailed)?;
        let now = (self.clock)();
        self.ledger().hide_card(mog_id, now).map(|_| ())
    }

    /// Destroy every card hidden for `CARD_HIDDEN_TTL` (MOG-18b). Returns how many went.
    pub fn reap_hidden_cards(&self) -> usize {
        let now = (self.clock)();
        let due = self.ledger().take_due_cards(now);
        let mut reaped = 0;
        for kind in due {
            let WindowKind::Card(id) = &kind else {
                continue;
            };
            match self.window(&kind) {
                Some(w) => match w.destroy() {
                    Ok(()) => {
                        self.ledger().expect_destroyed(&kind.label());
                        reaped += 1;
                    }
                    Err(_) => self.ledger().reap_failed(id, now),
                },
                // The window is gone already: no event will come.
                None => {
                    self.ledger().card_failed(id);
                    reaped += 1;
                }
            }
        }
        reaped
    }
}

/// Start the thread that calls `reap_hidden_cards` every `CARD_REAP_TICK` (MOG-18b). It ends
/// when the host is no longer managed state.
pub fn spawn_card_reaper<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    use tauri::Manager;
    std::thread::Builder::new()
        .name("bottle-card-reaper".into())
        .spawn(move || loop {
            std::thread::sleep(CARD_REAP_TICK);
            match app.try_state::<TauriWindowHost<R>>() {
                Some(host) => {
                    host.reap_hidden_cards();
                }
                None => return,
            }
        })
}
