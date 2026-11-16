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

/// The R1 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubHotkeyService;

impl HotkeyService for StubHotkeyService {
    fn start(&self) -> CoreResult<()> {
        // TODO(#6): macOS listen-only CGEventTap and watchdog (Windows hook is #7).
        Err(CoreError::NotImplemented { component: ID })
    }

    fn stop(&self) -> CoreResult<()> {
        // TODO(#6): remove the tap and stop the watchdog.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn secure_input_active(&self) -> CoreResult<bool> {
        // TODO(#6): Secure Event Input detection.
        Err(CoreError::NotImplemented { component: ID })
    }
}
