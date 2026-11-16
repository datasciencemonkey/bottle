//! Test doubles of the R1 OS seams for acceptance tests (#230, AT-03, AT-30).
//!
//! `hotkey_core` reads the OS through [`PlatformProbe`], [`TapControl`] and [`PermissionApi`],
//! and reports through [`HotkeySink`]. These doubles let a test drive the shipped
//! [`crate::hotkey_core::HotkeyEngine`] on the #229 `SimClock` with no OS call, no key and no
//! prompt. They record only R1 outputs and permission names, never key contents.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::hotkey_core::{
    HotkeyOutput, HotkeySink, Permission, PermissionApi, PermissionStatus, PhysicalState,
    PlatformProbe, TapControl,
};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A [`HotkeySink`] that keeps every output in order.
#[derive(Debug, Default)]
pub struct RecordingSink(Mutex<Vec<HotkeyOutput>>);

impl RecordingSink {
    /// A new, empty sink, ready to hand to `HotkeyEngine::with_clock`.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Take the outputs recorded since the last call.
    pub fn take(&self) -> Vec<HotkeyOutput> {
        std::mem::take(&mut *lock(&self.0))
    }
}

impl HotkeySink for RecordingSink {
    fn emit(&self, output: HotkeyOutput) {
        lock(&self.0).push(output);
    }
}

/// A [`PlatformProbe`] whose readings the test sets. It starts with Secure Input off, no key
/// down, no sleep, the session active and Input Monitoring granted.
#[derive(Debug)]
pub struct ScriptedProbe {
    secure_input: AtomicBool,
    talk_key_down: AtomicBool,
    chord_down: AtomicBool,
    slept_us: AtomicU64,
    session_active: AtomicBool,
    input_monitoring: AtomicBool,
}

impl Default for ScriptedProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedProbe {
    /// A probe with the start readings above.
    pub fn new() -> Self {
        Self {
            secure_input: AtomicBool::new(false),
            talk_key_down: AtomicBool::new(false),
            chord_down: AtomicBool::new(false),
            slept_us: AtomicU64::new(0),
            session_active: AtomicBool::new(true),
            input_monitoring: AtomicBool::new(true),
        }
    }

    /// Secure Event Input is on (`true`) or off.
    pub fn set_secure_input(&self, on: bool) {
        self.secure_input.store(on, Ordering::SeqCst);
    }

    /// The physical key state that the next read gives.
    pub fn set_physical(&self, state: PhysicalState) {
        self.talk_key_down
            .store(state.talk_key_down, Ordering::SeqCst);
        self.chord_down.store(state.chord_down, Ordering::SeqCst);
    }

    /// The device slept for `d` (the slept total grows by `d`).
    pub fn sleep(&self, d: Duration) {
        let us = u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        self.slept_us.fetch_add(us, Ordering::SeqCst);
    }

    /// The session is on the console and unlocked (`true`), or not.
    pub fn set_session_active(&self, on: bool) {
        self.session_active.store(on, Ordering::SeqCst);
    }

    /// Input Monitoring is granted (`true`), or not.
    pub fn set_input_monitoring(&self, granted: bool) {
        self.input_monitoring.store(granted, Ordering::SeqCst);
    }
}

impl PlatformProbe for ScriptedProbe {
    fn secure_input_active(&self) -> bool {
        self.secure_input.load(Ordering::SeqCst)
    }

    fn physical_state(&self) -> PhysicalState {
        PhysicalState {
            talk_key_down: self.talk_key_down.load(Ordering::SeqCst),
            chord_down: self.chord_down.load(Ordering::SeqCst),
        }
    }

    fn slept(&self) -> Duration {
        Duration::from_micros(self.slept_us.load(Ordering::SeqCst))
    }

    fn session_active(&self) -> bool {
        self.session_active.load(Ordering::SeqCst)
    }

    fn input_monitoring_granted(&self) -> bool {
        self.input_monitoring.load(Ordering::SeqCst)
    }
}

/// One request of the engine to the OS layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapCall {
    /// [`TapControl::re_enable`].
    ReEnable,
    /// [`TapControl::recreate`].
    Recreate,
    /// [`TapControl::set_chord`] with its argument.
    SetChord(bool),
}

