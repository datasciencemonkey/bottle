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
//! Requirements: CAP-35a, CAP-36, CAP-37, CAP-39c, CAP-40a, MB-06a, AT-30.

use crate::ipc::BrainChannel;
use mog_core::error::CoreError;
use mog_core::hotkey::HotkeyService;
use mog_core::hotkey_core::{
    self, HotkeyEngine, HotkeyOutput, HotkeySink, PermissionApi, PermissionStatus, PttPhase,
    TalkKey, UserAction,
};
use mog_core::hotkey_windows::{forward_to_engine, HookSignal};
use mog_core::ipc::{events, WindowClass};
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
}

impl<R: Runtime> BrainSink for AppHandle<R> {
    fn deliver(&self, event: &str, payload: Value) -> bool {
        self.try_state::<BrainChannel>()
            .is_some_and(|ch| ch.send(event, payload))
    }
}

/// A talk-key status change for the "Talk key paused" pill (CAP-39c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyStatus {
    /// Secure Event Input is on: show the pill.
    Paused,
    /// The talk key works again: hide the pill.
    Restored,
    /// Secure Event Input is on and the OS refused the fallback chord.
    ChordUnavailable,
}

/// Receives the talk-key status. The event for it is contract change #297.
pub trait StatusSink: Send + Sync {
    /// Take one status change.
    fn status(&self, status: HotkeyStatus);
}

/// The app's status sink until the `hotkey.status` event exists.
#[derive(Debug, Default, Clone, Copy)]
pub struct PendingStatusEvent;

impl StatusSink for PendingStatusEvent {
    fn status(&self, _status: HotkeyStatus) {
        // TODO(#297): send `hotkey:status` on the brain Channel once the contract has it.
    }
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

/// The last-interacted MOG: the target of the talk key (MB-06a).
///
/// Interim inputs, until the brain's `mog_set_active` (click, drag, card open, chord use)
/// replaces them: TODO(#73).
/// - A `mog-<id>` window opens: the last-opened MOG is the target. MOG windows cannot take
///   focus (MOG-01b), so their focus never counts.
/// - A `card-<id>` window opens or gets focus: its MOG is the target (a card open is an
///   interaction with that MOG).
/// - A `mog-<id>` window that is the target closes: no target.
#[derive(Debug, Default)]
pub struct PttTarget(Mutex<Option<String>>);

impl PttTarget {
    /// Make `mog_id` the target.
    pub fn set(&self, mog_id: &str) {
        *lock(&self.0) = Some(mog_id.to_owned());
    }

    /// The target now.
    pub fn current(&self) -> Option<String> {
        lock(&self.0).clone()
    }

