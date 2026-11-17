//! The host wiring of R1 HotkeyService (#305, ADR 0015).
//!
//! The Tauri host creates the platform HotkeyService with the D66 talk key and forwards its
//! outputs to the brain. `HotkeyOutput::Ptt(phase)` becomes the `hotkey:ptt` item
//! `{ mogId, phase }` on the brain Channel (ADR 0003), for the last-interacted MOG (MB-06a).
//!
//! Binding rules here:
//! - The host never prompts on its own. `start` only checks Input Monitoring. A prompt needs a
//!   [`UserAction`], which only a UI click handler makes (CAP-36, CAP-37).
//! - Nothing here logs. R1 outputs never carry key contents (ADR 0011).
//! - No `tauri-plugin-global-shortcut` (ARC-4): the platform service owns the OS hook.
//!
//! Platform seam: [`platform_service`] picks the service by `cfg`. macOS uses
//! `MacHotkeyService` (#6). Windows uses `WindowsHotkeyService` (#7) through the
//! [`HookSignal`] adapter (#314): one thread feeds the hook signals to the shared R1 engine,
//! which emits into the same queue sink. Other platforms use the R1 stub.
//!
//! Talk key: the service gets the saved `app.hotkeys.ptt` key from
//! [`crate::hotkey_settings::HotkeySettings`] (#322, ADR 0020). A new talk key replaces the
//! service: the host stops the old one and starts a new one, with no prompt.
//!
//! Copilot key (#332, ADR 0025): one [`CopilotKeyFlag`] is shared by the key check reads
//! ([`WithCopilotFlag`]) and the Windows hook seam ([`HookConsumers`]). The hook sets it when
//! it first sees Win+Shift+F23, so the next check flags the Copilot key.
//!
//! Requirements: CAP-35a, CAP-36, CAP-37, CAP-39c, CAP-40a, CAP-43a, CAP-43b, MB-06a, AT-30.

use crate::hotkey_settings::{HotkeySettings, MemorySettingsStore, TalkKeySink};
use crate::ipc::{BrainChannel, PermissionRoute};
use mog_core::clock::{MonotonicClock, SystemClock};
use mog_core::error::CoreError;
use mog_core::hotkey::HotkeyService;
use mog_core::hotkey_bindings::TalkBinding;
use mog_core::hotkey_core::{
    self, HotkeyEngine, HotkeyOutput, HotkeySink, PermissionApi, PermissionStatus, PttPhase,
    UserAction,
};
use mog_core::hotkey_windows::{forward_to_engine, HookSignal};
use mog_core::ipc::{events, PermissionKind, WindowClass};
use mog_core::keyboard_env::{self, CopilotKeyFlag, MemoryCopilotKeyFlag, WithCopilotFlag};
use mog_core::types::{TalkKeyReason, TalkKeyState, TalkKeyStatus};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use tauri::{AppHandle, Manager, Runtime};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Receives core-to-brain items. The app uses the brain Channel (`ipc::BrainChannel`).
pub trait BrainSink: Send + Sync {
    /// Deliver one item. `false` if no brain listens.
    fn deliver(&self, event: &str, payload: Value) -> bool;

    /// Deliver one item of a sticky event (`events::STICKY`): a brain that subscribes later
    /// still gets the last one (ADR 0039). `false` if no brain listens now.
    fn deliver_sticky(&self, event: &str, payload: Value) -> bool {
        self.deliver(event, payload)
    }
}

impl<R: Runtime> BrainSink for AppHandle<R> {
    fn deliver(&self, event: &str, payload: Value) -> bool {
        self.try_state::<BrainChannel>()
            .is_some_and(|ch| ch.send(event, payload))
    }

    fn deliver_sticky(&self, event: &str, payload: Value) -> bool {
        self.try_state::<BrainChannel>()
            .is_some_and(|ch| ch.send_sticky(event, payload))
    }
}

/// A talk-key status change for the "Talk key paused" pill (CAP-39c) and the elevated-window
/// tooltip (CAP-42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyStatus {
    /// Secure Event Input is on: show the pill.
    Paused,
    /// The talk key works again (Secure Event Input ended, or the hook is back): hide the pill.
    Restored,
    /// Secure Event Input is on and the OS refused the fallback chord.
    ChordUnavailable,
    /// Windows: an elevated window blocks the talk key (CAP-42).
    Blocked,
    /// The hook or the host could not start the talk key.
    Unavailable,
}

/// The `hotkey.status` payload of a status (ADR 0039). It carries no window title, process
/// name, key or failure text.
pub fn talk_key_status(status: HotkeyStatus) -> TalkKeyStatus {
    let (talk_key, reason) = match status {
        HotkeyStatus::Paused => (TalkKeyState::Paused, Some(TalkKeyReason::SecureInput)),
        HotkeyStatus::Restored => (TalkKeyState::Active, None),
        HotkeyStatus::ChordUnavailable => (
            TalkKeyState::Paused,
            Some(TalkKeyReason::FallbackChordUnavailable),
        ),
        HotkeyStatus::Blocked => (
            TalkKeyState::Paused,
            Some(TalkKeyReason::ElevatedForeground),
        ),
        HotkeyStatus::Unavailable => (TalkKeyState::Unavailable, Some(TalkKeyReason::HookFailed)),
    };
    TalkKeyStatus { talk_key, reason }
}

/// [`talk_key_status`] as JSON.
pub fn status_payload(status: HotkeyStatus) -> Value {
    serde_json::to_value(talk_key_status(status)).unwrap_or(Value::Null)
}

/// Receives the talk-key status.
pub trait StatusSink: Send + Sync {
    /// Take one status change.
    fn status(&self, status: HotkeyStatus);
}

/// The app's status sink: each change goes to the brain as the sticky `hotkey:status` item
/// (#297, ADR 0039), so a brain that subscribes later still gets the last status. The one
/// exception is `elevated_foreground`: a one-shot CAP-42 tooltip notice, sent with plain
/// `deliver` and never kept. It does not change the talk-key state (ADR 0039, decision 4).
#[derive(Debug, Clone)]
pub struct BrainStatus<B: BrainSink>(pub B);

impl<B: BrainSink> StatusSink for BrainStatus<B> {
    fn status(&self, status: HotkeyStatus) {
        let payload = status_payload(status);
        // CAP-42: the elevated-window tooltip shows on the first failure only. R1 sends
        // `Blocked` once per elevated process and no signal ends it, so it is not sticky: a
        // reloaded brain must not show the tooltip again (ADR 0039). Every other status is
        // a state, and the Channel keeps the last one for a brain that subscribes later.
        let _ = if status == HotkeyStatus::Blocked {
            self.0.deliver(events::HOTKEY_STATUS, payload)
        } else {
            self.0.deliver_sticky(events::HOTKEY_STATUS, payload)
        };
    }
}

/// Tell the brain that the talk key cannot work: `hotkey_host::install` failed (ADR 0039).
pub fn report_unavailable<R: Runtime>(app: &AppHandle<R>) {
    BrainStatus(app.clone()).status(HotkeyStatus::Unavailable);
}

/// A window signal that changes the PTT target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSignal {
    /// The window's page finished loading: R11 opened it (or the view reloaded).
    Opened,
    /// The window got focus (the user interacted with it).
    Focused,
    /// The window is gone.
    Destroyed,
}

/// The last-interacted MOG: the target of the talk key (MB-06a, ADR 0058).
///
/// `PttTarget` keeps the open MOGs in the order of their last interaction. The target is the
/// first one. The brain applies the same rules (`src/brain/talk-target.ts`).
/// - The first open of a `mog-<id>` window makes it the target. A reload of an open MOG view
///   is not an interaction, so it does not move the target (#305 review note A).
/// - A `mog-<id>` focus counts as an interaction. MOG windows cannot take focus (MOG-01b), so
///   this is rare.
/// - A `card-<id>` window that opens or gets focus makes its MOG the target, but only while
///   that MOG window is open (#305 review note B).
/// - [`PttTarget::set_active`] (the `mog_set_active` route) makes an open MOG the target: a
///   click, poke or drag end on the MOG, or input from its card.
/// - When the target MOG window closes, the target moves to the most recently interacted MOG
///   that is still open. With none open, there is no target.
#[derive(Debug, Default)]
pub struct PttTarget(Mutex<Vec<String>>);

/// `set_active` refused a MOG whose `mog-<id>` window is not open (R11 `unknown_mog`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownMog;

/// The host side of `mog_set_active`.
///
/// TODO(#439): the `mog_set_active` arm in `src-tauri/src/ipc.rs` calls this. Until then the
/// command answers `not_implemented` and the window signals alone move the target.
pub trait TalkTargetRoute: Send + Sync {
    /// Make the open MOG `mog_id` the talk-key target.
    fn set_active(&self, mog_id: &str) -> Result<(), UnknownMog>;
}

impl PttTarget {
    /// Make `mog_id` the target, and count its window as open.
    pub fn set(&self, mog_id: &str) {
        let mut mru = lock(&self.0);
        mru.retain(|m| m != mog_id);
        mru.insert(0, mog_id.to_owned());
    }

    /// The target now.
    pub fn current(&self) -> Option<String> {
        lock(&self.0).first().cloned()
    }

    /// Make the open MOG `mog_id` the target. An ID whose window is not open is refused.
    pub fn set_active(&self, mog_id: &str) -> Result<(), UnknownMog> {
        let mut mru = lock(&self.0);
        let at = mru.iter().position(|m| m == mog_id).ok_or(UnknownMog)?;
        let id = mru.remove(at);
        mru.insert(0, id);
        Ok(())
    }

    /// Apply a window signal for the window `label`. Only `mog-<id>` and `card-<id>` count.
    pub fn on_window(&self, label: &str, signal: WindowSignal) {
        let Some((class, Some(id))) = WindowClass::of_label(label) else {
            return;
        };
        let mut mru = lock(&self.0);
        let open = mru.iter().position(|m| m == id);
        match (class, signal, open) {
            // A first open, or a focus: the MOG is the target.
            (WindowClass::Mog, WindowSignal::Opened, None)
            | (WindowClass::Mog, WindowSignal::Focused, _) => {
                if let Some(at) = open {
                    mru.remove(at);
                }
                mru.insert(0, id.to_owned());
            }
            // A card of an open MOG.
            (WindowClass::Card, WindowSignal::Opened | WindowSignal::Focused, Some(at)) => {
                let id = mru.remove(at);
                mru.insert(0, id);
            }
            (WindowClass::Mog, WindowSignal::Destroyed, Some(at)) => {
                mru.remove(at);
            }
            _ => {}
        }
    }
}

