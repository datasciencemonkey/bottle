//! Win32 glue for the Windows HotkeyService (#7). Compiled on Windows only.
//!
//! One dedicated thread owns the `WH_KEYBOARD_LL` hook and runs its message loop (CAP-40a).
//! The callback calls [`HookCore::handle`] (O(1), no lock, no allocation on the hot path)
//! and returns. A 1 min `SetTimer` tick lets [`HookSupervisor`] install the hook again every
//! 10 min, and a suspend/resume notification installs it again after resume (CAP-40b). A
//! foreground WinEvent hook sends the foreground process id to a second thread, which reads
//! the token elevation and feeds [`ElevationGate`] (CAP-42).
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
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, GetWindowThreadProcessId, KillTimer,
    PeekMessageW, PostThreadMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx,
    DEVICE_NOTIFY_CALLBACK, EVENT_SYSTEM_FOREGROUND, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_INJECTED, LLKHF_UP, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PM_NOREMOVE,
    WH_KEYBOARD_LL, WINEVENT_OUTOFCONTEXT, WM_APP, WM_QUIT, WM_TIMER,
};

use super::{
    unavailable, Elevation, ElevationGate, HookCore, HookInstaller, HookSignal, HookSupervisor,
    KeyEvent, LatencyRecorder, TalkKey, Verdict,
};
use crate::clock::{MonotonicClock, SystemClock};
use crate::error::CoreResult;
use crate::hotkey::HotkeyService;

/// The supervisor tick. The policy decides when 10 min have passed.
const TICK_MS: u32 = 60_000;
/// Posted to the hook thread on resume from sleep.
const WM_APP_RESUME: u32 = WM_APP + 1;
/// Samples kept for the p99.
const LATENCY_SAMPLES: usize = 10_000;

thread_local! {
    /// The hook state. A low-level hook is called on the thread that installed it, so the
    /// callback finds its state here without a lock.
    static CORE: RefCell<Option<HookCore>> = const { RefCell::new(None) };
    /// Where the foreground WinEvent callback sends the foreground process id.
    static FOREGROUND: RefCell<Option<Sender<u32>>> = const { RefCell::new(None) };
}

/// The Windows R1 HotkeyService (CAP-40a, CAP-40b, CAP-41, CAP-42).
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
        if running.is_some() {
            return Ok(());
        }
        let ctx = ThreadCtx {
            talk: self.talk,
            signals: self.signals.clone(),
            hands_free: self.hands_free.clone(),
            recorder: self.recorder.clone(),
        };
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("bottle-hotkey".into())
            .spawn(move || hook_thread(ctx, ready_tx))
            .map_err(|_| unavailable("hook_thread_spawn_failed"))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => {
                *running = Some(Running { thread_id, join });
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
            // SAFETY: posting a message to a thread id has no memory-safety requirement.
            unsafe { PostThreadMessageW(r.thread_id, WM_QUIT, 0, 0) };
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

struct ThreadCtx {
    talk: TalkKey,
    signals: Sender<HookSignal>,
    hands_free: Arc<AtomicBool>,
    recorder: Arc<LatencyRecorder>,
}

struct Win32Installer {
    hook: HHOOK,
}

impl HookInstaller for Win32Installer {
    fn install(&mut self) -> Result<(), &'static str> {
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
        self.hook = hook;
        Ok(())
    }

    fn uninstall(&mut self) {
        if !self.hook.is_null() {
            // SAFETY: `self.hook` came from SetWindowsHookExW and is removed once.
            unsafe { UnhookWindowsHookEx(self.hook) };
            self.hook = null_mut();
        }
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

    let mut supervisor = HookSupervisor::new(Win32Installer { hook: null_mut() });
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
    let (timer, foreground_hook, power) = unsafe {
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
        )
    };
    let _ = ready.send(Ok(thread_id));

    loop {
        // SAFETY: `msg` is a valid out-pointer; a null HWND reads thread messages too.
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        let got = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if got == 0 || got == -1 || msg.message == WM_QUIT {
            break;
        }
        let reinstalled = match msg.message {
            WM_TIMER if msg.hwnd.is_null() => supervisor.on_timer(clock.now()).unwrap_or(false),
            WM_APP_RESUME => supervisor.on_resume(clock.now()).is_ok(),
            _ => {
                // SAFETY: `msg` came from GetMessageW.
                unsafe { DispatchMessageW(&msg) };
                false
            }
        };
        if reinstalled {
            after_reinstall();
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
    supervisor.stop();
    FOREGROUND.with(|f| f.borrow_mut().take());
    CORE.with(|c| c.borrow_mut().take());
    if let Some((tx, join)) = elevation {
        drop(tx);
        let _ = join.join();
    }
}

/// After a reinstall, read the physical key state again (a key-up may have been lost while
/// the old hook was gone) and report the reinstall.
fn after_reinstall() {
    CORE.with(|c| {
        if let Ok(mut core) = c.try_borrow_mut() {
            if let Some(core) = core.as_mut() {
                // SAFETY: GetAsyncKeyState takes any virtual-key code.
                core.reconcile(|vk| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0);
                core.send(HookSignal::HookReinstalled);
            }
        }
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
                replay_with_mask(vk);
                return 1;
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
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
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
            let self_elevated =
                token_elevation(unsafe { GetCurrentProcess() }) == Elevation::Elevated;
            let mut gate = ElevationGate::new(self_elevated);
            for pid in rx {
                if let Some(signal) = gate.on_foreground(pid, process_elevation(pid)) {
                    let _ = signals.send(signal);
                }
            }
        })
        .ok()?;
    Some((tx, join))
}

fn process_elevation(pid: u32) -> Elevation {
    // SAFETY: OpenProcess returns null on failure; the handle is closed once.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Elevation::Unknown;
        }
        let e = token_elevation(process);
        CloseHandle(process);
        e
    }
}

fn token_elevation(process: HANDLE) -> Elevation {
    // SAFETY: valid out-pointers and sizes; the token handle is closed once.
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return Elevation::Unknown;
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
        match (ok != 0, info.TokenIsElevated != 0) {
            (false, _) => Elevation::Unknown,
            (true, true) => Elevation::Elevated,
            (true, false) => Elevation::NotElevated,
        }
    }
}
