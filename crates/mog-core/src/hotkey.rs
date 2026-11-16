//! R1 HotkeyService (section 4.3).
//!
//! Responsibilities: Global PTT, panic and session keys. Listen-only CGEventTap on macOS (Input Monitoring), `WH_KEYBOARD_LL` on Windows, Carbon / `RegisterHotKey` fallback chords. Watchdogs, Secure Event Input detection, target toast.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R1";

/// Global keys: PTT, panic and session keys, with watchdogs and Secure Event Input detection.
pub trait HotkeyService: Send + Sync {
    /// Install the listen-only key tap (or the fallback chords) and its watchdog.
    fn start(&self) -> CoreResult<()>;
    /// Remove the key tap and stop the watchdog.
    fn stop(&self) -> CoreResult<()>;
    /// Report whether Secure Event Input is on, so the keys cannot be seen.
    fn secure_input_active(&self) -> CoreResult<bool>;
}

/// The R1 stub for a platform with no hook yet. Every method returns
/// [`CoreError::NotImplemented`]. macOS uses `hotkey_macos::MacHotkeyService` (#6); the
/// platform-neutral logic is in [`crate::hotkey_core`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubHotkeyService;

impl HotkeyService for StubHotkeyService {
    fn start(&self) -> CoreResult<()> {
        // TODO(#7): the Windows hook. macOS is `hotkey_macos::MacHotkeyService` (#6).
        Err(CoreError::NotImplemented { component: ID })
    }

    fn stop(&self) -> CoreResult<()> {
        // TODO(#7): remove the Windows hook.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn secure_input_active(&self) -> CoreResult<bool> {
        // TODO(#7): the Windows elevated-window check.
        Err(CoreError::NotImplemented { component: ID })
    }
}
