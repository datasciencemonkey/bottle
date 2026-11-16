//! Win32 glue for the Windows HotkeyService (#7). Compiled on Windows only.
//!
//! One dedicated thread owns the `WH_KEYBOARD_LL` hook and runs its message loop (CAP-40a).
//! The callback calls [`HookCore::handle`] (O(1), no lock, no I/O; the unbounded channel send
//! may allocate a block now and then) and returns. A mask replay is posted to the loop, so
//! `SendInput` never runs inside the callback. A 1 min `SetTimer` tick lets [`HookSupervisor`] install the hook again every
//! 10 min, and a suspend/resume notification installs it again after resume (CAP-40b). A
//! foreground WinEvent hook sends the foreground process id to a second thread, which reads
//! the token elevation and feeds [`ElevationGate`] (CAP-42). The foreground window at start is
//! probed once, so an elevated window that already has the foreground is found at once. A
//! message-only window gets `WTS_SESSION_UNLOCK`, and the hook reads the key state again on
//! unlock (CAP-45). If `GetMessageW` fails, the thread sends `HookFailed` and ends; the
//! service then counts as stopped, so a later `start()` installs a new hook (#313).
//!
//! TODO(#5): SP-03 must confirm on a Windows device that the hook misses the talk key while
//! an elevated window has the foreground (UIPI), and measure the real callback p99 (AT-30).
//! Until then this code is type-checked only (`cargo check --target x86_64-pc-windows-msvc`).

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{
    PowerRegisterSuspendResumeNotification, PowerUnregisterSuspendResumeNotification,
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
};
use windows_sys::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, KillTimer, PeekMessageW,
    PostThreadMessageW, RegisterClassW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx,
    UnregisterClassW, DEVICE_NOTIFY_CALLBACK, EVENT_SYSTEM_FOREGROUND, HC_ACTION, HHOOK,
    HWND_MESSAGE, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_UP, MSG, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, PM_NOREMOVE, WH_KEYBOARD_LL, WINEVENT_OUTOFCONTEXT, WM_APP, WM_QUIT,
    WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_SESSION_UNLOCK,
};

use super::{
    foreground_at_start, reinstall_signal, run_elevation_gate, unavailable, Elevation,
    ElevationGate, HookCore, HookInstaller, HookSignal, HookSupervisor, HookThread, KeyEvent,
    LatencyRecorder, LoopStep, TalkKey, Verdict,
};
use crate::clock::{MonotonicClock, SystemClock};
use crate::error::CoreResult;
use crate::hotkey::HotkeyService;

/// The supervisor tick. The policy decides when 10 min have passed.
const TICK_MS: u32 = 60_000;
/// Posted to the hook thread on resume from sleep.
const WM_APP_RESUME: u32 = WM_APP + 1;
/// Posted to the hook thread by the callback: replay a masked key-up (`wparam` = the key).
const WM_APP_REPLAY: u32 = WM_APP + 2;
/// The class name of the message-only window that gets the session notifications
/// (UTF-16, NUL-terminated).
const SESSION_CLASS: [u16; 20] = ascii_wide(b"BottleHotkeySession\0");
/// Samples kept for the p99.
const LATENCY_SAMPLES: usize = 10_000;

/// Widen an ASCII byte string to UTF-16 at compile time.
const fn ascii_wide<const N: usize>(s: &[u8; N]) -> [u16; N] {
    let mut out = [0u16; N];
    let mut i = 0;
    while i < N {
        out[i] = s[i] as u16;
        i += 1;
    }
    out
}

thread_local! {
    /// The hook state. A low-level hook is called on the thread that installed it, so the
    /// callback finds its state here without a lock.
    static CORE: RefCell<Option<HookCore>> = const { RefCell::new(None) };
    /// Where the foreground WinEvent callback sends the foreground process id.
    static FOREGROUND: RefCell<Option<Sender<u32>>> = const { RefCell::new(None) };
}

/// The Windows R1 HotkeyService (CAP-40a, CAP-40b, CAP-41, CAP-42, CAP-45).
///
/// The Tauri host creates and starts it (#314, `src-tauri/src/hotkey_host.rs`) and adapts its
/// signals. TODO(#14): the capture lifecycle calls `set_hands_free`; until then no caller turns
/// hands-free on.
pub struct WindowsHotkeyService {
    talk: TalkKey,
    signals: Sender<HookSignal>,
    hands_free: Arc<AtomicBool>,
    recorder: Arc<LatencyRecorder>,
    running: Mutex<Option<Running>>,
}