    /// Apply a window signal for the window `label`. Only `mog-<id>` and `card-<id>` count.
    pub fn on_window(&self, label: &str, signal: WindowSignal) {
        let Some((class, Some(id))) = WindowClass::of_label(label) else {
            return;
        };
        let mut target = lock(&self.0);
        match (class, signal) {
            (
                WindowClass::Mog | WindowClass::Card,
                WindowSignal::Opened | WindowSignal::Focused,
            ) => *target = Some(id.to_owned()),
            (WindowClass::Mog, WindowSignal::Destroyed) if target.as_deref() == Some(id) => {
                *target = None
            }
            _ => {}
        }
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
pub struct PttForwarder<B: BrainSink, S: StatusSink> {
    target: Arc<PttTarget>,
    brain: B,
    status: S,
    held: Option<String>,
}

impl<B: BrainSink, S: StatusSink> PttForwarder<B, S> {
    /// A forwarder that reads the target from `target`.
    pub fn new(target: Arc<PttTarget>, brain: B, status: S) -> Self {
        Self {
            target,
            brain,
            status,
            held: None,
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
                    PttPhase::Up | PttPhase::Cancel => self.held.take(),
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
            // dropped its audio. Section 4.5 has no `tap` phase; the double-tap rule
            // (CAP-14a) reads it later.
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
    /// Esc went down during capture. The capture lifecycle (#14) ends hands-free capture.
    Escape,
    /// Another key went down during a hold. The CAP-44c cancel (#24) consumes it.
    OtherKeyDown,
    /// An elevated window blocks the talk key. The CAP-42 tooltip (#294) consumes it.
    TalkKeyBlocked,
    /// The hook was installed again (CAP-40b).
    HookReinstalled,
    /// A hook install failed. The reason is a fixed, content-free string.
    HookFailed(&'static str),
}

/// Receives the [`DeferredSignal`]s of the Windows hook.
pub trait HookSignalSeam: Send {
    /// Take one deferred signal.
    fn deferred(&self, signal: DeferredSignal);
}

/// The app's seam until each consumer exists. Each arm names the issue that consumes it.
#[derive(Debug, Default, Clone, Copy)]
pub struct PendingHookConsumers;

impl HookSignalSeam for PendingHookConsumers {
    fn deferred(&self, signal: DeferredSignal) {
        match signal {
            DeferredSignal::Escape => {
                // TODO(#14): the capture lifecycle ends hands-free capture on Esc.
            }
            DeferredSignal::OtherKeyDown => {
                // TODO(#24): `HotkeyEngine::on_other_key_down` cancels the PTT (CAP-44c).
            }
            DeferredSignal::TalkKeyBlocked => {
                // TODO(#294): one `hotkey.status` event makes the MOG show the CAP-42 tooltip.
            }
            DeferredSignal::HookReinstalled | DeferredSignal::HookFailed(_) => {
                // TODO(#297): the hook health goes on the `hotkey.status` event.
            }
        }
    }
}

/// Map one Windows hook signal (#7) to the shared R1 engine (#6).
///
/// `TalkDown` and `TalkUp` go through [`forward_to_engine`] to the shared PTT machine, so
/// Windows gives the same `hotkey.ptt` phases as macOS. The other signals return as a
/// [`DeferredSignal`] for the seam.
pub fn adapt_hook_signal(signal: HookSignal, engine: &mut HotkeyEngine) -> Option<DeferredSignal> {
    Some(match forward_to_engine(signal, engine)? {
        HookSignal::Escape => DeferredSignal::Escape,
        HookSignal::OtherKeyDown => DeferredSignal::OtherKeyDown,
        HookSignal::TalkKeyBlocked => DeferredSignal::TalkKeyBlocked,
        HookSignal::HookReinstalled => DeferredSignal::HookReinstalled,
        HookSignal::HookFailed { reason } => DeferredSignal::HookFailed(reason),
        // `forward_to_engine` takes the talk-key signals.
        HookSignal::TalkDown { .. } | HookSignal::TalkUp { .. } => return None,
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
    std::thread::Builder::new()
        .name("bottle-hotkey-signals".into())
        .spawn(move || run_hook_adapter(rx, HotkeyEngine::new(sink), &seam))
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

/// The talk key of the settings value (D66). Unknown or missing values give Right-Option.
pub fn talk_key_from_setting(value: Option<&str>) -> TalkKey {
    match value {
        Some("right_control") => TalkKey::RightControl,
        Some("right_command") => TalkKey::RightCommand,
        Some("right_shift") => TalkKey::RightShift,
        _ => TalkKey::RightOption,
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
    service: Box<dyn HotkeyService>,
    permissions: Box<dyn PermissionApi>,
    state: Mutex<HostState>,
}

impl HotkeyHost {
    /// A stopped host.
    pub fn new(service: Box<dyn HotkeyService>, permissions: Box<dyn PermissionApi>) -> Self {
        Self {
            service,
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
        *state = match self.service.start() {
            Ok(()) => HostState::Running,
            Err(CoreError::Unavailable {
                reason: "input_monitoring_required",
                ..
            }) => HostState::NeedsPermission,
            Err(e) => HostState::Unavailable(e.code()),
        };
        *state
    }

    /// Ask for Input Monitoring after a user click, then start again if granted (CAP-36).
    /// The wizard (#43) and Settings call this through an IPC command: TODO(#312).
    pub fn request_ptt_permission(&self, action: UserAction) -> PermissionStatus {
        let status = hotkey_core::request_ptt_permission(self.permissions.as_ref(), action);
        if status == PermissionStatus::Granted {
            self.start();
        }
        status
    }

    /// Ask for Accessibility after the user selects text insertion, and only then (CAP-37).
    /// TODO(#312): the IPC command for the insertion choice.
    pub fn request_insertion_permission(&self, action: UserAction) -> PermissionStatus {
        hotkey_core::request_insertion_permission(self.permissions.as_ref(), action)
    }

    /// Stop the service (app quit).
    pub fn stop(&self) {
        let mut state = lock(&self.state);
        // A stop error leaves nothing to undo at quit.
        let _ = self.service.stop();
        *state = HostState::Stopped;
    }
}

/// The host state that the app manages.
pub struct HotkeyRuntime {
    /// The service owner.
    pub host: HotkeyHost,
    /// The talk-key target.
    pub target: Arc<PttTarget>,
}

/// The platform service and permission API (macOS).
#[cfg(target_os = "macos")]
pub fn platform_service(
    talk_key: TalkKey,
    sink: Arc<dyn HotkeySink>,
) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>) {
    use mog_core::hotkey_macos::{MacHotkeyService, MacPermissions};
    (
        Box::new(MacHotkeyService::new(talk_key, sink)),
        Box::new(MacPermissions),
    )
}

/// The platform service and permission API (Windows, #7 and #314).
///
/// The hook sends [`HookSignal`]s on its own channel; the adapter thread feeds them to the
/// shared engine, which emits into `sink`. The hook needs no OS prompt.
#[cfg(windows)]
pub fn platform_service(
    talk_key: TalkKey,
    sink: Arc<dyn HotkeySink>,
) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>) {
    use mog_core::hotkey_windows::win32::WindowsHotkeyService;
    let (service, rx) = WindowsHotkeyService::new(talk_key.into());
    let adapter = spawn_hook_adapter(rx, sink, PendingHookConsumers);
    (
        with_hook_adapter(Box::new(service), adapter),
        Box::new(NoOsPermission),
    )
}

/// The platform service and permission API (no hook on this OS).
#[cfg(not(any(target_os = "macos", windows)))]
pub fn platform_service(
    _talk_key: TalkKey,
    _sink: Arc<dyn HotkeySink>,
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

/// Create the host with the platform service and start it (no prompt).
pub fn install<R: Runtime>(app: &AppHandle<R>) -> std::io::Result<()> {
    install_with(app, platform_service)
}

/// Create the host with `factory`, start the forwarder thread and start the service.
pub fn install_with<R, F>(app: &AppHandle<R>, factory: F) -> std::io::Result<()>
where
    R: Runtime,
    F: FnOnce(TalkKey, Arc<dyn HotkeySink>) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>),
{
    let target = Arc::new(PttTarget::default());
    let (sink, rx) = queue();
    spawn_forwarder(
        rx,
        PttForwarder::new(Arc::clone(&target), app.clone(), PendingStatusEvent),
    )?;
    // TODO(#77): read the talk key from Settings (`hotkeys_set`); D66 default until then.
    let (service, permissions) = factory(talk_key_from_setting(None), Arc::new(sink));
    let host = HotkeyHost::new(service, permissions);
    host.start();
    app.manage(HotkeyRuntime { host, target });
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

    #[test]
    fn cap_43a_talk_key_setting_defaults_to_right_option() {
        assert_eq!(talk_key_from_setting(None), TalkKey::RightOption);
        assert_eq!(talk_key_from_setting(Some("bogus")), TalkKey::RightOption);
        assert_eq!(
            talk_key_from_setting(Some("right_option")),
            TalkKey::RightOption
        );
        assert_eq!(
            talk_key_from_setting(Some("right_control")),
            TalkKey::RightControl
        );
        assert_eq!(
            talk_key_from_setting(Some("right_command")),
            TalkKey::RightCommand
        );
        assert_eq!(
            talk_key_from_setting(Some("right_shift")),
            TalkKey::RightShift
        );
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
        install_with(app.handle(), move |key, _sink| {
            k2.lock().unwrap().push(key);
            (
                Box::new(s2) as Box<dyn HotkeyService>,
                Box::new(p2) as Box<dyn PermissionApi>,
            )
        })
        .unwrap();
        assert_eq!(*keys.lock().unwrap(), vec![TalkKey::RightOption]);
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
        install_with(app.handle(), |_key, _sink| {
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
            "tauri::RunEvent::Exit",
            "hotkey_host::shutdown(app)",
        ] {
            assert!(lib.contains(hook), "lib.rs misses {hook}");
        }
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

    /// Run `signals` through a fake hook receiver, the adapter thread, the host queue and the
    /// forwarder thread, as `platform_service` wires them on Windows.
    fn run_windows_path(
        target: &Arc<PttTarget>,
        signals: &[HookSignal],
    ) -> (Vec<(String, Value)>, Vec<HotkeyStatus>, Vec<DeferredSignal>) {
        let (f, brain, status) = forwarder(target);
        let (sink, out_rx) = queue();
        let forward = spawn_forwarder(out_rx, f).unwrap();
        let (hook_tx, hook_rx) = mpsc::channel();
        let seam = FakeSeam::default();
        let adapter = spawn_hook_adapter(hook_rx, Arc::new(sink), seam.clone()).unwrap();
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
                HookSignal::OtherKeyDown,
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
                DeferredSignal::OtherKeyDown,
                DeferredSignal::TalkKeyBlocked,
                DeferredSignal::HookReinstalled,
                DeferredSignal::HookFailed("set_windows_hook_failed"),
            ]
        );
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
            "WindowsHotkeyService::new(talk_key.into())",
            "spawn_hook_adapter(rx, sink, PendingHookConsumers)",
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
            .split("impl HookSignalSeam for PendingHookConsumers")
            .nth(1)
            .unwrap()
            .split("\n}\n")
            .next()
            .unwrap();
        for todo in ["TODO(#14)", "TODO(#24)", "TODO(#294)", "TODO(#297)"] {
            assert!(seam.contains(todo), "the seam misses {todo}");
        }
    }
}
