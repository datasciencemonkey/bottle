//! The host side of call recording capture (ADR 0103, Refs #65 #107 #108).
//!
//! One [`RecordingHost`] per process owns the [`RecordingSession`] over the app's one R2
//! [`CaptureHost`] core (CAP-01a). It:
//!
//! - holds the recording knob (`settings.enabled`), OFF at launch. The session checks it in
//!   Rust before any capture path starts. Turning it ON asks for the Microphone and System
//!   Audio Recording permissions ([`ask_on_enable`]).
//! - relays each R2 lease stop to the session (mic loss, CAP-17b zero buffers, the MB-05
//!   panic stop), so the recording finalizes its parts;
//! - runs a ticker thread that calls [`RecordingSession::tick`] every [`TICK_EVERY`] (the
//!   length preset and the CAP-10a 5 s check);
//! - stops the recording on OS sleep ([`RecordingHost::watch_sleep`]) and on quit
//!   ([`RecordingHost::shutdown`], before R2 shuts down).
//!
//! [`system_audio_device`] builds the system audio device of this OS for the app's
//! [`mog_core::audio_device::system::RecordingDevice`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use mog_core::audio_device::system::{SystemAudioDevice, SystemAudioStatus};
use mog_core::capture::{AudioDevice, CaptureService, Stopped};
use mog_core::clock::{MonotonicClock, WallClock};
use mog_core::error::CoreResult;
use mog_core::recording_capture::permissions::{ask_on_enable, PermissionAsker};
use mog_core::recording_capture::session::{
    PartsHandoff, RecordingSession, SessionConfig, StopCause,
};
use mog_core::recording_capture::tap::RecordingTap;
use mog_core::token_events::{SessionEventSource, SessionWatch};

use crate::capture_host::CaptureHost;

/// How often the ticker applies the length preset and the CAP-10a check.
pub const TICK_EVERY: Duration = Duration::from_millis(250);

/// The system audio device of this OS: the process tap on macOS, WASAPI loopback on Windows,
/// none elsewhere. Its frames go to `tap`. A device whose thread does not start is the
/// unavailable device (the recording goes on mic-only).
pub fn system_audio_device(tap: &RecordingTap) -> (Box<dyn AudioDevice>, SystemAudioStatus) {
    use mog_core::audio_device::system::OPEN_TIMEOUT;
    #[cfg(target_os = "macos")]
    let spawned = SystemAudioDevice::spawn(
        mog_core::audio_device::system_macos::TapHost::new,
        Box::new(tap.system_sink()),
        OPEN_TIMEOUT,
    );
    #[cfg(target_os = "windows")]
    let spawned = SystemAudioDevice::spawn(
        mog_core::audio_device::system_windows::LoopbackHost::new,
        Box::new(tap.system_sink()),
        OPEN_TIMEOUT,
    );
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let spawned: std::io::Result<(SystemAudioDevice, SystemAudioStatus)> = {
        let _ = (tap, OPEN_TIMEOUT);
        Ok(SystemAudioDevice::unavailable())
    };
    let (device, status) = spawned.unwrap_or_else(|_| SystemAudioDevice::unavailable());
    (Box::new(device), status)
}

/// The permission asks of this OS (ADR 0103): macOS asks; Windows and Linux ask nothing.
pub fn os_permission_asker() -> Box<dyn PermissionAsker + Send + Sync> {
    #[cfg(target_os = "macos")]
    {
        Box::new(mog_core::audio_device::system_macos::MacPermissionAsker)
    }
    #[cfg(not(target_os = "macos"))]
    {
        struct NoAsk;
        impl PermissionAsker for NoAsk {
            fn ask_mic(&self) {}
            fn ask_system_audio(&self) {}
        }
        Box::new(NoAsk)
    }
}

struct Ticker {
    stop: Mutex<bool>,
    wake: Condvar,
}

