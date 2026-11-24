//! R11 WindowHost: makes and closes the `mog-<id>` and `card-<id>` windows at run time (ARC-3).
//!
//! `WindowLedger` is the pure part: the cap of 8 active MOGs, label-safe IDs and one card per
//! open MOG. `TauriWindowHost` applies the ledger to real windows. Every label it makes matches
//! exactly one capability glob (`mog-*`, `card-*`), so the window gets only `view_emit`.
//!
//! The IPC gateway routes `window_mog_open`, `window_mog_close`, `window_card_show` and
//! `window_card_hide` to this host through `mog_core::window_host::WindowHost` (#285,
//! ADR 0040). The commands run on a worker thread, never on the main thread (ADR 0022).
//!
//! The card window (#78, ADR 0022) is made on the first show (MOG-18a). `spawn_card_reaper`
//! destroys a card that is hidden for `CARD_HIDDEN_TTL` (MOG-18b). A window that the host
//! destroys stays "closing" until its `Destroyed` event comes, so a late event never frees the
//! slot of a new window with the same label, and the new window never adopts the dying one.

use crate::windows::{self, WindowKind, CARD_HIDDEN_TTL, CARD_REAP_TICK, MAX_ACTIVE_MOGS};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::Geometry;
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
    /// A window operation was called on the main thread after setup. It could wait for the
    /// main thread (a window build) and deadlock, so it is refused (ADR 0022).
    MainThread,
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
            WindowError::MainThread => "main_thread",
        }
    }
}

impl WindowError {
    /// The IPC code of this refusal (`ipc_gateway::ERROR_CODES`, ADR 0040 decision 3).
    pub fn ipc_code(&self) -> &'static str {
        match self {
            WindowError::InvalidId => "invalid_args",
            WindowError::CapReached { .. } => "mog_cap_reached",
            WindowError::UnknownMog => "unknown_mog",
            WindowError::Closing => "window_closing",
            WindowError::HostFailed | WindowError::MainThread => "host_failed",
        }
    }
}