impl TalkTargetRoute for PttTarget {
    fn set_active(&self, mog_id: &str) -> Result<(), UnknownMog> {
        PttTarget::set_active(self, mog_id)
    }
}

/// The `hotkey:ptt` payload of section 4.5: `{ mogId, phase }`.
pub fn ptt_payload(mog_id: &str, phase: PttPhase) -> Value {
    json!({ "mogId": mog_id, "phase": phase.as_str() })
}

/// Turns R1 outputs into brain items and status changes.
///
/// The target is read at `down` and kept until `up` or `cancel`, so one press goes to one
/// MOG even if the target moves during the hold. With no target, the press sends nothing.
/// A `double_tap` goes to the MOG of the press it follows (CAP-14a).
pub struct PttForwarder<B: BrainSink, S: StatusSink> {
    target: Arc<PttTarget>,
    brain: B,
    status: S,
    held: Option<String>,
    /// The MOG of the last press that ended, for the `double_tap` that follows it.
    last: Option<String>,
}

impl<B: BrainSink, S: StatusSink> PttForwarder<B, S> {
    /// A forwarder that reads the target from `target`.
    pub fn new(target: Arc<PttTarget>, brain: B, status: S) -> Self {
        Self {
            target,
            brain,
            status,
            held: None,
            last: None,
        }
    }

    /// Forward one output.
    pub fn on_output(&mut self, output: HotkeyOutput) {
        match output {
            HotkeyOutput::Ptt(phase) => {
                let mog = match phase {
                    PttPhase::Down => {
                        self.held = self.target.current();
                        self.held.clone()
                    }
                    PttPhase::Up | PttPhase::Cancel => {
                        self.last = self.held.take();
                        self.last.clone()
                    }
                    PttPhase::DoubleTap => self.last.take(),
                };
                if let Some(mog) = mog {
                    // `false` means no brain listens yet; the press is dropped.
                    let _ = self
                        .brain
                        .deliver(events::HOTKEY_PTT, ptt_payload(&mog, phase));
                }
            }
            HotkeyOutput::TalkKeyPaused => self.status.status(HotkeyStatus::Paused),
            HotkeyOutput::TalkKeyRestored => self.status.status(HotkeyStatus::Restored),
            HotkeyOutput::FallbackChordUnavailable => {
                self.status.status(HotkeyStatus::ChordUnavailable)
            }
            // A tap (CAP-44a) comes after `Ptt(Cancel)`, which already ended the PTT and
            // dropped its audio. Section 4.5 has no `tap` phase: the brain reads a short
            // `cancel` as a tap (CAP-14b), and R1 sends `Ptt(DoubleTap)` for two (CAP-14a).
            // TODO(#444): mark that `cancel` with `tap: true` so the brain uses R1's decision.
            HotkeyOutput::Tap => {}
        }
    }
}

/// The R1 sink of the host. It only queues the output, so the tap callback returns fast.
#[derive(Debug, Clone)]
pub struct QueueSink(mpsc::Sender<HotkeyOutput>);

impl HotkeySink for QueueSink {
    fn emit(&self, output: HotkeyOutput) {
        // An error means the forwarder is gone (app quit); nothing to do.
        let _ = self.0.send(output);
    }
}

/// A queue sink and the receiver that the forwarder thread drains.
pub fn queue() -> (QueueSink, mpsc::Receiver<HotkeyOutput>) {
    let (tx, rx) = mpsc::channel();
    (QueueSink(tx), rx)
}

/// Start the forwarder thread. It ends when every sender is gone.
pub fn spawn_forwarder<B, S>(
    rx: mpsc::Receiver<HotkeyOutput>,
    mut forwarder: PttForwarder<B, S>,
) -> std::io::Result<JoinHandle<()>>
where
    B: BrainSink + 'static,
    S: StatusSink + 'static,
{
    std::thread::Builder::new()
        .name("bottle-hotkey-forward".into())
        .spawn(move || {
            for output in rx {
                forwarder.on_output(output);
            }
        })
}

/// A Windows hook signal that has no consumer in the host yet (#314).
///
/// Each kind has its own consumer issue, so the adapter never drops a signal in silence.
/// No kind carries the identity of a key (SEC rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferredSignal {
    /// Esc went down during capture. It ends hands-free capture (CAP-14b, #434).
    Escape,
    /// An elevated window blocks the talk key. It becomes `hotkey.status` for the CAP-42
    /// tooltip (#294).
    TalkKeyBlocked,
    /// The hook was installed again (CAP-40b).
    HookReinstalled,
    /// A hook install failed. The reason is a fixed, content-free string.
    HookFailed(&'static str),
    /// The hook saw the Copilot key (CAP-43b, #332). [`HookConsumers`] sets the flag.
    CopilotKeySeen,
}

/// Receives the [`DeferredSignal`]s of the Windows hook.
pub trait HookSignalSeam: Send {
    /// Take one deferred signal.
    fn deferred(&self, signal: DeferredSignal);
}

/// The app's seam. The Copilot signal sets the shared flag; the talk-key signals go to the
/// status sink as `hotkey.status`; each other arm names the issue that consumes it.
pub struct HookConsumers {
    copilot: Arc<dyn CopilotKeyFlag>,
    status: Option<Arc<dyn StatusSink>>,
}

impl HookConsumers {
    /// A seam that sets `copilot` when the hook sees the Copilot key.
    pub fn new(copilot: Arc<dyn CopilotKeyFlag>) -> Self {
        Self {
            copilot,
            status: None,
        }
    }

    /// The same seam, with `status` for the talk-key signals (CAP-42, CAP-40a).
    pub fn with_status(self, status: Arc<dyn StatusSink>) -> Self {
        Self {
            status: Some(status),
            ..self
        }
    }

    fn status(&self, status: HotkeyStatus) {
        if let Some(sink) = &self.status {
            sink.status(status);
        }
    }
}

impl Default for HookConsumers {
    /// A seam with its own flag in memory (tests and platforms with no key check).
    fn default() -> Self {
        Self::new(Arc::new(MemoryCopilotKeyFlag::default()))
    }
}

impl HookSignalSeam for HookConsumers {
    fn deferred(&self, signal: DeferredSignal) {
        match signal {
            DeferredSignal::CopilotKeySeen => {
                // The next key check flags the Copilot key: `hotkeys_set` returns the
                // conflict for Settings (#77) and the wizard (#43).
                if self.copilot.set(true) {
                    // TODO(#346): on this change, show the conflict of the current keys.
                }
            }
            DeferredSignal::Escape => {
                // TODO(#434): the host calls `CaptureCore::stop_hands_free(Escape)` (with #425).
            }
            // CAP-42: the core sends it once per elevated process; the MOG shows the tooltip.
            DeferredSignal::TalkKeyBlocked => self.status(HotkeyStatus::Blocked),
            DeferredSignal::HookReinstalled => self.status(HotkeyStatus::Restored),
            // The fixed reason string stays here: the payload carries only `hook_failed`.
            DeferredSignal::HookFailed(_) => self.status(HotkeyStatus::Unavailable),
        }
    }
}

/// Map one Windows hook signal (#7) to the shared R1 engine (#6).
///
/// `TalkDown` and `TalkUp` go through [`forward_to_engine`] to the shared PTT machine, so
/// Windows gives the same `hotkey.ptt` phases as macOS. `OtherKeyDown` goes to
/// `HotkeyEngine::on_other_key_down`, which cancels a PTT in its first 1 s (CAP-44c). The
/// other signals return as a [`DeferredSignal`] for the seam.
pub fn adapt_hook_signal(signal: HookSignal, engine: &mut HotkeyEngine) -> Option<DeferredSignal> {
    Some(match forward_to_engine(signal, engine)? {
        HookSignal::Escape => DeferredSignal::Escape,
        HookSignal::TalkKeyBlocked => DeferredSignal::TalkKeyBlocked,
        HookSignal::HookReinstalled => DeferredSignal::HookReinstalled,
        HookSignal::HookFailed { reason } => DeferredSignal::HookFailed(reason),
        HookSignal::CopilotKeySeen => DeferredSignal::CopilotKeySeen,
        // `forward_to_engine` takes the talk-key signals and the other-key signal.
        HookSignal::TalkDown { .. } | HookSignal::TalkUp { .. } | HookSignal::OtherKeyDown => {
            return None
        }
    })
}

/// Drain `rx` until the service drops its sender.
pub fn run_hook_adapter(
    rx: mpsc::Receiver<HookSignal>,
    mut engine: HotkeyEngine,
    seam: &dyn HookSignalSeam,
) {
    for signal in rx {
        if let Some(deferred) = adapt_hook_signal(signal, &mut engine) {
            seam.deferred(deferred);
        }
    }
    // The hook stopped (quit or a talk-key restart) with no key-up: end a held PTT with
    // `cancel`, so no capture stays open (CAP-45).
    engine.reset();
}

/// Start the adapter thread. The engine sends its outputs to `sink` (the host [`QueueSink`]).
pub fn spawn_hook_adapter<D>(
    rx: mpsc::Receiver<HookSignal>,
    sink: Arc<dyn HotkeySink>,
    seam: D,
) -> std::io::Result<JoinHandle<()>>
where
    D: HookSignalSeam + 'static,
{
    spawn_hook_adapter_with_clock(rx, sink, seam, Arc::new(SystemClock::new()))
}

/// [`spawn_hook_adapter`] with the clock that times the holds (CAP-44a, CAP-44c). Tests
/// pass a simulated clock.
pub fn spawn_hook_adapter_with_clock<D>(
    rx: mpsc::Receiver<HookSignal>,
    sink: Arc<dyn HotkeySink>,
    seam: D,
    clock: Arc<dyn MonotonicClock>,
) -> std::io::Result<JoinHandle<()>>
where
    D: HookSignalSeam + 'static,
{
    std::thread::Builder::new()
        .name("bottle-hotkey-signals".into())
        .spawn(move || run_hook_adapter(rx, HotkeyEngine::with_clock(sink, clock), &seam))
}