/// See the [module docs](self).
pub struct RecordingHost {
    session: Arc<RecordingSession>,
    enabled: Arc<AtomicBool>,
    ticker: Arc<Ticker>,
    thread: Mutex<Option<JoinHandle<()>>>,
    sleep_watch: Mutex<Option<SessionWatch>>,
}

impl RecordingHost {
    /// The session over `capture`'s core, with the knob OFF. `tap` and `system` must be the
    /// tap and system audio status of the core's device. Starts the ticker thread.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        capture: &CaptureHost,
        tap: RecordingTap,
        system: SystemAudioStatus,
        clock: Arc<dyn MonotonicClock>,
        wall: Arc<dyn WallClock>,
        handoff: Arc<dyn PartsHandoff>,
        config: SessionConfig,
    ) -> std::io::Result<Self> {
        let enabled = Arc::new(AtomicBool::new(false));
        let knob = Arc::clone(&enabled);
        let session = RecordingSession::new(
            Arc::clone(capture.service()) as Arc<dyn CaptureService>,
            tap,
            system,
            clock,
            wall,
            Arc::new(move || knob.load(Ordering::SeqCst)),
            handoff,
            config,
        );
        // The observer holds the session weakly: the host owns it.
        let weak = Arc::downgrade(&session);
        capture.add_stop_observer(Arc::new(move |stopped: &Stopped| {
            if let Some(s) = weak.upgrade() {
                s.lease_stopped(stopped.lease_id, stopped.reason);
            }
        }));
        let ticker = Arc::new(Ticker {
            stop: Mutex::new(false),
            wake: Condvar::new(),
        });
        let thread = {
            let ticker = Arc::clone(&ticker);
            let weak = Arc::downgrade(&session);
            std::thread::Builder::new()
                .name("bottle-rec-ticker".into())
                .spawn(move || run_ticker(&ticker, &weak))?
        };
        Ok(Self {
            session,
            enabled,
            ticker,
            thread: Mutex::new(Some(thread)),
            sleep_watch: Mutex::new(None),
        })
    }

    /// The recording session.
    pub fn session(&self) -> &Arc<RecordingSession> {
        &self.session
    }

    /// The knob now.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Set the knob. OFF to ON asks for the permissions through `asker`. Turning it OFF does
    /// not stop a running recording; it blocks the next start.
    pub fn set_enabled(&self, on: bool, asker: &dyn PermissionAsker) {
        let was = self.enabled.swap(on, Ordering::SeqCst);
        ask_on_enable(was, on, asker);
    }

    /// Stop the recording on each OS sleep event of `source`, for the host's lifetime.
    pub fn watch_sleep(&self, source: &dyn SessionEventSource) -> CoreResult<()> {
        let weak = Arc::downgrade(&self.session);
        let watch = source.watch(Arc::new(move |event| {
            if let Some(s) = weak.upgrade() {
                s.session_event(event);
            }
        }))?;
        *self
            .sleep_watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(watch);
        Ok(())
    }

    /// On quit: no new recording, the running one stops with `quit` and hands off its parts,
    /// and the ticker ends. Call it before [`CaptureHost::shutdown`]. Safe to call twice.
    pub fn shutdown(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        let _ = self.session.stop(StopCause::Quit);
        *self
            .ticker
            .stop
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = true;
        self.ticker.wake.notify_all();
        let handle = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(h) = handle {
            let _ = h.join();
        }
        self.sleep_watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

impl Drop for RecordingHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_ticker(ticker: &Ticker, session: &std::sync::Weak<RecordingSession>) {
    let mut stop = ticker.stop.lock().unwrap_or_else(PoisonError::into_inner);
    while !*stop {
        stop = ticker
            .wake
            .wait_timeout(stop, TICK_EVERY)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
        if *stop {
            break;
        }
        drop(stop);
        if let Some(s) = session.upgrade() {
            // A panic in an observer must not end the ticker.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.tick()));
        }
        stop = ticker.stop.lock().unwrap_or_else(PoisonError::into_inner);
    }
}