impl From<WindowError> for CoreError {
    fn from(e: WindowError) -> Self {
        CoreError::IpcRefused { code: e.ipc_code() }
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
            WindowError::MainThread => write!(f, "window operations do not run on the main thread"),
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

/// What `begin_show` decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowPlan {
    /// No card WebView exists: make it (MOG-18a).
    Make,
    /// Show the live card window. The value is the state before the show, to restore it if
    /// the native show fails.
    Reuse(CardState),
}

/// Which MOG and card windows are open, and which windows the host destroyed and still waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowLedger {
    cap: usize,
    mogs: BTreeMap<String, CardState>,
    /// Labels of windows the host destroyed and whose `Destroyed` event is still due. One
    /// window gives one event, so a label is in the set at most once.
    closing: BTreeSet<String>,
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
            closing: BTreeSet::new(),
        }
    }

    /// True if a `Destroyed` event of the window of `kind` is due.
    pub fn is_closing(&self, kind: &WindowKind) -> bool {
        self.closing.contains(&kind.label())
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

    /// A show of a hidden card failed: it stays hidden from `since`. Only a card still
    /// `Shown` (the state `begin_show` wrote) changes; a card that closed meanwhile stays as is.
    pub fn restore_hidden(&mut self, mog_id: &str, since: Instant) {
        if let Some(card) = self.mogs.get_mut(mog_id) {
            if *card == CardState::Shown {
                *card = CardState::Hidden(since);
            }
        }
    }

    /// A card that failed to open is not open: the next show makes it again.
    pub fn card_failed(&mut self, mog_id: &str) {
        if let Some(card) = self.mogs.get_mut(mog_id) {
            *card = CardState::Absent;
        }
    }

    /// The `Destroyed` event of the window `label` is due. A second call for the same window
    /// changes nothing.
    pub fn expect_destroyed(&mut self, label: &str) {
        self.closing.insert(label.to_owned());
    }

    /// Record a destroy before it is sent, so its event can never come first. `window_exists`
    /// is whether Tauri still has the window, read under the same ledger lock. Tauri drops a
    /// window from its manager before the app's `Destroyed` handler runs, and that handler
    /// needs this lock, so a live window here means its event is still to come. `false`: do not
    /// destroy (the window is gone, or a destroy is on its way already).
    pub fn begin_closing(&mut self, label: &str, window_exists: bool) -> bool {
        window_exists && self.closing.insert(label.to_owned())
    }

    /// The destroy that `begin_closing` recorded failed: no event comes.
    pub fn cancel_closing(&mut self, label: &str) {
        self.closing.remove(label);
    }

    /// Start a card show. `window_exists` is whether Tauri has the card window, read under the
    /// same ledger lock, so the lookup and the state change are one step. The card is `Shown`
    /// on `Ok`. A card the ledger has but Tauri has not is mid-destroy (`card_window_missing`).
    pub fn begin_show(
        &mut self,
        mog_id: &str,
        window_exists: bool,
    ) -> Result<ShowPlan, WindowError> {
        let kind = WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        let was = self.card_state(mog_id).ok_or(WindowError::UnknownMog)?;
        if self.is_closing(&kind) || was == CardState::Closing {
            return Err(WindowError::Closing);
        }
        let plan = match was {
            CardState::Absent => ShowPlan::Make,
            _ if !window_exists => {
                self.card_window_missing(mog_id);
                return Err(WindowError::Closing);
            }
            _ => ShowPlan::Reuse(was),
        };
        self.show_card(mog_id)?;
        Ok(plan)
    }

    /// Start a card hide, with the window lookup in the same ledger lock. `Ok(true)`: hide the
    /// window, then call `hide_card`. `Ok(false)`: the card is closing; nothing to hide.
    pub fn begin_hide(&mut self, mog_id: &str, window_exists: bool) -> Result<bool, WindowError> {
        WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        match self.card_state(mog_id) {
            None | Some(CardState::Absent) => Err(WindowError::UnknownMog),
            Some(CardState::Closing) => Ok(false),
            Some(CardState::Shown | CardState::Hidden(_)) if !window_exists => {
                self.card_window_missing(mog_id);
                Err(WindowError::UnknownMog)
            }
            Some(_) => Ok(true),
        }
    }

    /// The MOG window of `mog_id` left Tauri's manager, but its `Destroyed` event is not handled
    /// yet. The MOG is closed now and its label waits for that event, so a reopen does not make
    /// a second window whose slot the late event then frees. Returns the MOG's card if it has
    /// one: the caller must destroy it, as for a MOG destroyed outside the host.
    pub fn mog_window_missing(&mut self, mog_id: &str) -> Option<WindowKind> {
        let card = self.mogs.remove(mog_id)?;
        let kind = WindowKind::mog(mog_id)?;
        self.closing.insert(kind.label());
        matches!(card, CardState::Shown | CardState::Hidden(_))
            .then(|| WindowKind::Card(mog_id.to_owned()))
    }

    /// The card window of `mog_id` left Tauri's manager, but its `Destroyed` event is not
    /// handled yet. The card is absent, and its label waits for that event, so a show does
    /// not make a new window that the late event then marks absent.
    pub fn card_window_missing(&mut self, mog_id: &str) {
        if let Some(card) = self.mogs.get_mut(mog_id) {
            if matches!(*card, CardState::Shown | CardState::Hidden(_)) {
                *card = CardState::Absent;
                if let Some(kind) = WindowKind::card(mog_id) {
                    self.closing.insert(kind.label());
                }
            }
        }
    }

    /// Follow a `Destroyed` event. An event the host expects (it destroyed the window) only
    /// ends the closing state. Otherwise the window closed outside the host (crash, OS close):
    /// a `mog-<id>` frees its slot and returns its card, which the caller must destroy; a
    /// `card-<id>` is no longer open. Other labels change nothing.
    pub fn window_destroyed(&mut self, label: &str) -> Vec<WindowKind> {
        let kind = WindowKind::from_label(label);
        if self.closing.remove(label) {
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
    /// Hide the window (`orderOut` on macOS).
    fn hide(
        &self,
        window: &tauri::WebviewWindow<R>,
        _kind: &WindowKind,
    ) -> Result<(), WindowError> {
        window.hide().map_err(|_| WindowError::HostFailed)
    }
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
///
/// Two locks: `ops` makes each public operation (open, close, show, hide, reap) one critical
/// section from its ledger read to its ledger write, so the ledger always matches the window
/// (UX-19b). `ledger` guards the state and is held only briefly; each window lookup and the
/// state change it decides are made under one `ledger` lock. `window_destroyed` takes `ledger`
/// only, because it runs on the main thread.
///
/// Thread rule (ADR 0022): an operation can wait for the main thread while it holds `ops`
/// (off the main thread, a window build blocks until the main thread made the window). So
/// after `seal_main_thread` (end of setup) the operations refuse to run on the main thread
/// (`WindowError::MainThread`): a main-thread caller could wait for `ops` while the holder
/// waits for the main thread. The IPC host runs the `window_*` commands on a worker (ADR 0040).
pub struct TauriWindowHost<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    ops: Mutex<()>,
    /// The thread that sealed the host (the main thread, at the end of setup).
    sealed: Mutex<Option<std::thread::ThreadId>>,
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
            ops: Mutex::new(()),
            sealed: Mutex::new(None),
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

    /// End of setup. Call on the main thread: from now on the operations refuse to run on
    /// this thread. The check compares thread IDs, so it holds on every OS.
    pub fn seal_main_thread(&self) {
        *self.sealed.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::thread::current().id());
    }

    fn op(&self) -> Result<std::sync::MutexGuard<'_, ()>, WindowError> {
        let sealed = *self.sealed.lock().unwrap_or_else(|p| p.into_inner());
        if sealed == Some(std::thread::current().id()) {
            return Err(WindowError::MainThread);
        }
        Ok(self.ops.lock().unwrap_or_else(|p| p.into_inner()))
    }

    fn ledger(&self) -> std::sync::MutexGuard<'_, WindowLedger> {
        self.ledger.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn window(&self, kind: &WindowKind) -> Option<tauri::WebviewWindow<R>> {
        use tauri::Manager;
        self.app.get_webview_window(&kind.label())
    }

    /// Build a window hidden if it is a panel, place it at `geometry` if given, apply the
    /// native step, then show it.
    fn make(
        &self,
        kind: &WindowKind,
        geometry: Option<&Geometry>,
    ) -> Result<tauri::WebviewWindow<R>, WindowError> {
        let w =
            windows::build(&self.app, &kind.build_spec()).map_err(|_| WindowError::HostFailed)?;
        // A MOG window that is not a panel could take focus: do not keep a half-made window.
        if let Err(e) = place(&w, geometry)
            .and_then(|()| self.native.style(&w, kind))
            .and_then(|()| self.native.show(&w, kind))
        {
            let _ = self.destroy(kind);
            return Err(e);
        }
        Ok(w)
    }

    /// Destroy the window of `kind` if it exists and no destroy of it is on its way. The
    /// `Destroyed` event is recorded before the destroy is sent, in the same ledger lock as the
    /// window lookup (`WindowLedger::begin_closing`), so the event never comes first.
    fn destroy(&self, kind: &WindowKind) -> Result<(), WindowError> {
        let label = kind.label();
        let w = {
            let mut ledger = self.ledger();
            let w = self.window(kind);
            if !ledger.begin_closing(&label, w.is_some()) {
                return Ok(());
            }
            w
        };
        match w.map(|w| w.destroy()) {
            Some(Err(_)) => {
                self.ledger().cancel_closing(&label);
                Err(WindowError::HostFailed)
            }
            _ => Ok(()),
        }
    }

    /// Open the MOG window of `mog_id`. A second call shows the window, or makes it again if
    /// it is gone.
    pub fn open_mog_window(&self, mog_id: &str) -> Result<(), WindowError> {
        self.open_mog_window_at(mog_id, None)
    }

    /// Open the MOG window of `mog_id` at `geometry` (logical units). A second call moves
    /// the open window there and shows it.
    pub fn open_mog_window_at(
        &self,
        mog_id: &str,
        geometry: Option<&Geometry>,
    ) -> Result<(), WindowError> {
        let _op = self.op()?;
        let (kind, existing, missing) = {
            let mut ledger = self.ledger();
            match ledger.open_mog(mog_id)? {
                OpenOutcome::Opened(kind) => (kind, None, None),
                OpenOutcome::AlreadyOpen => {
                    let kind = WindowKind::Mog(mog_id.to_owned());
                    match self.window(&kind) {
                        Some(w) => (kind, Some(w), None),
                        // Mid-destroy: the MOG waits for its event and its card goes with it.
                        None => (kind, None, Some(ledger.mog_window_missing(mog_id))),
                    }
                }
            }
        };
        if let Some(orphan) = missing {
            if let Some(card) = orphan {
                let _ = self.destroy(&card);
            }
            return Err(WindowError::Closing);
        }
        if let Some(w) = existing {
            place(&w, geometry)?;
            self.native.show(&w, &kind)?;
            // lamp-alive: the eye poller sleeps with no visible MOG; a shown one wakes it.
            crate::mog_look_host::gate().wake();
            return Ok(());
        }
        // Only this call made the reservation, and its window does not exist: roll it back.
        if let Err(e) = self.make(&kind, geometry) {
            self.ledger().close_mog(mog_id);
            return Err(e);
        }
        crate::mog_look_host::gate().wake();
        Ok(())
    }

    /// Close the MOG window of `mog_id` and its card window. The slot is freed only after
    /// both windows are destroyed; the label is used again after their `Destroyed` events.
    pub fn close_mog_window(&self, mog_id: &str) -> Result<(), WindowError> {
        let _op = self.op()?;
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
            // A failed destroy takes its record back, so nothing waits for an event.
            let _ = self.destroy(&card);
        }
    }

    /// Show the card window of an open MOG; make it on the first show (MOG-18a). The card
    /// shows through the native step only, which never activates Bottle (MOG-03b). If the show
    /// fails, the card keeps its earlier state.
    pub fn show_card(&self, mog_id: &str) -> Result<(), WindowError> {
        let _op = self.op()?;
        let kind = WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        let (plan, existing) = {
            let mut ledger = self.ledger();
            let w = self.window(&kind);
            (ledger.begin_show(mog_id, w.is_some())?, w)
        };
        // #559: the pop-up goes beside its MOG. With no MOG geometry it keeps its place.
        // #573: a card that is on screen keeps the height its view fitted (the answer card); only
        // its place follows the MOG. A hidden card starts again at one line.
        let kept = existing
            .as_ref()
            .filter(|w| w.is_visible().unwrap_or(false))
            .and_then(logical_size);
        let placed = self.card_geometry(mog_id, kept);
        let at = placed.as_ref().map(|(g, _)| g.clone());
        let side = placed.map(|(_, side)| side);
        let shown = match (plan, &existing) {
            (ShowPlan::Reuse(_), Some(w)) => {
                // A failed move is not a failed show: the card shows where it was.
                let _ = if kept.is_some() {
                    move_to(w, at.as_ref())
                } else {
                    place(w, at.as_ref())
                };
                // The side goes into the title before the card shows, so the view never reads
                // the side of its last show.
                set_side_title(w, side);
                self.native.show(w, &kind)
            }
            _ => self.make(&kind, at.as_ref()).map(|w| {
                set_side_title(&w, side);
            }),
        };
        if shown.is_err() {
            let mut ledger = self.ledger();
            match plan {
                // The card is still there, hidden as before.
                ShowPlan::Reuse(CardState::Hidden(since)) => ledger.restore_hidden(mog_id, since),
                ShowPlan::Reuse(_) => {}
                // The card did not open: the next show makes it again.
                ShowPlan::Make => ledger.card_failed(mog_id),
            }
        }
        shown
    }

    /// The MOG window moved (the user dragged the lamp): a card that is on screen goes with it,
    /// to the same place beside the MOG, keeping its size. This runs on the main thread (a
    /// window event), so it takes no `op` guard and never builds or waits for a window. If the
    /// ledger is busy, it skips this step: the next move event comes at once. A card that is
    /// hidden, absent or closing stays where it is; its next show places it.
    pub fn follow_mog(&self, mog_id: &str) {
        let Ok(ledger) = self.ledger.try_lock() else {
            return;
        };
        let state = ledger.card_state(mog_id);
        drop(ledger);
        if !card_follows_mog(state) {
            return;
        }
        let Some(kind) = WindowKind::card(mog_id) else {
            return;
        };
        let Some(w) = self.window(&kind) else {
            return;
        };
        let Some(size) = logical_size(&w) else {
            return;
        };
        let Some((at, side)) = self.card_geometry(mog_id, Some(size)) else {
            return;
        };
        let _ = move_to(&w, Some(&at));
        set_side_title(&w, Some(side));
    }

    /// #559: the card place beside the MOG window of `mog_id`, in logical units, on the MOG's
    /// monitor. `None` if the MOG window or its geometry is not there.
    /// `kept` is the `(width, height)` of a card that keeps its size (#573); `None` is one line.
    fn card_geometry(
        &self,
        mog_id: &str,
        kept: Option<(f64, f64)>,
    ) -> Option<(Geometry, windows::CardSide)> {
        let w = self.window(&WindowKind::mog(mog_id)?)?;
        let scale = w.scale_factor().ok()?;
        let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
        let size = w.outer_size().ok()?.to_logical::<f64>(scale);
        let mog = Geometry {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
        };
        let screen = w.current_monitor().ok().flatten().map(|m| {
            let s = m.scale_factor();
            let p = m.position().to_logical::<f64>(s);
            let z = m.size().to_logical::<f64>(s);
            Geometry {
                x: p.x,
                y: p.y,
                width: z.width,
                height: z.height,
            }
        });
        let (w, h) = kept.unwrap_or((
            f64::from(windows::CARD_WIDTH),
            f64::from(windows::CARD_HEIGHT),
        ));
        Some(windows::card_beside_sized(&mog, screen.as_ref(), w, h))
    }

    /// Hide the card window of `mog_id`. The ledger marks it hidden only after the window
    /// hid, so a failed hide leaves no half state (UX-19b). The reaper destroys it later.
    pub fn hide_card(&self, mog_id: &str) -> Result<(), WindowError> {
        let _op = self.op()?;
        let kind = WindowKind::card(mog_id).ok_or(WindowError::InvalidId)?;
        let w = {
            let mut ledger = self.ledger();
            let w = self.window(&kind);
            if !ledger.begin_hide(mog_id, w.is_some())? {
                return Ok(());
            }
            w
        };
        let Some(w) = w else {
            return Err(WindowError::UnknownMog);
        };
        self.native.hide(&w, &kind)?;
        let now = (self.clock)();
        // If the window closed meanwhile, its event made the card absent: nothing shows.
        let _ = self.ledger().hide_card(mog_id, now);
        Ok(())
    }

    /// Destroy every card hidden for `CARD_HIDDEN_TTL` (MOG-18b). Returns how many went.
    pub fn reap_hidden_cards(&self) -> usize {
        let Ok(_op) = self.op() else {
            return 0;
        };
        let now = (self.clock)();
        let due = self.ledger().take_due_cards(now);
        let mut reaped = 0;
        for kind in due {
            let WindowKind::Card(id) = &kind else {
                continue;
            };
            if self.destroy(&kind).is_err() {
                self.ledger().reap_failed(id, now);
                continue;
            }
            let mut ledger = self.ledger();
            if !ledger.is_closing(&kind) {
                // No window to destroy, or its event came already: the card is gone.
                ledger.card_failed(id);
            }
            reaped += 1;
        }
        reaped
    }
}