struct Running {
    thread_id: u32,
    join: JoinHandle<()>,
    /// Set to `false` by the hook thread when its loop ends, before it reports a failure and
    /// cleans up. So `start()` and `stop()` never treat an ending thread as live.
    alive: Arc<AtomicBool>,
}

impl WindowsHotkeyService {
    /// A stopped service for `talk`, and the receiver of its signals.
    pub fn new(talk: TalkKey) -> (Self, Receiver<HookSignal>) {
        let (signals, rx) = mpsc::channel();
        let service = Self {
            talk,
            signals,
            hands_free: Arc::new(AtomicBool::new(false)),
            recorder: Arc::new(LatencyRecorder::with_capacity(LATENCY_SAMPLES)),
            running: Mutex::new(None),
        };
        (service, rx)
    }

    /// Hands-free capture is on or off: Esc is swallowed while it is on (CAP-41).
    pub fn set_hands_free(&self, on: bool) {
        self.hands_free.store(on, Ordering::Relaxed);
    }

    /// The p99 of the recent hook callbacks (AT-30, CAP-40a).
    pub fn callback_p99(&self) -> Option<Duration> {
        self.recorder.p99()
    }
}

impl HotkeyService for WindowsHotkeyService {
    fn start(&self) -> CoreResult<()> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        let state = thread_state(running.as_ref());
        if !state.start_spawns() {
            return Ok(());
        }
        if let Some(finished) = running.take() {
            // The loop ended by itself (it sent HookFailed): reap it, then start a new hook.
            let _ = finished.join.join();
        }
        let ctx = ThreadCtx {
            talk: self.talk,
            signals: self.signals.clone(),
            hands_free: self.hands_free.clone(),
            recorder: self.recorder.clone(),
            alive: Arc::new(AtomicBool::new(true)),
        };
        let alive = ctx.alive.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("bottle-hotkey".into())
            .spawn(move || hook_thread(ctx, ready_tx))
            .map_err(|_| unavailable("hook_thread_spawn_failed"))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => {
                *running = Some(Running {
                    thread_id,
                    join,
                    alive,
                });
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                let _ = join.join();
                Err(unavailable("hook_thread_exited"))
            }
        }
    }

    fn stop(&self) -> CoreResult<()> {
        let running = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(r) = running {
            if !thread_state(Some(&r)).stop_posts_quit() {
                // The loop ended by itself: there is no queue to post to, only a thread to join.
                let _ = r.join.join();
                return Ok(());
            }
            // SAFETY: posting a message to a thread id has no memory-safety requirement.
            if unsafe { PostThreadMessageW(r.thread_id, WM_QUIT, 0, 0) } == 0 {
                // The thread did not get the quit message: do not wait on it forever.
                *self.running.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
                return Err(unavailable("stop_post_failed"));
            }
            let _ = r.join.join();
        }
        Ok(())
    }

    /// Windows has no Secure Event Input; the elevated-window case is CAP-42.
    fn secure_input_active(&self) -> CoreResult<bool> {
        Ok(false)
    }
}

impl Drop for WindowsHotkeyService {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn thread_state(running: Option<&Running>) -> HookThread {
    HookThread::of(
        running.is_some(),
        running.is_some_and(|r| r.join.is_finished() || !r.alive.load(Ordering::Acquire)),
    )
}

struct ThreadCtx {
    alive: Arc<AtomicBool>,
    talk: TalkKey,
    signals: Sender<HookSignal>,
    hands_free: Arc<AtomicBool>,
    recorder: Arc<LatencyRecorder>,
}

struct Win32Installer;

impl HookInstaller for Win32Installer {
    type Handle = HHOOK;

    fn install(&mut self) -> Result<HHOOK, &'static str> {
        // SAFETY: `ll_keyboard_proc` has the HOOKPROC signature; a null module name returns
        // the handle of this executable.
        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(ll_keyboard_proc),
                GetModuleHandleW(null()),
                0,
            )
        };
        if hook.is_null() {
            return Err("set_windows_hook_failed");
        }
        Ok(hook)
    }

    fn uninstall(&mut self, hook: HHOOK) {
        // SAFETY: `hook` came from SetWindowsHookExW; the supervisor removes each one once.
        unsafe { UnhookWindowsHookEx(hook) };
    }
}