/// The service to give the host, given the result of [`spawn_hook_adapter`].
/// With no adapter thread the talk key cannot work, so the host gets a service whose `start`
/// reports `hook_adapter_spawn_failed` and the real hook is never installed.
pub fn with_hook_adapter(
    service: Box<dyn HotkeyService>,
    adapter: std::io::Result<JoinHandle<()>>,
) -> Box<dyn HotkeyService> {
    match adapter {
        // The thread is detached; it ends when the service drops its sender.
        Ok(_) => service,
        Err(_) => Box::new(AdapterFailed),
    }
}

/// The service when the adapter thread did not start.
#[derive(Debug, Clone, Copy)]
struct AdapterFailed;

impl HotkeyService for AdapterFailed {
    fn start(&self) -> mog_core::error::CoreResult<()> {
        Err(CoreError::Unavailable {
            component: "R1",
            reason: "hook_adapter_spawn_failed",
        })
    }
    fn stop(&self) -> mog_core::error::CoreResult<()> {
        Ok(())
    }
    fn secure_input_active(&self) -> mog_core::error::CoreResult<bool> {
        Ok(false)
    }
}

/// The state of the host's HotkeyService.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostState {
    /// Not started, or stopped on quit.
    Stopped,
    /// The talk key works.
    Running,
    /// Input Monitoring is missing. The wizard or Settings asks after a click.
    NeedsPermission,
    /// The service cannot run here. The code is content-free (for example `not_implemented`).
    Unavailable(&'static str),
}

/// Owns the HotkeyService and the permission API of the platform.
pub struct HotkeyHost {
    service: Mutex<Box<dyn HotkeyService>>,
    permissions: Box<dyn PermissionApi>,
    state: Mutex<HostState>,
}

impl HotkeyHost {
    /// A stopped host.
    pub fn new(service: Box<dyn HotkeyService>, permissions: Box<dyn PermissionApi>) -> Self {
        Self {
            service: Mutex::new(service),
            permissions,
            state: Mutex::new(HostState::Stopped),
        }
    }

    /// The state now.
    pub fn state(&self) -> HostState {
        *lock(&self.state)
    }

    /// Start the service. This never prompts (CAP-36): without Input Monitoring the state is
    /// [`HostState::NeedsPermission`] until [`HotkeyHost::request_ptt_permission`].
    pub fn start(&self) -> HostState {
        let mut state = lock(&self.state);
        *state = start_state(lock(&self.service).as_ref());
        *state
    }

    /// Ask for Input Monitoring after a user click, then start again if granted (CAP-36).
    /// The wizard (#43) and Settings call this with the `permission_request` command (#312).
    pub fn request_ptt_permission(&self, action: UserAction) -> PermissionStatus {
        let status = hotkey_core::request_ptt_permission(self.permissions.as_ref(), action);
        if status == PermissionStatus::Granted {
            self.start();
        }
        status
    }

    /// Ask for Accessibility after the user selects text insertion, and only then (CAP-37).
    /// The hub calls this with `permission_request` `accessibility` (#312).
    pub fn request_insertion_permission(&self, action: UserAction) -> PermissionStatus {
        hotkey_core::request_insertion_permission(self.permissions.as_ref(), action)
    }

    /// Stop the service (app quit).
    pub fn stop(&self) {
        let mut state = lock(&self.state);
        // A stop error leaves nothing to undo at quit.
        let _ = lock(&self.service).stop();
        *state = HostState::Stopped;
    }

    /// Use `service` from now on (a new talk key, CAP-43a). The old service stops. The new
    /// one starts with no prompt (CAP-36), unless the host was stopped on quit. If the old
    /// service does not stop, it stays, the state does not change, and the result is
    /// [`HostState::Unavailable`] with the stop code.
    pub fn replace_service(&self, service: Box<dyn HotkeyService>) -> HostState {
        let mut state = lock(&self.state);
        let mut current = lock(&self.service);
        // If the old service does not stop (Windows `stop_post_failed`), its hook may still
        // run. Keep it, so two hooks never run, and do not start the new one.
        if let Err(e) = current.stop() {
            return HostState::Unavailable(e.code());
        }
        *current = service;
        if *state != HostState::Stopped {
            *state = start_state(current.as_ref());
        }
        *state
    }
}

/// Start `service` with no prompt and map the result to a [`HostState`].
fn start_state(service: &dyn HotkeyService) -> HostState {
    match service.start() {
        Ok(()) => HostState::Running,
        Err(CoreError::Unavailable {
            reason: "input_monitoring_required",
            ..
        }) => HostState::NeedsPermission,
        Err(e) => HostState::Unavailable(e.code()),
    }
}

/// Makes the platform service for a talk key, with the host sink inside.
type MakeService = Box<dyn Fn(TalkBinding) -> Box<dyn HotkeyService> + Send + Sync>;

/// The host state that the app manages.
pub struct HotkeyRuntime {
    /// The service owner.
    pub host: HotkeyHost,
    /// The talk-key target.
    pub target: Arc<PttTarget>,
    make_service: MakeService,
}

impl HotkeyRuntime {
    /// Restart the talk key with `talk` (a new `app.hotkeys.ptt`, CAP-43a). No prompt.
    pub fn set_talk_key(&self, talk: TalkBinding) -> HostState {
        self.host.replace_service((self.make_service)(talk))
    }
}

/// `permission_request` (#312, ADR 0039): Input Monitoring starts R1 again when granted.
impl PermissionRoute for HotkeyRuntime {
    fn request_permission(
        &self,
        permission: PermissionKind,
        action: UserAction,
    ) -> PermissionStatus {
        match permission {
            PermissionKind::InputMonitoring => self.host.request_ptt_permission(action),
            PermissionKind::Accessibility => self.host.request_insertion_permission(action),
        }
    }
}

/// The app's [`TalkKeySink`]: a new talk key restarts the managed [`HotkeyRuntime`].
pub struct AppTalkKey<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> TalkKeySink for AppTalkKey<R> {
    fn apply(&self, talk: TalkBinding) {
        if let Some(rt) = self.0.try_state::<HotkeyRuntime>() {
            rt.set_talk_key(talk);
        }
    }
}

/// The platform service and permission API (macOS).
/// macOS has no Copilot key and no hook seam, so `_copilot` and `_status` are unused: the
/// pill outputs reach the status sink through the forwarder.
#[cfg(target_os = "macos")]
pub fn platform_service(
    talk: TalkBinding,
    sink: Arc<dyn HotkeySink>,
    _copilot: Arc<dyn CopilotKeyFlag>,
    _status: Arc<dyn StatusSink>,
) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>) {
    use mog_core::hotkey_macos::{MacHotkeyService, MacPermissions};
    // Ctrl+Win is a Windows key: macOS settings never hold it, so it gives the D66 default.
    let talk_key = talk.core_talk_key().unwrap_or_default();
    (
        Box::new(MacHotkeyService::new(talk_key, sink)),
        Box::new(MacPermissions),
    )
}

/// The platform service and permission API (Windows, #7 and #314).
///
/// The hook sends [`HookSignal`]s on its own channel; the adapter thread feeds them to the
/// shared engine, which emits into `sink`. The hook needs no OS prompt. A Copilot chord
/// seen by the hook sets `copilot` (#332). The talk-key signals go to `status` (ADR 0039).
#[cfg(windows)]
pub fn platform_service(
    talk: TalkBinding,
    sink: Arc<dyn HotkeySink>,
    copilot: Arc<dyn CopilotKeyFlag>,
    status: Arc<dyn StatusSink>,
) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>) {
    use mog_core::hotkey_windows::win32::WindowsHotkeyService;
    let (service, rx) = WindowsHotkeyService::new(talk.windows_talk_key());
    let seam = HookConsumers::new(copilot).with_status(status);
    let adapter = spawn_hook_adapter(rx, sink, seam);
    (
        with_hook_adapter(Box::new(service), adapter),
        Box::new(NoOsPermission),
    )
}

/// The platform service and permission API (no hook on this OS).
#[cfg(not(any(target_os = "macos", windows)))]
pub fn platform_service(
    _talk: TalkBinding,
    _sink: Arc<dyn HotkeySink>,
    _copilot: Arc<dyn CopilotKeyFlag>,
    _status: Arc<dyn StatusSink>,
) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>) {
    (
        Box::new(mog_core::hotkey::StubHotkeyService),
        Box::new(NoOsPermission),
    )
}

/// The permission API of an OS that has no PTT permission prompt (Windows has no prompt for
/// a `WH_KEYBOARD_LL` hook). It never opens an OS prompt.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOsPermission;

impl PermissionApi for NoOsPermission {
    fn status(&self, _permission: hotkey_core::Permission) -> PermissionStatus {
        PermissionStatus::Granted
    }
    fn request(&self, _permission: hotkey_core::Permission) -> PermissionStatus {
        PermissionStatus::Granted
    }
}

/// Load the key settings, create the host with the platform service and start it (no
/// prompt). The settings store is in memory until #77 (see `hotkey_settings`). One Copilot
/// key flag goes to the key check reads and to the hook (#332, ADR 0025); it is in memory
/// until #341. The Windows hook seam sends the talk-key status to the brain (ADR 0039).
pub fn install<R: Runtime>(app: &AppHandle<R>) -> std::io::Result<()> {
    let copilot: Arc<dyn CopilotKeyFlag> = Arc::new(MemoryCopilotKeyFlag::default());
    let status: Arc<dyn StatusSink> = Arc::new(BrainStatus(app.clone()));
    let settings = HotkeySettings::load(
        keyboard_env::host_os(),
        Box::new(MemorySettingsStore::default()),
        Box::new(WithCopilotFlag::new(
            keyboard_env::os_env_reads(),
            Arc::clone(&copilot),
        )),
        Box::new(AppTalkKey(app.clone())),
    );
    install_with(app, settings, move |talk, sink| {
        platform_service(talk, sink, Arc::clone(&copilot), Arc::clone(&status))
    })
}

