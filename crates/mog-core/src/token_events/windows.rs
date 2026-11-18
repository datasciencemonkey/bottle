//! The Windows sleep and lock source (#465, ADR 0076). Compiled on Windows only.
//!
//! One thread per watch owns a message-only window that is registered for the session
//! notifications (`WTS_SESSION_LOCK`), and runs its message loop. The suspend notification
//! (`PBT_APMSUSPEND`) arrives on a system thread through `PowerRegisterSuspendResumeNotification`
//! and calls the sink there at once, before Windows sleeps. Dropping the watch posts `WM_QUIT`
//! to the thread and joins it; the thread removes both registrations and the window.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, SyncSender};
use std::thread;

use windows_sys::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{
    PowerRegisterSuspendResumeNotification, PowerUnregisterSuspendResumeNotification,
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
};
use windows_sys::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PeekMessageW,
    PostThreadMessageW, RegisterClassW, UnregisterClassW, DEVICE_NOTIFY_CALLBACK, HWND_MESSAGE,
    MSG, PM_NOREMOVE, WM_APP, WM_QUIT, WM_WTSSESSION_CHANGE, WNDCLASSW,
};

use super::{
    unavailable, win_power_event, win_session_event, SessionEventSource, SessionSink, SessionWatch,
    WIN_SESSION_CLASS,
};
use crate::error::CoreResult;

/// The Windows source.
pub(super) struct WinSessionEvents;

thread_local! {
    /// The sink of the watch thread. The window procedure runs on that thread.
    static SINK: RefCell<Option<SessionSink>> = const { RefCell::new(None) };
}

fn class_name() -> Vec<u16> {
    WIN_SESSION_CLASS.encode_utf16().chain([0]).collect()
}

impl SessionEventSource for WinSessionEvents {
    fn watch(&self, sink: SessionSink) -> CoreResult<SessionWatch> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("bottle-token-session".into())
            .spawn(move || session_thread(sink, ready_tx))
            .map_err(|_| unavailable("session_events_failed"))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(SessionWatch::on_drop(move || {
                // SAFETY: posting to a thread id has no memory requirement.
                unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
                let _ = join.join();
            })),
            Ok(Err(reason)) => {
                let _ = join.join();
                Err(unavailable(reason))
            }
            Err(_) => {
                let _ = join.join();
                Err(unavailable("session_events_failed"))
            }
        }
    }
}

/// The handles of one watch thread, removed once in [`Registrations::remove`].
struct Registrations {
    window: HWND,
    session: bool,
    power: *mut c_void,
}

impl Registrations {
    fn remove(self, class: &[u16]) {
        // SAFETY: each handle came from the matching register call on this thread and is
        // removed once.
        unsafe {
            if !self.power.is_null() {
                PowerUnregisterSuspendResumeNotification(self.power as isize);
            }
            if self.session {
                WTSUnRegisterSessionNotification(self.window);
            }
            if !self.window.is_null() {
                DestroyWindow(self.window);
            }
            UnregisterClassW(class.as_ptr(), GetModuleHandleW(null()));
        }
    }
}

fn session_thread(sink: SessionSink, ready: SyncSender<Result<u32, &'static str>>) {
    let class = class_name();
    // SAFETY: valid out-pointer; this makes the thread's message queue before anyone posts.
    let thread_id = unsafe {
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, null_mut(), WM_APP, WM_APP, PM_NOREMOVE);
        GetCurrentThreadId()
    };
    SINK.with(|s| *s.borrow_mut() = Some(sink.clone()));
    // The power callback gets the sink through this pointer. It stays valid until the
    // registration is removed below, and is freed after that.
    let power_sink: *mut SessionSink = Box::into_raw(Box::new(sink));
    let mut params = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(on_power),
        Context: power_sink as *mut c_void,
    };
    let mut reg = Registrations {
        window: null_mut(),
        session: false,
        power: null_mut(),
    };
    // SAFETY: valid class data with a NUL-terminated name; each handle is checked. `params`
    // lives on this stack until the registration is removed.
    let failed = unsafe {
        let module = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(on_window),
            hInstance: module,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        // A second watch registers the class again; that fails with "class exists", which is
        // fine. CreateWindowExW fails if there is no class.
        RegisterClassW(&wc);
        reg.window = CreateWindowExW(
            0,
            class.as_ptr(),
            null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            module,
            null(),
        );
        if reg.window.is_null()
            || WTSRegisterSessionNotification(reg.window, NOTIFY_FOR_THIS_SESSION) == 0
        {
            Some("lock_notification_failed")
        } else {
            reg.session = true;
            let status = PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                &mut params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS as HANDLE,
                &mut reg.power,
            );
            if status != 0 {
                reg.power = null_mut();
                Some("sleep_notification_failed")
            } else {
                None
            }
        }
    };
    if let Some(reason) = failed {
        reg.remove(&class);
        finish(power_sink);
        let _ = ready.send(Err(reason));
        return;
    }
    let _ = ready.send(Ok(thread_id));
    loop {
        // SAFETY: `msg` is a valid out-pointer; a null HWND reads thread messages too.
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        let got = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if got <= 0 || msg.message == WM_QUIT {
            break;
        }
        // SAFETY: `msg` came from GetMessageW.
        unsafe { DispatchMessageW(&msg) };
    }
    reg.remove(&class);
    finish(power_sink);
}

/// Free the power sink (after its registration is gone) and clear the thread sink.
fn finish(power_sink: *mut SessionSink) {
    // SAFETY: the pointer came from Box::into_raw above, and no callback can use it now.
    drop(unsafe { Box::from_raw(power_sink) });
    SINK.with(|s| s.borrow_mut().take());
}

/// The window procedure. It runs on the watch thread, which owns the window.
unsafe extern "system" fn on_window(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_WTSSESSION_CHANGE {
        if let Some(event) = win_session_event(wparam as u32) {
            SINK.with(|s| {
                if let Some(sink) = s.borrow().as_ref() {
                    sink(event);
                }
            });
        }
        return 0;
    }
    // SAFETY: the arguments came from Windows for this window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The suspend and resume callback. It runs on a system thread, so it uses the sink from its
/// context, and it drops the tokens before it returns (Windows waits for it before sleep).
unsafe extern "system" fn on_power(
    context: *const c_void,
    kind: u32,
    _setting: *const c_void,
) -> u32 {
    if let Some(event) = win_power_event(kind) {
        // SAFETY: `context` is the boxed sink of the watch, valid while it is registered.
        let sink = unsafe { &*(context as *const SessionSink) };
        sink(event);
    }
    0
}