/// A [`TapControl`] that records the requests. The tap is alive unless the test says not.
#[derive(Debug)]
pub struct RecordingTapControl {
    calls: Mutex<Vec<TapCall>>,
    alive: AtomicBool,
}

impl Default for RecordingTapControl {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingTapControl {
    /// A control with a live tap and no calls.
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            alive: AtomicBool::new(true),
        }
    }

    /// Take the requests recorded since the last call.
    pub fn take(&self) -> Vec<TapCall> {
        std::mem::take(&mut *lock(&self.calls))
    }

    /// Whether a tap exists now ([`TapControl::tap_alive`]).
    pub fn set_alive(&self, alive: bool) {
        self.alive.store(alive, Ordering::SeqCst);
    }
}

impl TapControl for RecordingTapControl {
    fn re_enable(&self) {
        lock(&self.calls).push(TapCall::ReEnable);
    }

    fn recreate(&self) {
        lock(&self.calls).push(TapCall::Recreate);
    }

    fn set_chord(&self, on: bool) {
        lock(&self.calls).push(TapCall::SetChord(on));
    }

    fn tap_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

/// A [`PermissionApi`] that records each prompt. `status` never prompts. Each permission starts
/// `Unknown`; a `request` grants it unless the test set it to `Denied`.
#[derive(Debug, Default)]
pub struct FakePermissions {
    input_monitoring: Mutex<Option<PermissionStatus>>,
    accessibility: Mutex<Option<PermissionStatus>>,
    prompts: Mutex<Vec<Permission>>,
}

impl FakePermissions {
    /// No permission asked yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the status of `permission` with no prompt (the user changed it in Settings).
    pub fn set(&self, permission: Permission, status: PermissionStatus) {
        *lock(self.slot(permission)) = Some(status);
    }

    /// The permissions that showed a prompt, in order.
    pub fn prompts(&self) -> Vec<Permission> {
        lock(&self.prompts).clone()
    }

    fn slot(&self, permission: Permission) -> &Mutex<Option<PermissionStatus>> {
        match permission {
            Permission::InputMonitoring => &self.input_monitoring,
            Permission::Accessibility => &self.accessibility,
        }
    }
}

impl PermissionApi for FakePermissions {
    fn status(&self, permission: Permission) -> PermissionStatus {
        lock(self.slot(permission)).unwrap_or(PermissionStatus::Unknown)
    }

    fn request(&self, permission: Permission) -> PermissionStatus {
        lock(&self.prompts).push(permission);
        let mut slot = lock(self.slot(permission));
        let status = match *slot {
            Some(PermissionStatus::Denied) => PermissionStatus::Denied,
            _ => PermissionStatus::Granted,
        };
        *slot = Some(status);
        status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_30_fake_permissions_status_never_prompts_and_request_records() {
        let p = FakePermissions::new();
        assert_eq!(
            p.status(Permission::InputMonitoring),
            PermissionStatus::Unknown
        );
        assert_eq!(p.prompts(), vec![]);
        assert_eq!(
            p.request(Permission::InputMonitoring),
            PermissionStatus::Granted
        );
        p.set(Permission::Accessibility, PermissionStatus::Denied);
        assert_eq!(
            p.request(Permission::Accessibility),
            PermissionStatus::Denied
        );
        assert_eq!(
            p.prompts(),
            vec![Permission::InputMonitoring, Permission::Accessibility]
        );
    }

    #[test]
    fn at_30_scripted_probe_and_tap_control_record_what_the_test_sets() {
        let probe = ScriptedProbe::new();
        assert!(probe.session_active() && probe.input_monitoring_granted());
        probe.sleep(Duration::from_secs(2));
        probe.set_physical(PhysicalState {
            talk_key_down: true,
            chord_down: false,
        });
        assert_eq!(probe.slept(), Duration::from_secs(2));
        assert!(probe.physical_state().talk_key_down);
        let ctl = RecordingTapControl::new();
        ctl.set_chord(true);
        ctl.recreate();
        ctl.set_alive(false);
        assert!(!ctl.tap_alive());
        assert_eq!(ctl.take(), vec![TapCall::SetChord(true), TapCall::Recreate]);
        assert_eq!(ctl.take(), vec![]);
        let sink = RecordingSink::new();
        sink.emit(HotkeyOutput::Tap);
        assert_eq!(sink.take(), vec![HotkeyOutput::Tap]);
    }
}