/// Create the host with `factory` and the saved talk key of `settings`, start the forwarder
/// thread and start the service. The app then manages `settings` and the runtime.
pub fn install_with<R, F>(
    app: &AppHandle<R>,
    settings: HotkeySettings,
    factory: F,
) -> std::io::Result<()>
where
    R: Runtime,
    F: Fn(TalkBinding, Arc<dyn HotkeySink>) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>)
        + Send
        + Sync
        + 'static,
{
    let target = Arc::new(PttTarget::default());
    let (sink, rx) = queue();
    spawn_forwarder(
        rx,
        PttForwarder::new(Arc::clone(&target), app.clone(), BrainStatus(app.clone())),
    )?;
    let sink: Arc<dyn HotkeySink> = Arc::new(sink);
    let (service, permissions) = factory(settings.talk_binding(), Arc::clone(&sink));
    let host = HotkeyHost::new(service, permissions);
    host.start();
    // A later talk key keeps the permission API and makes only a new service.
    let make_service: MakeService = Box::new(move |talk| factory(talk, Arc::clone(&sink)).0);
    app.manage(HotkeyRuntime {
        host,
        target,
        make_service,
    });
    app.manage(settings);
    Ok(())
}

/// The target signal of a window event, if any.
pub fn signal_of_window_event(event: &tauri::WindowEvent) -> Option<WindowSignal> {
    match event {
        tauri::WindowEvent::Focused(true) => Some(WindowSignal::Focused),
        tauri::WindowEvent::Destroyed => Some(WindowSignal::Destroyed),
        _ => None,
    }
}

/// The target signal of a page load, if any.
pub fn signal_of_page_load(event: tauri::webview::PageLoadEvent) -> Option<WindowSignal> {
    match event {
        tauri::webview::PageLoadEvent::Finished => Some(WindowSignal::Opened),
        tauri::webview::PageLoadEvent::Started => None,
    }
}

/// Apply `signal` for the window `label` to the talk-key target.
pub fn apply_signal<R: Runtime>(app: &AppHandle<R>, label: &str, signal: Option<WindowSignal>) {
    if let (Some(signal), Some(rt)) = (signal, app.try_state::<HotkeyRuntime>()) {
        rt.target.on_window(label, signal);
    }
}

/// Update the talk-key target from a window event (`Builder::on_window_event`).
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    apply_signal(
        window.app_handle(),
        window.label(),
        signal_of_window_event(event),
    );
}

/// Update the talk-key target when a view finishes loading (`Builder::on_page_load`).
pub fn on_page_load<R: Runtime>(
    webview: &tauri::Webview<R>,
    payload: &tauri::webview::PageLoadPayload<'_>,
) {
    apply_signal(
        webview.app_handle(),
        webview.label(),
        signal_of_page_load(payload.event()),
    );
}