fn hook_thread(ctx: ThreadCtx, ready: SyncSender<CoreResult<u32>>) {
    let clock: Arc<SystemClock> = Arc::new(SystemClock::new());
    // SAFETY: plain Win32 calls on this thread with valid out-pointers.
    let thread_id = unsafe {
        let mut msg: MSG = std::mem::zeroed();
        // Make the thread's message queue before anyone posts to it.
        PeekMessageW(&mut msg, null_mut(), WM_APP, WM_APP, PM_NOREMOVE);
        GetCurrentThreadId()
    };
    CORE.with(|c| {
        *c.borrow_mut() = Some(HookCore::new(
            ctx.talk,
            ctx.hands_free,
            ctx.recorder,
            clock.clone(),
            ctx.signals.clone(),
        ));
    });

    let mut supervisor = HookSupervisor::new(Win32Installer);
    if let Err(e) = supervisor.start(clock.now()) {
        CORE.with(|c| c.borrow_mut().take());
        let _ = ready.send(Err(e));
        return;
    }

    let elevation = spawn_elevation_worker(ctx.signals);
    FOREGROUND.with(|f| *f.borrow_mut() = elevation.as_ref().map(|(tx, _)| tx.clone()));

    // SAFETY: the callbacks have the WINEVENTPROC / PDEVICE_NOTIFY_CALLBACK_ROUTINE
    // signatures. `params` lives on this stack until it is unregistered below.
    let mut params = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(on_power),
        Context: thread_id as usize as *mut c_void,
    };
    let (timer, foreground_hook, power, session) = unsafe {
        let timer = SetTimer(null_mut(), 0, TICK_MS, None);
        let foreground_hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            null_mut(),
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        let mut power: *mut c_void = null_mut();
        let status = PowerRegisterSuspendResumeNotification(
            DEVICE_NOTIFY_CALLBACK,
            &mut params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS as HANDLE,
            &mut power,
        );
        (
            timer,
            foreground_hook,
            if status == 0 { power } else { null_mut() },
            register_session_window(),
        )
    };
    if timer == 0 {
        // Without the tick there is no periodic reinstall (CAP-40b): refuse to start.
        // SAFETY: each handle came from the matching register call above.
        unsafe {
            if !foreground_hook.is_null() {
                UnhookWinEvent(foreground_hook);
            }
            if !power.is_null() {
                PowerUnregisterSuspendResumeNotification(power as isize);
            }
        }
        unregister_session_window(session);
        supervisor.stop();
        FOREGROUND.with(|f| f.borrow_mut().take());
        CORE.with(|c| c.borrow_mut().take());
        let _ = ready.send(Err(unavailable("set_timer_failed")));
        if let Some((tx, join)) = elevation {
            drop(tx);
            let _ = join.join();
        }
        return;
    }
    let _ = ready.send(Ok(thread_id));
    if power.is_null() {
        send_signal(HookSignal::HookFailed {
            reason: "resume_notification_failed",
        });
    }
    if foreground_hook.is_null() {
        send_signal(HookSignal::HookFailed {
            reason: "foreground_hook_failed",
        });
    }
    if session.is_null() {
        send_signal(HookSignal::HookFailed {
            reason: "session_notification_failed",
        });
    }
    // CAP-42: an elevated window that has the foreground now sends no foreground event, so
    // probe it once. The WinEvent callbacks run only inside the loop below, so this id is
    // the first one that the elevation worker gets.
    // SAFETY: GetForegroundWindow takes no argument; `pid` is a valid out-pointer.
    let start_pid = unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
        pid
    };
    if let (Some(pid), Some((tx, _))) = (foreground_at_start(start_pid), elevation.as_ref()) {
        let _ = tx.send(pid);
    }

    loop {
        // SAFETY: `msg` is a valid out-pointer; a null HWND reads thread messages too.
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        let got = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        let step = LoopStep::from_get_message(got);
        if step != LoopStep::Dispatch || msg.message == WM_QUIT {
            // Mark the thread as ending first, so a `start()` that the signal below triggers
            // joins this thread and installs a new hook instead of seeing a live one.
            ctx.alive.store(false, Ordering::Release);
            // A failed loop must not leave a dead hook that looks alive: report it, then
            // clean up. `start()` then sees a finished thread and starts a new one.
            if let Some(signal) = step.exit_signal() {
                send_signal(signal);
            }
            break;
        }
        match msg.message {
            WM_TIMER if msg.hwnd.is_null() => {
                let result = supervisor.on_timer(clock.now());
                if matches!(result, Ok(true)) {
                    reconcile_passed_keys();
                }
                if let Some(signal) = reinstall_signal(&result) {
                    send_signal(signal);
                }
            }
            WM_APP_RESUME => {
                // A hold cannot last through sleep: end it before the new hook goes in.
                with_core(HookCore::on_resume);
                let result = supervisor.on_resume(clock.now()).map(|()| true);
                if let Some(signal) = reinstall_signal(&result) {
                    send_signal(signal);
                }
            }
            WM_APP_REPLAY => replay_with_mask(msg.wParam as u32),
            _ => {
                // SAFETY: `msg` came from GetMessageW.
                unsafe { DispatchMessageW(&msg) };
            }
        }
    }

    // SAFETY: each handle came from the matching register call above and is removed once.
    unsafe {
        if timer != 0 {
            KillTimer(null_mut(), timer);
        }
        if !foreground_hook.is_null() {
            UnhookWinEvent(foreground_hook);
        }
        if !power.is_null() {
            PowerUnregisterSuspendResumeNotification(power as isize);
        }
    }
    unregister_session_window(session);
    supervisor.stop();
    FOREGROUND.with(|f| f.borrow_mut().take());
    CORE.with(|c| c.borrow_mut().take());
    if let Some((tx, join)) = elevation {
        drop(tx);
        let _ = join.join();
    }
}