/// True if a card in this state goes with its MOG when the lamp moves: only a card on screen.
pub fn card_follows_mog(state: Option<CardState>) -> bool {
    state == Some(CardState::Shown)
}

/// Tell the card view which side of its MOG it is on, in the window title (`windows::card_title`).
/// The title is not shown (no decorations); the view reads it, so the card needs no read of the
/// MOG window. A failed set changes nothing.
fn set_side_title<R: tauri::Runtime>(w: &tauri::WebviewWindow<R>, side: Option<windows::CardSide>) {
    if let Some(side) = side {
        let _ = w.set_title(&windows::card_title(side));
    }
}

/// The logical `(width, height)` of `w`, if the platform gives it.
fn logical_size<R: tauri::Runtime>(w: &tauri::WebviewWindow<R>) -> Option<(f64, f64)> {
    let scale = w.scale_factor().ok()?;
    let size = w.outer_size().ok()?.to_logical::<f64>(scale);
    Some((size.width, size.height))
}

/// Move `w` to the place of `geometry` (logical units) and keep its size. `None` changes nothing.
fn move_to<R: tauri::Runtime>(
    w: &tauri::WebviewWindow<R>,
    geometry: Option<&Geometry>,
) -> Result<(), WindowError> {
    let Some(g) = geometry else {
        return Ok(());
    };
    w.set_position(tauri::LogicalPosition::new(g.x, g.y))
        .map_err(|_| WindowError::HostFailed)
}

