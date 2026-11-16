//! The host wiring of R1 HotkeyService (#305, ADR 0013).
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
//! `MacHotkeyService` (#6). Other platforms use the R1 stub until the Windows hook is wired:
//! TODO(#314).
//!
//! Requirements: CAP-35a, CAP-36, CAP-37, CAP-39c, MB-06a, AT-30.

use crate::ipc::BrainChannel;
use mog_core::error::CoreError;
use mog_core::hotkey::HotkeyService;
use mog_core::hotkey_core::{
    self, HotkeyOutput, HotkeySink, PermissionApi, PermissionStatus, PttPhase, TalkKey, UserAction,
};
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
    /// The window got focus (the user interacted with it).
    Focused,
    /// The window is gone.
    Destroyed,
}

/// The last-interacted MOG: the target of the talk key (MB-06a).
///
/// Interim input: a `mog-<id>` window that gets focus. The brain's `mog_set_active` (click,
/// drag, card open, chord use) replaces it: TODO(#73).
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

    /// Apply a window signal for the window `label`. Only `mog-<id>` windows count.
    pub fn on_window(&self, label: &str, signal: WindowSignal) {
        let Some((WindowClass::Mog, Some(id))) = WindowClass::of_label(label) else {
            return;
        };
        let mut target = lock(&self.0);
        match signal {
            WindowSignal::Focused => *target = Some(id.to_owned()),
            WindowSignal::Destroyed => {
                if target.as_deref() == Some(id) {
                    *target = None;
                }
            }
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

/// The platform service and permission API (no hook yet on this OS).
#[cfg(not(target_os = "macos"))]
pub fn platform_service(
    _talk_key: TalkKey,
    _sink: Arc<dyn HotkeySink>,
) -> (Box<dyn HotkeyService>, Box<dyn PermissionApi>) {
    // TODO(#314): `WindowsHotkeyService` (#7) plugs in here through a `HookSignal` adapter.
    (
        Box::new(mog_core::hotkey::StubHotkeyService),
        Box::new(NoOsPermission),
    )
}

/// The permission API of an OS that has no PTT permission prompt.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOsPermission;

#[cfg(not(target_os = "macos"))]
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

/// Update the talk-key target from a window event.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    let signal = match event {
        tauri::WindowEvent::Focused(true) => WindowSignal::Focused,
        tauri::WindowEvent::Destroyed => WindowSignal::Destroyed,
        _ => return,
    };
    if let Some(rt) = window.try_state::<HotkeyRuntime>() {
        rt.target.on_window(window.label(), signal);
    }
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
        // The hub, the brain, cards and bad labels do not change the target.
        for label in ["hub", "brain", "card-b2", "mog-", "mog-a b", "other"] {
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
}