/// Make a message-only window on this thread and register it for the session notifications
/// (CAP-45). Returns null if a step fails; the window and the class are then removed.
fn register_session_window() -> HWND {
    // SAFETY: valid class data with a static NUL-terminated name; each handle is checked
    // and released once on failure.
    unsafe {
        let module = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(on_session_message),
            hInstance: module,
            lpszClassName: SESSION_CLASS.as_ptr(),
            ..std::mem::zeroed()
        };
        // A second start registers the class again; that fails with "class exists", which
        // is fine, so the result is not checked. CreateWindowExW fails if there is no class.
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            0,
            SESSION_CLASS.as_ptr(),
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
        if hwnd.is_null() {
            UnregisterClassW(SESSION_CLASS.as_ptr(), module);
            return null_mut();
        }
        if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) == 0 {
            DestroyWindow(hwnd);
            UnregisterClassW(SESSION_CLASS.as_ptr(), module);
            return null_mut();
        }
        hwnd
    }
}

/// Undo [`register_session_window`]. A null `hwnd` does nothing.
fn unregister_session_window(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    // SAFETY: `hwnd` came from register_session_window on this thread and is released once.
    unsafe {
        WTSUnRegisterSessionNotification(hwnd);
        DestroyWindow(hwnd);
        UnregisterClassW(SESSION_CLASS.as_ptr(), GetModuleHandleW(null()));
    }
}

/// The window procedure of the session window. It runs on the hook thread (the window
/// belongs to it), so it reaches the hook state without a lock.
unsafe extern "system" fn on_session_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_WTSSESSION_CHANGE && wparam as u32 == WTS_SESSION_UNLOCK {
        // CAP-45: read the key state again after the lock gap.
        with_core(|core| {
            // SAFETY: GetAsyncKeyState takes any virtual-key code.
            core.on_unlock(|vk| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0);
        });
        return 0;
    }
    // SAFETY: the arguments came from Windows for this window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Run `f` on the hook state of this thread, if it is there and not in use.
fn with_core(f: impl FnOnce(&mut HookCore)) {
    CORE.with(|c| {
        if let Ok(mut core) = c.try_borrow_mut() {
            if let Some(core) = core.as_mut() {
                f(core);
            }
        }
    });
}

fn send_signal(signal: HookSignal) {
    with_core(|core| core.send(signal));
}