/// Move and size `w` to `geometry` (logical units). `None` changes nothing.
fn place<R: tauri::Runtime>(
    w: &tauri::WebviewWindow<R>,
    geometry: Option<&Geometry>,
) -> Result<(), WindowError> {
    let Some(g) = geometry else {
        return Ok(());
    };
    w.set_size(tauri::LogicalSize::new(g.width, g.height))
        .and_then(|()| w.set_position(tauri::LogicalPosition::new(g.x, g.y)))
        .map_err(|_| WindowError::HostFailed)
}

/// R11 through the core trait, for the IPC gateway (#285, ADR 0040). Each refusal is
/// `CoreError::IpcRefused` with the code of [`WindowError::ipc_code`].
impl<R: tauri::Runtime> mog_core::window_host::WindowHost for TauriWindowHost<R> {
    fn open_mog_window(&self, mog_id: &str, geometry: Option<&Geometry>) -> CoreResult<()> {
        Ok(self.open_mog_window_at(mog_id, geometry)?)
    }

    fn close_mog_window(&self, mog_id: &str) -> CoreResult<()> {
        Ok(TauriWindowHost::close_mog_window(self, mog_id)?)
    }

    fn show_card(&self, mog_id: &str) -> CoreResult<()> {
        Ok(TauriWindowHost::show_card(self, mog_id)?)
    }

    fn hide_card(&self, mog_id: &str) -> CoreResult<()> {
        Ok(TauriWindowHost::hide_card(self, mog_id)?)
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