/// Stop the service on app quit.
pub fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    if let Some(rt) = app.try_state::<HotkeyRuntime>() {
        rt.host.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mog_core::error::{CoreError, CoreResult};
    use mog_core::hotkey_core::Permission;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default, Clone)]
    struct FakeBrain(Arc<Mutex<Vec<(String, Value)>>>);

    impl BrainSink for FakeBrain {
        fn deliver(&self, event: &str, payload: Value) -> bool {
            self.0.lock().unwrap().push((event.to_owned(), payload));
            true
        }
    }

    impl FakeBrain {
        fn items(&self) -> Vec<(String, Value)> {
            self.0.lock().unwrap().clone()
        }
    }

    #[derive(Default, Clone)]
    struct FakeStatus(Arc<Mutex<Vec<HotkeyStatus>>>);

    impl StatusSink for FakeStatus {
        fn status(&self, status: HotkeyStatus) {
            self.0.lock().unwrap().push(status);
        }
    }

    fn forwarder(
        target: &Arc<PttTarget>,
    ) -> (PttForwarder<FakeBrain, FakeStatus>, FakeBrain, FakeStatus) {
        let brain = FakeBrain::default();
        let status = FakeStatus::default();
        (
            PttForwarder::new(Arc::clone(target), brain.clone(), status.clone()),
            brain,
            status,
        )
    }

    fn ptt(id: &str, phase: &str) -> (String, Value) {
        (
            "hotkey:ptt".to_owned(),
            json!({ "mogId": id, "phase": phase }),
        )
    }

    #[test]
    fn cap_35a_mb_06a_ptt_down_and_up_reach_the_brain_for_the_target() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let (mut f, brain, _) = forwarder(&target);
        f.on_output(HotkeyOutput::Ptt(PttPhase::Down));
        f.on_output(HotkeyOutput::Ptt(PttPhase::Up));
        assert_eq!(brain.items(), vec![ptt("a1", "down"), ptt("a1", "up")]);
    }

    #[test]
    fn mb_06a_ptt_payload_is_mog_id_and_phase() {
        assert_eq!(
            ptt_payload("a1", PttPhase::Cancel),
            json!({ "mogId": "a1", "phase": "cancel" })
        );
        assert_eq!(mog_core::ipc::events::HOTKEY_PTT, "hotkey:ptt");
    }

    #[test]
    fn mb_06a_up_and_cancel_go_to_the_mog_that_got_down() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let (mut f, brain, _) = forwarder(&target);
        f.on_output(HotkeyOutput::Ptt(PttPhase::Down));
        target.set("b2");
        f.on_output(HotkeyOutput::Ptt(PttPhase::Cancel));
        f.on_output(HotkeyOutput::Ptt(PttPhase::Down));
        f.on_output(HotkeyOutput::Ptt(PttPhase::Up));
        assert_eq!(
            brain.items(),
            vec![
                ptt("a1", "down"),
                ptt("a1", "cancel"),
                ptt("b2", "down"),
                ptt("b2", "up")
            ]
        );
    }

    #[test]
    fn cap_14a_double_tap_goes_to_the_mog_of_the_two_taps() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let (mut f, brain, _) = forwarder(&target);
        for _ in 0..2 {
            f.on_output(HotkeyOutput::Ptt(PttPhase::Down));
            f.on_output(HotkeyOutput::Ptt(PttPhase::Cancel));
            f.on_output(HotkeyOutput::Tap);
        }
        // The target changes after the taps: the double-tap stays with the tapped MOG.
        target.set("b2");
        f.on_output(HotkeyOutput::Ptt(PttPhase::DoubleTap));
        let items = brain.items();
        assert_eq!(items.last(), Some(&ptt("a1", "double_tap")));
        assert_eq!(items.len(), 5);
    }

    #[test]
    fn mb_06a_no_target_means_no_ptt_event() {
        let target = Arc::new(PttTarget::default());
        let (mut f, brain, _) = forwarder(&target);
        f.on_output(HotkeyOutput::Ptt(PttPhase::Down));
        // A target that shows up during the hold does not get a lone `up`.
        target.set("a1");
        f.on_output(HotkeyOutput::Ptt(PttPhase::Up));
        assert!(brain.items().is_empty());
    }

    #[test]
    fn mb_06a_a_focused_mog_window_becomes_the_target() {
        let target = PttTarget::default();
        target.on_window("mog-a1", WindowSignal::Focused);
        assert_eq!(target.current().as_deref(), Some("a1"));
        // The hub, the brain and bad labels do not change the target (cards: see below).
        for label in ["hub", "brain", "card-", "mog-", "mog-a b", "other"] {
            target.on_window(label, WindowSignal::Focused);
        }
        assert_eq!(target.current().as_deref(), Some("a1"));
        target.on_window("mog-b2", WindowSignal::Focused);
        assert_eq!(target.current().as_deref(), Some("b2"));
    }

    #[test]
    fn mb_06a_a_destroyed_target_window_clears_the_target() {
        let target = PttTarget::default();
        target.on_window("mog-a1", WindowSignal::Focused);
        target.on_window("mog-b2", WindowSignal::Destroyed);
        assert_eq!(target.current().as_deref(), Some("a1"));
        target.on_window("mog-a1", WindowSignal::Destroyed);
        assert_eq!(target.current(), None);
    }

    #[test]
    fn cap_39c_status_outputs_go_to_the_status_seam_not_the_brain() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let (mut f, brain, status) = forwarder(&target);
        f.on_output(HotkeyOutput::TalkKeyPaused);
        f.on_output(HotkeyOutput::FallbackChordUnavailable);
        f.on_output(HotkeyOutput::TalkKeyRestored);
        assert!(brain.items().is_empty());
        assert_eq!(
            *status.0.lock().unwrap(),
            vec![
                HotkeyStatus::Paused,
                HotkeyStatus::ChordUnavailable,
                HotkeyStatus::Restored
            ]
        );
    }

    #[test]
    fn cap_35a_at_30_sink_queue_and_thread_deliver_in_order() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let (f, brain, _) = forwarder(&target);
        let (sink, rx) = queue();
        let join = spawn_forwarder(rx, f).unwrap();
        for _ in 0..100 {
            sink.emit(HotkeyOutput::Ptt(PttPhase::Down));
            sink.emit(HotkeyOutput::Ptt(PttPhase::Up));
        }
        drop(sink);
        join.join().unwrap();
        let items = brain.items();
        assert_eq!(items.len(), 200);
        for pair in items.chunks(2) {
            assert_eq!(pair, [ptt("a1", "down"), ptt("a1", "up")]);
        }
    }

    /// A HotkeyService double: start fails until `granted` is set.
    #[derive(Default, Clone)]
    struct FakeService {
        granted: Arc<std::sync::atomic::AtomicBool>,
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
    }

    impl HotkeyService for FakeService {
        fn start(&self) -> CoreResult<()> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            if self.granted.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(CoreError::Unavailable {
                    component: "R1",
                    reason: "input_monitoring_required",
                })
            }
        }
        fn stop(&self) -> CoreResult<()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn secure_input_active(&self) -> CoreResult<bool> {
            Ok(false)
        }
    }

    /// A PermissionApi double that counts prompts. A prompt grants when `grant` is true.
    #[derive(Default, Clone)]
    struct FakePermissions {
        service: FakeService,
        grant: bool,
        prompts: Arc<Mutex<Vec<Permission>>>,
    }

    impl PermissionApi for FakePermissions {
        fn status(&self, _p: Permission) -> PermissionStatus {
            PermissionStatus::Unknown
        }
        fn request(&self, p: Permission) -> PermissionStatus {
            self.prompts.lock().unwrap().push(p);
            if self.grant {
                if p == Permission::InputMonitoring {
                    self.service.granted.store(true, Ordering::SeqCst);
                }
                PermissionStatus::Granted
            } else {
                PermissionStatus::Denied
            }
        }
    }

    fn host(grant: bool) -> (HotkeyHost, FakeService, FakePermissions) {
        let service = FakeService::default();
        let perms = FakePermissions {
            service: service.clone(),
            grant,
            prompts: Arc::default(),
        };
        (
            HotkeyHost::new(Box::new(service.clone()), Box::new(perms.clone())),
            service,
            perms,
        )
    }

    #[test]
    fn cap_36_start_without_input_monitoring_never_prompts() {
        let (h, service, perms) = host(true);
        assert_eq!(h.start(), HostState::NeedsPermission);
        assert_eq!(h.state(), HostState::NeedsPermission);
        assert_eq!(service.starts.load(Ordering::SeqCst), 1);
        assert!(perms.prompts.lock().unwrap().is_empty());
    }

    #[test]
    fn cap_36_ptt_request_after_a_click_asks_input_monitoring_only_and_starts_again() {
        let (h, service, perms) = host(true);
        h.start();
        assert_eq!(
            h.request_ptt_permission(UserAction::from_user_click()),
            PermissionStatus::Granted
        );
        assert_eq!(
            *perms.prompts.lock().unwrap(),
            vec![Permission::InputMonitoring]
        );
        assert_eq!(service.starts.load(Ordering::SeqCst), 2);
        assert_eq!(h.state(), HostState::Running);
    }

    #[test]
    fn cap_36_a_denied_request_does_not_start_again() {
        let (h, service, _) = host(false);
        h.start();
        assert_eq!(
            h.request_ptt_permission(UserAction::from_user_click()),
            PermissionStatus::Denied
        );
        assert_eq!(service.starts.load(Ordering::SeqCst), 1);
        assert_eq!(h.state(), HostState::NeedsPermission);
    }

    #[test]
    fn cap_37_insertion_request_asks_accessibility_only() {
        let (h, service, perms) = host(true);
        h.start();
        assert_eq!(
            h.request_insertion_permission(UserAction::from_user_click()),
            PermissionStatus::Granted
        );
        assert_eq!(
            *perms.prompts.lock().unwrap(),
            vec![Permission::Accessibility]
        );
        // Accessibility does not start PTT.
        assert_eq!(service.starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn at_30_stop_on_quit_stops_the_service() {
        let (h, service, perms) = host(true);
        h.request_ptt_permission(UserAction::from_user_click());
        assert_eq!(h.state(), HostState::Running);
        h.stop();
        assert_eq!(service.stops.load(Ordering::SeqCst), 1);
        assert_eq!(h.state(), HostState::Stopped);
        assert_eq!(perms.prompts.lock().unwrap().len(), 1);
    }

    #[test]
    fn at_30_an_unsupported_platform_reports_its_code() {
        let h = HotkeyHost::new(
            Box::new(mog_core::hotkey::StubHotkeyService),
            Box::new(FakePermissions::default()),
        );
        assert_eq!(h.start(), HostState::Unavailable("not_implemented"));
    }

    #[test]
    fn at_30_install_starts_the_service_from_the_factory_with_no_prompt() {
        let app = tauri::test::mock_app();
        let service = FakeService::default();
        let perms = FakePermissions {
            service: service.clone(),
            grant: true,
            prompts: Arc::default(),
        };
        let (s2, p2) = (service.clone(), perms.clone());
        let keys = Arc::new(Mutex::new(Vec::new()));
        let k2 = Arc::clone(&keys);
        install_with(app.handle(), memory_settings(), move |key, _sink| {
            k2.lock().unwrap().push(key);
            (
                Box::new(s2.clone()) as Box<dyn HotkeyService>,
                Box::new(p2.clone()) as Box<dyn PermissionApi>,
            )
        })
        .unwrap();
        assert_eq!(*keys.lock().unwrap(), vec![TalkBinding::RightOption]);
        assert_eq!(service.starts.load(Ordering::SeqCst), 1);
        assert!(perms.prompts.lock().unwrap().is_empty());
        shutdown(app.handle());
        assert_eq!(service.stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn mb_06a_an_opened_mog_window_becomes_the_target() {
        // MOG windows are not focusable (MOG-01b), so opening one must set the target.
        let target = PttTarget::default();
        target.on_window("mog-a1", WindowSignal::Opened);
        assert_eq!(target.current().as_deref(), Some("a1"));
        target.on_window("mog-b2", WindowSignal::Opened);
        assert_eq!(target.current().as_deref(), Some("b2"));
    }

    #[test]
    fn mb_06a_a_card_open_or_focus_targets_its_mog() {
        let target = PttTarget::default();
        target.on_window("mog-a1", WindowSignal::Opened);
        target.on_window("mog-b2", WindowSignal::Opened);
        target.on_window("card-a1", WindowSignal::Focused);
        assert_eq!(target.current().as_deref(), Some("a1"));
        target.on_window("card-b2", WindowSignal::Opened);
        assert_eq!(target.current().as_deref(), Some("b2"));
        // A closed card does not clear its MOG.
        target.on_window("card-b2", WindowSignal::Destroyed);
        assert_eq!(target.current().as_deref(), Some("b2"));
    }

    #[test]
    fn mb_06a_window_events_and_page_loads_map_to_signals() {
        use tauri::webview::PageLoadEvent;
        use tauri::WindowEvent;
        assert_eq!(
            signal_of_window_event(&WindowEvent::Focused(true)),
            Some(WindowSignal::Focused)
        );
        assert_eq!(
            signal_of_window_event(&WindowEvent::Destroyed),
            Some(WindowSignal::Destroyed)
        );
        assert_eq!(signal_of_window_event(&WindowEvent::Focused(false)), None);
        assert_eq!(
            signal_of_page_load(PageLoadEvent::Finished),
            Some(WindowSignal::Opened)
        );
        assert_eq!(signal_of_page_load(PageLoadEvent::Started), None);
    }

    #[test]
    fn mb_06a_a_real_mog_window_load_and_close_move_the_target() {
        // The real path: install, a `mog-a1` webview window, the page-load and window hooks.
        let app = tauri::test::mock_app();
        install_with(app.handle(), memory_settings(), |_key, _sink| {
            (
                Box::new(FakeService::default()) as Box<dyn HotkeyService>,
                Box::new(FakePermissions::default()) as Box<dyn PermissionApi>,
            )
        })
        .unwrap();
        let win = tauri::WebviewWindowBuilder::new(&app, "mog-a1", tauri::WebviewUrl::default())
            .build()
            .unwrap();
        apply_signal(
            app.handle(),
            win.label(),
            signal_of_page_load(tauri::webview::PageLoadEvent::Finished),
        );
        let rt = app.state::<HotkeyRuntime>();
        assert_eq!(rt.target.current().as_deref(), Some("a1"));
        on_window_event(&win.as_ref().window(), &tauri::WindowEvent::Destroyed);
        assert_eq!(rt.target.current(), None);
    }

    // ---------- #73: the last-interacted MOG (MB-06a, ADR 0058) ----------

    #[test]
    fn mb_06a_a_reloaded_mog_view_keeps_the_target() {
        // #305 review note A: a reload is not an interaction with that MOG.
        let target = PttTarget::default();
        target.on_window("mog-a1", WindowSignal::Opened);
        target.on_window("mog-b2", WindowSignal::Opened);
        target.on_window("card-a1", WindowSignal::Focused);
        assert_eq!(target.current().as_deref(), Some("a1"));
        target.on_window("mog-b2", WindowSignal::Opened);
        assert_eq!(target.current().as_deref(), Some("a1"));
    }

    #[test]
    fn mb_06a_a_closed_target_falls_back_to_the_last_interacted_open_mog() {
        // #305 review note A: other MOGs are open, so the talk key still has a target.
        let target = PttTarget::default();
        for id in ["mog-a1", "mog-b2", "mog-c3"] {
            target.on_window(id, WindowSignal::Opened);
        }
        target.on_window("card-a1", WindowSignal::Focused);
        target.on_window("card-c3", WindowSignal::Focused);
        target.on_window("mog-c3", WindowSignal::Destroyed);
        assert_eq!(target.current().as_deref(), Some("a1"));
        target.on_window("mog-a1", WindowSignal::Destroyed);
        assert_eq!(target.current().as_deref(), Some("b2"));
        target.on_window("mog-b2", WindowSignal::Destroyed);
        assert_eq!(target.current(), None);
        // A MOG that opens again is a first open.
        target.on_window("mog-a1", WindowSignal::Opened);
        assert_eq!(target.current().as_deref(), Some("a1"));
    }

    #[test]
    fn mb_06a_a_card_of_a_closed_mog_never_becomes_the_target() {
        // #305 review note B: the card of a MOG whose window is closed.
        let target = PttTarget::default();
        target.on_window("mog-a1", WindowSignal::Opened);
        target.on_window("card-z9", WindowSignal::Opened);
        target.on_window("card-z9", WindowSignal::Focused);
        assert_eq!(target.current().as_deref(), Some("a1"));
        target.on_window("mog-a1", WindowSignal::Destroyed);
        target.on_window("card-a1", WindowSignal::Focused);
        assert_eq!(target.current(), None);
    }

    #[test]
    fn mb_06a_set_active_targets_an_open_mog_and_refuses_an_unknown_one() {
        // The `mog_set_active` route of #439 calls this (click, drag, card open, chord use).
        let target = PttTarget::default();
        let route: &dyn TalkTargetRoute = &target;
        assert_eq!(route.set_active("a1"), Err(UnknownMog));
        assert_eq!(target.current(), None);
        target.on_window("mog-a1", WindowSignal::Opened);
        target.on_window("mog-b2", WindowSignal::Opened);
        assert_eq!(route.set_active("a1"), Ok(()));
        assert_eq!(target.current().as_deref(), Some("a1"));
        assert_eq!(route.set_active("z9"), Err(UnknownMog));
        assert_eq!(route.set_active(""), Err(UnknownMog));
        assert_eq!(target.current().as_deref(), Some("a1"));
    }

    #[test]
    fn mb_06a_set_active_moves_the_mog_to_the_front_of_the_fallback_order() {
        let target = PttTarget::default();
        for id in ["mog-a1", "mog-b2", "mog-c3"] {
            target.on_window(id, WindowSignal::Opened);
        }
        target.set_active("a1").unwrap();
        target.set_active("c3").unwrap();
        target.on_window("mog-c3", WindowSignal::Destroyed);
        assert_eq!(target.current().as_deref(), Some("a1"));
    }

    #[test]
    fn mb_06a_a_press_after_set_active_goes_to_the_new_target() {
        let target = Arc::new(PttTarget::default());
        target.on_window("mog-a1", WindowSignal::Opened);
        target.on_window("mog-b2", WindowSignal::Opened);
        let (mut f, brain, _) = forwarder(&target);
        target.set_active("a1").unwrap();
        f.on_output(HotkeyOutput::Ptt(PttPhase::Down));
        f.on_output(HotkeyOutput::Ptt(PttPhase::Up));
        target.set_active("b2").unwrap();
        f.on_output(HotkeyOutput::Ptt(PttPhase::Down));
        f.on_output(HotkeyOutput::Ptt(PttPhase::Up));
        assert_eq!(
            brain.items(),
            vec![
                ptt("a1", "down"),
                ptt("a1", "up"),
                ptt("b2", "down"),
                ptt("b2", "up")
            ]
        );
    }

    #[test]
    fn at_30_lib_wires_the_page_load_window_and_exit_hooks() {
        let lib = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
        )
        .unwrap();
        for hook in [
            ".on_page_load(hotkey_host::on_page_load)",
            "hotkey_host::on_window_event(window, event)",
            "hotkey_host::install(handle)",
            "hotkey_host::report_unavailable(handle)",
            "tauri::RunEvent::Exit",
            "hotkey_host::shutdown(app)",
        ] {
            assert!(lib.contains(hook), "lib.rs misses {hook}");
        }
    }

    /// Settings with an empty memory store and no OS reads (first run, D66 defaults).
    fn memory_settings() -> HotkeySettings {
        settings_with(MemorySettingsStore::default(), Box::new(NoTalk))
    }

    fn settings_with(store: MemorySettingsStore, talk: Box<dyn TalkKeySink>) -> HotkeySettings {
        HotkeySettings::load(
            mog_core::hotkey_bindings::Os::MacOs,
            Box::new(store),
            Box::new(mog_core::keyboard_env::NoEnvReads),
            talk,
        )
    }

    struct NoTalk;

    impl TalkKeySink for NoTalk {
        fn apply(&self, _talk: TalkBinding) {}
    }

    /// A factory double: each call makes a new granted [`FakeService`] and records the key.
    #[derive(Default, Clone)]
    struct FakeFactory {
        keys: Arc<Mutex<Vec<TalkBinding>>>,
        services: Arc<Mutex<Vec<FakeService>>>,
        perms: FakePermissions,
    }

    impl FakeFactory {
        fn make(&self, key: TalkBinding) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>) {
            let service = FakeService::default();
            service.granted.store(true, Ordering::SeqCst);
            self.keys.lock().unwrap().push(key);
            self.services.lock().unwrap().push(service.clone());
            (Box::new(service), Box::new(self.perms.clone()))
        }
        fn service(&self, i: usize) -> FakeService {
            self.services.lock().unwrap()[i].clone()
        }
    }

    #[test]
    fn cap_43a_install_gives_the_saved_talk_key_to_the_service() {
        let app = tauri::test::mock_app();
        let store = MemorySettingsStore::default();
        let mut saved =
            mog_core::hotkey_bindings::default_bindings(mog_core::hotkey_bindings::Os::MacOs);
        saved.set(
            mog_core::hotkey_bindings::HotkeyAction::Ptt,
            mog_core::hotkey_bindings::Binding::Talk(TalkBinding::RightControl),
        );
        use crate::hotkey_settings::HotkeySettingsStore;
        store
            .save(&crate::hotkey_settings::settings_from_bindings(
                mog_core::hotkey_bindings::Os::MacOs,
                &saved,
            ))
            .unwrap();
        let factory = FakeFactory::default();
        let f2 = factory.clone();
        install_with(
            app.handle(),
            settings_with(store, Box::new(NoTalk)),
            move |k, _| f2.make(k),
        )
        .unwrap();
        assert_eq!(
            *factory.keys.lock().unwrap(),
            vec![TalkBinding::RightControl]
        );
        assert_eq!(
            app.state::<HotkeyRuntime>().host.state(),
            HostState::Running
        );
    }

    #[test]
    fn cap_43a_a_new_talk_key_from_hotkeys_set_restarts_the_service_with_no_prompt() {
        let app = tauri::test::mock_app();
        let factory = FakeFactory::default();
        let f2 = factory.clone();
        let settings = settings_with(
            MemorySettingsStore::default(),
            Box::new(AppTalkKey(app.handle().clone())),
        );
        install_with(app.handle(), settings, move |k, _| f2.make(k)).unwrap();
        let s = app.state::<HotkeySettings>();
        let input: mog_core::ipc::JsonObject =
            serde_json::from_value(serde_json::json!({ "ptt": "RightControl" })).unwrap();
        s.set(&input).unwrap();
        assert_eq!(
            *factory.keys.lock().unwrap(),
            vec![TalkBinding::RightOption, TalkBinding::RightControl]
        );
        let (old, new) = (factory.service(0), factory.service(1));
        assert_eq!(old.stops.load(Ordering::SeqCst), 1, "the old service stops");
        assert_eq!(
            new.starts.load(Ordering::SeqCst),
            1,
            "the new service starts"
        );
        assert_eq!(
            app.state::<HotkeyRuntime>().host.state(),
            HostState::Running
        );
        assert!(
            factory.perms.prompts.lock().unwrap().is_empty(),
            "no prompt"
        );
        // Quit stops the new service.
        shutdown(app.handle());
        assert_eq!(new.stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cap_45_an_adapter_that_ends_during_a_hold_cancels_the_ptt() {
        // A talk-key restart stops the Windows hook with no key-up. The old adapter must
        // still end the PTT, or capture stays open.
        let (tx, rx) = mpsc::channel();
        let (sink, out) = queue();
        tx.send(down()).unwrap();
        drop(tx);
        run_hook_adapter(
            rx,
            HotkeyEngine::new(Arc::new(sink)),
            &HookConsumers::default(),
        );
        let got: Vec<HotkeyOutput> = out.try_iter().collect();
        assert_eq!(
            got,
            vec![
                HotkeyOutput::Ptt(PttPhase::Down),
                HotkeyOutput::Ptt(PttPhase::Cancel)
            ]
        );
    }

    /// A service whose stop fails, as `stop_post_failed` on Windows.
    struct StuckService(FakeService);

    impl HotkeyService for StuckService {
        fn start(&self) -> CoreResult<()> {
            self.0.start()
        }
        fn stop(&self) -> CoreResult<()> {
            self.0.stop()?;
            Err(CoreError::Unavailable {
                component: "R1",
                reason: "stop_post_failed",
            })
        }
        fn secure_input_active(&self) -> CoreResult<bool> {
            Ok(false)
        }
    }

    #[test]
    fn cap_43a_a_failed_stop_keeps_the_old_service_and_never_starts_a_second() {
        let old = FakeService::default();
        old.granted.store(true, Ordering::SeqCst);
        let h = HotkeyHost::new(
            Box::new(StuckService(old.clone())),
            Box::new(NoOsPermission),
        );
        assert_eq!(h.start(), HostState::Running);
        let new = FakeService::default();
        new.granted.store(true, Ordering::SeqCst);
        assert_eq!(
            h.replace_service(Box::new(new.clone())),
            HostState::Unavailable("stop_post_failed")
        );
        assert_eq!(new.starts.load(Ordering::SeqCst), 0, "no second hook");
        // The old service still runs the talk key.
        assert_eq!(h.state(), HostState::Running);
        h.stop();
        assert_eq!(old.stops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cap_36_replace_service_without_input_monitoring_never_prompts() {
        let (h, _old, perms) = host(true);
        assert_eq!(h.start(), HostState::NeedsPermission);
        let new = FakeService::default();
        assert_eq!(
            h.replace_service(Box::new(new.clone())),
            HostState::NeedsPermission
        );
        assert_eq!(new.starts.load(Ordering::SeqCst), 1);
        assert!(perms.prompts.lock().unwrap().is_empty());
    }

    #[test]
    fn cap_43a_replace_service_after_quit_does_not_start_it() {
        let (h, old, _) = host(true);
        old.granted.store(true, Ordering::SeqCst);
        assert_eq!(h.start(), HostState::Running);
        h.stop();
        let new = FakeService::default();
        new.granted.store(true, Ordering::SeqCst);
        assert_eq!(h.replace_service(Box::new(new.clone())), HostState::Stopped);
        assert_eq!(new.starts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cap_43a_replace_service_stops_the_old_and_runs_the_new() {
        let (h, old, _) = host(true);
        old.granted.store(true, Ordering::SeqCst);
        h.start();
        let new = FakeService::default();
        new.granted.store(true, Ordering::SeqCst);
        assert_eq!(h.replace_service(Box::new(new.clone())), HostState::Running);
        assert_eq!(old.stops.load(Ordering::SeqCst), 1);
        assert_eq!(new.starts.load(Ordering::SeqCst), 1);
        h.stop();
        assert_eq!(new.stops.load(Ordering::SeqCst), 1);
        assert_eq!(
            old.stops.load(Ordering::SeqCst),
            1,
            "the old service stops once"
        );
    }

    fn production_code(text: &str) -> String {
        text.split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn at_30_the_host_never_logs_and_uses_no_global_shortcut_plugin() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let code =
            production_code(&std::fs::read_to_string(root.join("src/hotkey_host.rs")).unwrap());
        assert!(
            code.contains("fn install_with"),
            "the host module is present"
        );
        for banned in [
            "println!",
            "eprintln!",
            "print!",
            "dbg!",
            "log::",
            "tracing::",
            "{:?}",
        ] {
            assert!(!code.contains(banned), "hotkey_host.rs uses {banned}");
        }
        for manifest in [root.join("Cargo.toml"), root.join("../Cargo.toml")] {
            let text = std::fs::read_to_string(&manifest).unwrap();
            assert!(!text.contains("global-shortcut"), "{}", manifest.display());
        }
    }

    // ---- Windows hook adapter (#314): a fake `Receiver<HookSignal>`, never a real hook ----

    use std::time::Duration;

    #[derive(Default, Clone)]
    struct FakeSeam(Arc<Mutex<Vec<DeferredSignal>>>);

    impl HookSignalSeam for FakeSeam {
        fn deferred(&self, signal: DeferredSignal) {
            self.0.lock().unwrap().push(signal);
        }
    }

    fn down() -> HookSignal {
        HookSignal::TalkDown {
            at: Duration::from_millis(1),
        }
    }

    fn up() -> HookSignal {
        HookSignal::TalkUp {
            at: Duration::from_millis(2),
        }
    }

    /// A clock that moves `step` forward on each read, so each hook signal that the engine
    /// times lands `step` after the one before, whatever the thread timing (CAP-44a).
    #[derive(Debug)]
    struct StepClock {
        now: Mutex<Duration>,
        step: Duration,
    }

    impl MonotonicClock for StepClock {
        fn now(&self) -> Duration {
            let mut now = self.now.lock().unwrap();
            *now += self.step;
            *now
        }
    }

    /// Run `signals` through a fake hook receiver, the adapter thread, the host queue and the
    /// forwarder thread, as `platform_service` wires them on Windows. The signals land 300 ms
    /// apart, so a press is not a tap (CAP-44a).
    fn run_windows_path(
        target: &Arc<PttTarget>,
        signals: &[HookSignal],
    ) -> (Vec<(String, Value)>, Vec<HotkeyStatus>, Vec<DeferredSignal>) {
        run_windows_path_stepped(target, signals, Duration::from_millis(300))
    }

    fn run_windows_path_stepped(
        target: &Arc<PttTarget>,
        signals: &[HookSignal],
        step: Duration,
    ) -> (Vec<(String, Value)>, Vec<HotkeyStatus>, Vec<DeferredSignal>) {
        let (f, brain, status) = forwarder(target);
        let (sink, out_rx) = queue();
        let forward = spawn_forwarder(out_rx, f).unwrap();
        let (hook_tx, hook_rx) = mpsc::channel();
        let seam = FakeSeam::default();
        let clock = Arc::new(StepClock {
            now: Mutex::new(Duration::ZERO),
            step,
        });
        let adapter =
            spawn_hook_adapter_with_clock(hook_rx, Arc::new(sink), seam.clone(), clock).unwrap();
        for s in signals {
            hook_tx.send(*s).unwrap();
        }
        // The service drops its sender on quit: the adapter ends, then the forwarder.
        drop(hook_tx);
        adapter.join().unwrap();
        forward.join().unwrap();
        let statuses = status.0.lock().unwrap().clone();
        let deferred = seam.0.lock().unwrap().clone();
        (brain.items(), statuses, deferred)
    }

    #[test]
    fn cap_40a_mb_06a_talk_down_and_up_reach_the_brain_as_hotkey_ptt() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let (items, statuses, deferred) = run_windows_path(&target, &[down(), up()]);
        assert_eq!(items, vec![ptt("a1", "down"), ptt("a1", "up")]);
        assert!(statuses.is_empty());
        assert!(deferred.is_empty());
    }

    #[test]
    fn cap_40a_a_repeated_talk_down_gives_one_ptt_down() {
        // The shared PTT machine (#6) drops a second `down` while the key is held, so
        // Windows gives the same phases as macOS.
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let (items, _, _) = run_windows_path(&target, &[down(), down(), up(), up()]);
        assert_eq!(items, vec![ptt("a1", "down"), ptt("a1", "up")]);
    }

    #[test]
    fn mb_06a_a_windows_press_goes_to_the_mog_that_got_down() {
        let target = Arc::new(PttTarget::default());
        let mut engine = HotkeyEngine::new(Arc::new(queue().0));
        // No target: nothing reaches the brain, and the adapter keeps no state of its own.
        assert_eq!(adapt_hook_signal(down(), &mut engine), None);
        let (items, _, _) = run_windows_path(&target, &[down(), up()]);
        assert!(items.is_empty());
        target.set("b2");
        let (items, _, _) = run_windows_path(&target, &[down(), up()]);
        assert_eq!(items, vec![ptt("b2", "down"), ptt("b2", "up")]);
    }

    #[test]
    fn cap_40a_deferred_hook_signals_go_to_the_seam_not_the_brain() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let (items, statuses, deferred) = run_windows_path(
            &target,
            &[
                HookSignal::Escape,
                HookSignal::TalkKeyBlocked,
                HookSignal::HookReinstalled,
                HookSignal::HookFailed {
                    reason: "set_windows_hook_failed",
                },
            ],
        );
        assert!(items.is_empty());
        assert!(statuses.is_empty());
        assert_eq!(
            deferred,
            vec![
                DeferredSignal::Escape,
                DeferredSignal::TalkKeyBlocked,
                DeferredSignal::HookReinstalled,
                DeferredSignal::HookFailed("set_windows_hook_failed"),
            ]
        );
    }

    #[test]
    fn cap_44c_at_03_a_windows_other_key_cancels_the_ptt_and_is_not_deferred() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        // The other key lands 300 ms into the hold: inside the 1 s window.
        let (items, _, deferred) =
            run_windows_path(&target, &[down(), HookSignal::OtherKeyDown, up()]);
        assert_eq!(items, vec![ptt("a1", "down"), ptt("a1", "cancel")]);
        assert!(deferred.is_empty());
    }

    #[test]
    fn cap_44a_at_03_a_short_windows_press_reaches_the_brain_as_cancel() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        // 100 ms between down and up: a tap. The brain gets `cancel`, never `up`.
        let (items, _, _) =
            run_windows_path_stepped(&target, &[down(), up()], Duration::from_millis(100));
        assert_eq!(items, vec![ptt("a1", "down"), ptt("a1", "cancel")]);
    }

    #[test]
    fn cap_40a_talk_key_signals_are_not_deferred() {
        let mut engine = HotkeyEngine::new(Arc::new(queue().0));
        assert_eq!(adapt_hook_signal(down(), &mut engine), None);
        assert!(engine.machine().is_held());
        assert_eq!(adapt_hook_signal(up(), &mut engine), None);
        assert!(!engine.machine().is_held());
    }

    #[test]
    fn at_30_a_failed_adapter_spawn_reports_unavailable_at_start() {
        let service = FakeService::default();
        let failed = with_hook_adapter(
            Box::new(service.clone()),
            Err(std::io::Error::other("spawn")),
        );
        let h = HotkeyHost::new(failed, Box::new(NoOsPermission));
        assert_eq!(
            h.start(),
            HostState::Unavailable("hook_adapter_spawn_failed")
        );
        // The real service never starts without its adapter.
        assert_eq!(service.starts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn at_30_a_spawned_adapter_keeps_the_real_service() {
        let service = FakeService::default();
        service.granted.store(true, Ordering::SeqCst);
        let ok = with_hook_adapter(
            Box::new(service.clone()),
            std::thread::Builder::new().spawn(|| {}),
        );
        let h = HotkeyHost::new(ok, Box::new(NoOsPermission));
        assert_eq!(h.start(), HostState::Running);
        assert_eq!(service.starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn at_30_the_windows_permission_api_never_prompts() {
        // Windows has no OS prompt for the hook: status and request both say granted.
        let p = NoOsPermission;
        for perm in [Permission::InputMonitoring, Permission::Accessibility] {
            assert_eq!(p.status(perm), PermissionStatus::Granted);
            assert_eq!(p.request(perm), PermissionStatus::Granted);
        }
    }

    #[test]
    fn at_30_the_windows_platform_service_wires_the_hook_adapter() {
        // The `cfg(windows)` branch cannot build on a macOS host (tauri-winres needs
        // llvm-rc), so this checks its source. CI on Windows (#227) and #296 build and run it.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let code =
            production_code(&std::fs::read_to_string(root.join("src/hotkey_host.rs")).unwrap());
        let windows = code
            .split("#[cfg(windows)]\npub fn platform_service")
            .nth(1)
            .expect("a cfg(windows) platform_service")
            .split("\n}\n")
            .next()
            .unwrap();
        for part in [
            "WindowsHotkeyService::new(talk.windows_talk_key())",
            "HookConsumers::new(copilot).with_status(status)",
            "spawn_hook_adapter(rx, sink, seam)",
            "with_hook_adapter(Box::new(service), adapter)",
            "Box::new(NoOsPermission)",
        ] {
            assert!(
                windows.contains(part),
                "windows platform_service misses {part}"
            );
        }
        assert!(!windows.contains("StubHotkeyService"));
    }

    #[test]
    fn cap_40a_every_deferred_signal_names_its_consumer_issue() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(root.join("src/hotkey_host.rs")).unwrap();
        let seam = text
            .split("impl HookSignalSeam for HookConsumers")
            .nth(1)
            .unwrap()
            .split("\n}\n")
            .next()
            .unwrap();
        for todo in ["TODO(#434)", "TODO(#346)"] {
            assert!(seam.contains(todo), "the seam misses {todo}");
        }
        // #294 and #297 are done: those arms send `hotkey.status` (ADR 0039).
        for done in ["TODO(#294)", "TODO(#297)"] {
            assert!(!seam.contains(done), "the seam still has {done}");
        }
        for status in [
            "HotkeyStatus::Blocked",
            "HotkeyStatus::Restored",
            "HotkeyStatus::Unavailable",
        ] {
            assert!(seam.contains(status), "the seam misses {status}");
        }
    }

    // ---- CAP-43b (#332, ADR 0025): the hook sets the Copilot key flag ----

    #[test]
    fn cap_43b_copilot_key_seen_is_deferred_to_the_seam() {
        let (sink, _out) = queue();
        let mut engine = HotkeyEngine::new(Arc::new(sink));
        assert_eq!(
            adapt_hook_signal(HookSignal::CopilotKeySeen, &mut engine),
            Some(DeferredSignal::CopilotKeySeen)
        );
    }

    #[test]
    fn cap_43b_the_seam_sets_the_shared_copilot_flag() {
        let flag = Arc::new(MemoryCopilotKeyFlag::default());
        let seam = HookConsumers::new(flag.clone());
        seam.deferred(DeferredSignal::Escape);
        assert!(!flag.present(), "only the Copilot signal sets the flag");
        seam.deferred(DeferredSignal::CopilotKeySeen);
        assert!(flag.present());
    }

    #[test]
    fn cap_43b_the_hook_adapter_sets_the_flag_with_no_ptt_output() {
        let flag = Arc::new(MemoryCopilotKeyFlag::default());
        let (tx, rx) = mpsc::channel();
        let (sink, out) = queue();
        tx.send(HookSignal::CopilotKeySeen).unwrap();
        drop(tx);
        run_hook_adapter(
            rx,
            HotkeyEngine::new(Arc::new(sink)),
            &HookConsumers::new(flag.clone()),
        );
        assert!(flag.present());
        assert!(out.try_iter().next().is_none());
    }

    #[test]
    fn cap_43b_install_shares_one_copilot_flag_with_the_env_reads_and_the_hook() {
        // The `cfg(windows)` factory cannot build on a macOS host, so this checks the source.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let code =
            production_code(&std::fs::read_to_string(root.join("src/hotkey_host.rs")).unwrap());
        let install = code
            .split("pub fn install<R: Runtime>")
            .nth(1)
            .unwrap()
            .split("\n}\n")
            .next()
            .unwrap();
        // rustfmt may wrap a call, so compare with no whitespace.
        let flat: String = install.split_whitespace().collect();
        for part in [
            "MemoryCopilotKeyFlag::default()",
            "WithCopilotFlag::new(keyboard_env::os_env_reads(),Arc::clone(&copilot)",
            "platform_service(talk,sink,Arc::clone(&copilot),Arc::clone(&status))",
            "BrainStatus(app.clone())",
        ] {
            assert!(flat.contains(part), "install misses {part}");
        }
    }

    // ---- CAP-39c, CAP-42, CAP-40a (#297, #294, ADR 0039): the `hotkey.status` event ----

    /// A brain double that keeps sticky items apart from plain ones.
    #[derive(Default, Clone)]
    struct StickyBrain {
        plain: Arc<Mutex<Vec<(String, Value)>>>,
        sticky: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl BrainSink for StickyBrain {
        fn deliver(&self, event: &str, payload: Value) -> bool {
            self.plain.lock().unwrap().push((event.to_owned(), payload));
            true
        }
        fn deliver_sticky(&self, event: &str, payload: Value) -> bool {
            self.sticky
                .lock()
                .unwrap()
                .push((event.to_owned(), payload));
            true
        }
    }

    fn status_item(talk_key: &str, reason: Value) -> (String, Value) {
        (
            "hotkey:status".to_owned(),
            json!({ "talkKey": talk_key, "reason": reason }),
        )
    }

    #[test]
    fn cap_39c_cap_42_each_status_maps_to_its_talk_key_payload() {
        for (status, want) in [
            (
                HotkeyStatus::Paused,
                json!({"talkKey": "paused", "reason": "secure_input"}),
            ),
            (
                HotkeyStatus::Restored,
                json!({"talkKey": "active", "reason": null}),
            ),
            (
                HotkeyStatus::ChordUnavailable,
                json!({"talkKey": "paused", "reason": "fallback_chord_unavailable"}),
            ),
            (
                HotkeyStatus::Blocked,
                json!({"talkKey": "paused", "reason": "elevated_foreground"}),
            ),
            (
                HotkeyStatus::Unavailable,
                json!({"talkKey": "unavailable", "reason": "hook_failed"}),
            ),
        ] {
            assert_eq!(status_payload(status), want, "{status:?}");
        }
    }

    #[test]
    fn cap_39c_arc_5_pill_outputs_go_sticky_on_hotkey_status_and_restore_clears() {
        let target = Arc::new(PttTarget::default());
        target.set("a1");
        let brain = StickyBrain::default();
        let mut f = PttForwarder::new(
            Arc::clone(&target),
            brain.clone(),
            BrainStatus(brain.clone()),
        );
        f.on_output(HotkeyOutput::TalkKeyPaused);
        f.on_output(HotkeyOutput::FallbackChordUnavailable);
        f.on_output(HotkeyOutput::TalkKeyRestored);
        assert!(brain.plain.lock().unwrap().is_empty(), "status is sticky");
        assert_eq!(
            *brain.sticky.lock().unwrap(),
            vec![
                status_item("paused", json!("secure_input")),
                status_item("paused", json!("fallback_chord_unavailable")),
                status_item("active", Value::Null),
            ]
        );
    }

    #[test]
    fn cap_42_an_elevated_window_gives_paused_elevated_foreground() {
        let brain = StickyBrain::default();
        let seam = HookConsumers::default().with_status(Arc::new(BrainStatus(brain.clone())));
        seam.deferred(DeferredSignal::TalkKeyBlocked);
        assert_eq!(
            *brain.plain.lock().unwrap(),
            vec![status_item("paused", json!("elevated_foreground"))]
        );
    }

    #[test]
    fn cap_42_the_elevated_tooltip_is_not_sticky_so_a_reloaded_brain_does_not_see_it_again() {
        // R1 sends `Blocked` once per elevated process and nothing ends it. A sticky copy would
        // replay the tooltip and a false "paused" to every new brain Channel (CAP-42: first
        // failure only).
        let brain = StickyBrain::default();
        let seam = HookConsumers::default().with_status(Arc::new(BrainStatus(brain.clone())));
        seam.deferred(DeferredSignal::HookReinstalled);
        seam.deferred(DeferredSignal::TalkKeyBlocked);
        assert_eq!(
            *brain.sticky.lock().unwrap(),
            vec![status_item("active", Value::Null)],
            "the kept state stays the last real state"
        );
    }

    #[test]
    fn cap_40a_sec_01_hook_health_goes_on_hotkey_status_with_no_reason_text() {
        let brain = StickyBrain::default();
        let seam = HookConsumers::default().with_status(Arc::new(BrainStatus(brain.clone())));
        seam.deferred(DeferredSignal::HookFailed("set_windows_hook_failed"));
        seam.deferred(DeferredSignal::HookReinstalled);
        let got = brain.sticky.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![
                status_item("unavailable", json!("hook_failed")),
                status_item("active", Value::Null),
            ]
        );
        assert!(!format!("{got:?}").contains("set_windows_hook_failed"));
        // Escape and the Copilot key are not talk-key status.
        seam.deferred(DeferredSignal::Escape);
        seam.deferred(DeferredSignal::CopilotKeySeen);
        assert_eq!(brain.sticky.lock().unwrap().len(), 2);
    }

    #[test]
    fn cap_40a_a_seam_with_no_status_sink_still_takes_every_signal() {
        let seam = HookConsumers::default();
        for s in [
            DeferredSignal::TalkKeyBlocked,
            DeferredSignal::HookReinstalled,
            DeferredSignal::HookFailed("x"),
        ] {
            seam.deferred(s);
        }
    }

    /// A brain Channel on `app` that records each message; returns the record.
    fn subscribe_recorder<R: Runtime>(app: &AppHandle<R>) -> Arc<Mutex<Vec<Value>>> {
        use tauri::ipc::{Channel, InvokeResponseBody};
        let got: Arc<Mutex<Vec<Value>>> = Arc::default();
        let sink = Arc::clone(&got);
        let channel = Channel::<Value>::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                sink.lock()
                    .unwrap()
                    .push(serde_json::from_str(&text).unwrap());
            }
            Ok(())
        });
        app.state::<BrainChannel>().set(channel);
        got
    }

    #[test]
    fn at_30_arc_5_a_failed_install_reaches_a_brain_that_subscribes_later() {
        let app = tauri::test::mock_app();
        app.manage(BrainChannel::default());
        // Setup runs before the brain subscribes.
        report_unavailable(app.handle());
        let got = subscribe_recorder(app.handle());
        assert_eq!(
            *got.lock().unwrap(),
            vec![json!({
                "event": "hotkey:status",
                "payload": {"talkKey": "unavailable", "reason": "hook_failed"}
            })]
        );
    }

    #[test]
    fn cap_39c_install_sends_r1_pill_outputs_on_the_brain_channel() {
        let app = tauri::test::mock_app();
        app.manage(BrainChannel::default());
        let sinks: Arc<Mutex<Vec<Arc<dyn HotkeySink>>>> = Arc::default();
        let keep = Arc::clone(&sinks);
        install_with(app.handle(), memory_settings(), move |_key, sink| {
            keep.lock().unwrap().push(sink);
            (
                Box::new(FakeService::default()) as Box<dyn HotkeyService>,
                Box::new(FakePermissions::default()) as Box<dyn PermissionApi>,
            )
        })
        .unwrap();
        let got = subscribe_recorder(app.handle());
        let sink = Arc::clone(&sinks.lock().unwrap()[0]);
        sink.emit(HotkeyOutput::TalkKeyPaused);
        let want = json!({
            "event": "hotkey:status",
            "payload": {"talkKey": "paused", "reason": "secure_input"}
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while got.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(*got.lock().unwrap(), vec![want]);
    }

    // ---- CAP-36, CAP-37 (#312, ADR 0039): the runtime is the permission route ----

    #[test]
    fn cap_36_cap_37_the_runtime_routes_each_permission_to_its_host_call() {
        use crate::ipc::PermissionRoute;
        use mog_core::ipc::PermissionKind;
        let app = tauri::test::mock_app();
        let factory = FakeFactory {
            perms: FakePermissions {
                grant: true,
                ..FakePermissions::default()
            },
            ..FakeFactory::default()
        };
        let f2 = factory.clone();
        install_with(app.handle(), memory_settings(), move |k, _| f2.make(k)).unwrap();
        let rt = app.state::<HotkeyRuntime>();
        assert_eq!(
            rt.request_permission(PermissionKind::Accessibility, UserAction::from_user_click()),
            PermissionStatus::Granted
        );
        assert_eq!(
            rt.request_permission(
                PermissionKind::InputMonitoring,
                UserAction::from_user_click()
            ),
            PermissionStatus::Granted
        );
        assert_eq!(
            *factory.perms.prompts.lock().unwrap(),
            vec![Permission::Accessibility, Permission::InputMonitoring]
        );
        // A granted Input Monitoring starts R1 again (CAP-36); Accessibility does not.
        assert_eq!(factory.service(0).starts.load(Ordering::SeqCst), 2);
    }
}