/// After a reinstall, read the key state again for the keys whose key-down reached Windows
/// (see [`HookCore::reconcile`]; swallowed keys are skipped).
fn reconcile_passed_keys() {
    with_core(|core| {
        // SAFETY: GetAsyncKeyState takes any virtual-key code.
        core.reconcile(|vk| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0);
    });
}

unsafe extern "system" fn ll_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam != 0 {
        // SAFETY: for HC_ACTION, `lparam` points to a KBDLLHOOKSTRUCT owned by the system.
        let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let ev = KeyEvent {
            vk: kb.vkCode,
            down: kb.flags & LLKHF_UP == 0,
            injected: kb.flags & LLKHF_INJECTED != 0,
        };
        let verdict = CORE.with(|c| {
            c.try_borrow_mut()
                .ok()
                .and_then(|mut core| core.as_mut().map(|core| core.handle(ev)))
        });
        match verdict {
            Some(Verdict::Swallow) => return 1,
            Some(Verdict::SwallowAndReplay { vk }) => {
                // Replay after the callback returns, on the hook thread's loop. If the post
                // fails, pass the key-up as it is, so Windows never keeps the key down.
                // SAFETY: posting a message to this thread has no memory requirement.
                let posted = unsafe {
                    PostThreadMessageW(GetCurrentThreadId(), WM_APP_REPLAY, vk as usize, 0)
                };
                if posted != 0 {
                    return 1;
                }
            }
            _ => {}
        }
    }
    // SAFETY: the hook handle argument is ignored by Windows.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

/// Inject a tap of the mask key, then the key-up of `vk`. The injected events pass the
/// filter, so the app and the shell see a complete key with something in between.
fn replay_with_mask(vk: u32) {
    let key = |vk: u32, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk as u16,
                wScan: 0,
                dwFlags: (if up { KEYEVENTF_KEYUP } else { 0 })
                    | (if is_extended(vk) {
                        KEYEVENTF_EXTENDEDKEY
                    } else {
                        0
                    }),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [
        key(super::vk::MASK, false),
        key(super::vk::MASK, true),
        key(vk, true),
    ];
    // SAFETY: `inputs` is a valid array of INPUT of the given length and size.
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
}

/// The Win keys are extended keys.
fn is_extended(vk: u32) -> bool {
    vk == super::vk::LWIN || vk == super::vk::RWIN
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out-pointer; a stale HWND gives pid 0.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid != 0 {
        FOREGROUND.with(|f| {
            if let Some(tx) = f.borrow().as_ref() {
                let _ = tx.send(pid);
            }
        });
    }
}

unsafe extern "system" fn on_power(
    context: *const c_void,
    kind: u32,
    _setting: *const c_void,
) -> u32 {
    if kind == PBT_APMRESUMEAUTOMATIC || kind == PBT_APMRESUMESUSPEND {
        // SAFETY: `context` carries the hook thread id; posting has no memory requirement.
        unsafe { PostThreadMessageW(context as usize as u32, WM_APP_RESUME, 0, 0) };
    }
    0
}

/// The elevation probe runs off the hook thread, so a slow token query never delays a key.
fn spawn_elevation_worker(signals: Sender<HookSignal>) -> Option<(Sender<u32>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel::<u32>();
    let join = std::thread::Builder::new()
        .name("bottle-hotkey-elevation".into())
        .spawn(move || {
            // SAFETY: GetCurrentProcess returns a pseudo handle that needs no close.
            let self_elevated = token_elevated(unsafe { GetCurrentProcess() }) == Some(true);
            let mut gate = ElevationGate::new(self_elevated);
            run_elevation_gate(&mut gate, rx, process_elevation, &signals);
        })
        .ok()?;
    Some((tx, join))
}

fn process_elevation(pid: u32) -> Elevation {
    // SAFETY: OpenProcess returns null on failure; the handle is closed once.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Elevation::from_probe(false, None);
        }
        let e = Elevation::from_probe(true, token_elevated(process));
        CloseHandle(process);
        e
    }
}

/// The token elevation of `process`, or `None` when the token query is refused.
fn token_elevated(process: HANDLE) -> Option<bool> {
    // SAFETY: valid out-pointers and sizes; the token handle is closed once.
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut info = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut info as *mut TOKEN_ELEVATION as *mut c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        (ok != 0).then_some(info.TokenIsElevated != 0)
    }
}
