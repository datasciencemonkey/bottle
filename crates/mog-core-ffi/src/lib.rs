//! `mog-core-ffi`: the C ABI over `mog-core` (ARC-1, R15).
//!
//! R15 responsibilities (section 4.3): C ABI: `mog_init`, `mog_call` (JSON in, JSON out), `mog_set_event_callback`, `mog_abi_version`.
//! The C header is `include/mog_core.h` (cbindgen, checked by `tests/header.rs`).
//!
//! Rules:
//! - `mog_call` takes the IPC call envelope `{"command": "...", "args": {...}}` as a C string,
//!   with no other key. It always returns a JSON C string: `{"ok": <result>}` on success (no
//!   command has a result yet) or `{"error": "<code>"}` on failure. The caller frees it with
//!   `mog_free_string`.
//! - Every command goes through the hardened R12 gateway `hardened_gateway_with` (ADR 0019) with
//!   the label `brain`: the FFI caller has the brain window class (ADR 0006). The gateway checks
//!   the window and the arguments, writes one content-free log line, and scrubs replies. Errors
//!   cross only as fixed codes, for example `{"error":"not_implemented"}`, with no command name,
//!   window label or value (ADR 0019 decision 2).
//! - Events reach the host as `{"event": "<name>", "payload": {...}}`, the message shape of the
//!   brain Channel of the IPC surface (#223). One delivery thread calls the one callback, so the
//!   core side never waits for the host (NFR-18b). The queue holds `MOG_EVENT_QUEUE_CAPACITY`
//!   events; when it is full, new events are dropped and counted ([`dropped_events`]).
//! - No token or secret crosses the ABI (ARC-6, NFR-18a): `mog_init` takes no config key, and
//!   every reply and event passes the shared rule-7 guard `mog_core::token_guard` before it
//!   leaves (ADR 0013).
//! - Every entry point that can panic runs inside `catch_unwind`; null and non-UTF-8 input give
//!   an error.

#![deny(unsafe_op_in_unsafe_fn)]

use std::cell::Cell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;

use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc_gateway::{
    hardened_gateway_with, CheckedGateway, GatewayComponents, IpcGateway, NoLog, ScrubbingGateway,
};
use mog_core::token_guard::{contains_secret, json_contains_secret};
use serde_json::{json, Value};

/// The ABI version that `mog_abi_version` returns (NFR-18c).
pub const MOG_ABI_VERSION: u32 = 1;

/// Status: success.
pub const MOG_STATUS_OK: i32 = 0;
/// Status: an argument is not valid (for example null where a value is needed, not UTF-8,
/// not JSON, or a config key).
pub const MOG_STATUS_INVALID_ARGUMENT: i32 = 1;
/// Status: an internal error (a caught panic). The core state is unchanged.
/// Code 2 stays unused: the stubs returned it as "not implemented".
pub const MOG_STATUS_INTERNAL: i32 = 3;

/// The window label that R12 sees for calls through the C ABI. The ABI host takes the place of
/// the brain window, so it gets the brain window class (ADR 0006).
pub const FFI_WINDOW_LABEL: &str = "brain";

/// The number of events the queue holds while the host callback runs. When the queue is full,
/// `emit_event` drops the new event and counts it, so the core never waits (NFR-18b).
pub const MOG_EVENT_QUEUE_CAPACITY: usize = 1024;

/// The event callback: a JSON event as a C string, plus the host's `user_data`. Null clears it.
///
/// The string is valid only during the call; the host must copy it and must not free it.
/// The callback must return in 1 ms or less (NFR-18b). It may call any ABI function. One thread
/// calls it, never two at once.
pub type MogEventCallback =
    Option<extern "C" fn(event_json: *const c_char, user_data: *mut c_void)>;

/// Re-export the core version so the FFI crate proves it links `mog-core`.
pub fn linked_core_version() -> &'static str {
    mog_core::core_version()
}

/// The R1-R17 components behind the gateway for the C ABI. No component is wired yet, so every
/// checked call returns `not_implemented`, except `brain_subscribe`: the event callback
/// replaces the brain Channel, so it returns `not_supported`. Feature issues fill this in.
#[derive(Debug, Default, Clone, Copy)]
struct FfiComponents;

impl GatewayComponents for FfiComponents {
    fn call(&self, command: &'static str, _args: Value) -> CoreResult<Value> {
        match command {
            "brain_subscribe" => Err(CoreError::Unavailable {
                component: "R15",
                reason: "not_supported",
            }),
            _ => Err(CoreError::IpcRefused {
                code: "not_implemented",
            }),
        }
    }
}

type Gateway = ScrubbingGateway<CheckedGateway<NoLog, FfiComponents>>;

/// Set by `mog_init`. The log sink is `NoLog` until the log backend is chosen (ADR 0019).
static GATEWAY: OnceLock<Gateway> = OnceLock::new();

/// Return the ABI version (NFR-18c).
#[no_mangle]
pub extern "C" fn mog_abi_version() -> u32 {
    MOG_ABI_VERSION
}

/// Start the core. `config_json` is null or a JSON object; ABI v1 defines no config key, so
/// only `{}` is valid. A second call is a no-op that returns `MOG_STATUS_OK`.
///
/// Returns a `MOG_STATUS_*` code.
///
/// # Safety
///
/// `config_json` must be null or point to a NUL-terminated string that stays valid for the call.
#[no_mangle]
pub unsafe extern "C" fn mog_init(config_json: *const c_char) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if !config_json.is_null() {
            // SAFETY: the caller guarantees a valid NUL-terminated string (see `# Safety`).
            let config = unsafe { CStr::from_ptr(config_json) };
            // Any key is refused, so no token, host or header can enter here (ARC-6).
            match config.to_str().map(serde_json::from_str::<Value>) {
                Ok(Ok(Value::Object(keys))) if keys.is_empty() => {}
                _ => return MOG_STATUS_INVALID_ARGUMENT,
            }
        }
        GATEWAY.get_or_init(|| hardened_gateway_with(NoLog, FfiComponents));
        MOG_STATUS_OK
    }))
    .unwrap_or(MOG_STATUS_INTERNAL)
}

/// Run one command. `request_json` is `{"command": "...", "args": {...}}`.
///
/// Always returns a non-null JSON C string, `{"ok": ...}` or `{"error": "<code>"}`, that the
/// caller must free with `mog_free_string`.
///
/// # Safety
///
/// `request_json` must be null or point to a NUL-terminated string that stays valid for the call.
#[no_mangle]
pub unsafe extern "C" fn mog_call(request_json: *const c_char) -> *mut c_char {
    let reply = catch_unwind(AssertUnwindSafe(|| {
        if request_json.is_null() {
            return error_json("invalid_argument");
        }
        // SAFETY: the caller guarantees a valid NUL-terminated string (see `# Safety`).
        let request = unsafe { CStr::from_ptr(request_json) };
        match request.to_str() {
            Ok(text) => call_json(text),
            Err(_) => error_json("invalid_argument"),
        }
    }))
    .unwrap_or_else(|_| error_json("internal"));
    into_c_string(reply)
}

/// Free a string that `mog_call` returned. Null is a no-op.
///
/// # Safety
///
/// `s` must be null or a pointer that `mog_call` returned and that is not freed yet.
#[no_mangle]
pub unsafe extern "C" fn mog_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `s` came from `CString::into_raw` in `mog_call` and is freed only once.
        drop(unsafe { CString::from_raw(s) });
    }));
}

/// Set the one event callback, or clear it with null. `user_data` goes back to each call.
///
/// When this returns, the old callback is not running and is never called again, except when
/// a callback calls this function itself. So this call blocks until a callback in flight
/// returns: the host must not call it while it holds a lock that the callback takes.
///
/// The host must call `mog_set_event_callback(NULL, NULL)` to clear the callback
/// before it frees `user_data` or exits. The library does not support unloading: the delivery
/// thread never exits.
///
/// Returns a `MOG_STATUS_*` code.
#[no_mangle]
pub extern "C" fn mog_set_event_callback(
    callback: MogEventCallback,
    user_data: *mut c_void,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        *lock(&SUBSCRIBER) = callback.map(|callback| Subscriber {
            callback,
            user_data: user_data as usize,
        });
        // Wait for a call in flight to end. The delivery thread itself must not wait for it.
        if !ON_DELIVERY_THREAD.with(Cell::get) {
            drop(lock(&IN_FLIGHT));
        }
        MOG_STATUS_OK
    }))
    .unwrap_or(MOG_STATUS_INTERNAL)
}

/// Queue one core event for the host callback as `{"event": ..., "payload": ...}`.
///
/// Never waits for the host (NFR-18b). Returns `false` and drops the event if it holds a
/// rule-7 secret (ARC-6), if the queue is full (counted in [`dropped_events`]), or if the
/// delivery thread is not available.
pub fn emit_event(event: &str, payload: Value) -> bool {
    if contains_secret(event) || json_contains_secret(&payload) {
        return false;
    }
    let Ok(c) = CString::new(json!({ "event": event, "payload": payload }).to_string()) else {
        return false;
    };
    match events().try_send(c) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            false
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// The number of events dropped because the queue was full, since the process started.
pub fn dropped_events() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

static DROPPED: AtomicU64 = AtomicU64::new(0);

/// The host callback and its `user_data` (kept as an address so the slot is `Send`).
#[derive(Clone, Copy)]
struct Subscriber {
    callback: extern "C" fn(*const c_char, *mut c_void),
    user_data: usize,
}

static SUBSCRIBER: Mutex<Option<Subscriber>> = Mutex::new(None);
/// Held by the delivery thread for the length of each callback.
static IN_FLIGHT: Mutex<()> = Mutex::new(());

thread_local! {
    static ON_DELIVERY_THREAD: Cell<bool> = const { Cell::new(false) };
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The queue of the one delivery thread, started on first use.
fn events() -> &'static SyncSender<CString> {
    static QUEUE: OnceLock<SyncSender<CString>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<CString>(MOG_EVENT_QUEUE_CAPACITY);
        // If the spawn fails, `rx` drops and `emit_event` returns false.
        let _ = thread::Builder::new()
            .name("mog-ffi-events".to_owned())
            .spawn(move || {
                ON_DELIVERY_THREAD.with(|f| f.set(true));
                for event in rx {
                    let _in_flight = lock(&IN_FLIGHT);
                    let subscriber = *lock(&SUBSCRIBER);
                    if let Some(s) = subscriber {
                        (s.callback)(event.as_ptr(), s.user_data as *mut c_void);
                    }
                }
            });
        tx
    })
}

/// Parse the envelope and send it through the hardened R12 gateway. The reply passes the
/// shared rule-7 guard (ARC-6).
fn call_json(text: &str) -> String {
    let Some(gateway) = GATEWAY.get() else {
        return error_json("not_initialized");
    };
    let Ok(Value::Object(mut envelope)) = serde_json::from_str(text) else {
        return error_json("invalid_json");
    };
    let (Some(Value::String(command)), Some(args)) =
        (envelope.remove("command"), envelope.remove("args"))
    else {
        return error_json("invalid_json");
    };
    if !envelope.is_empty() {
        return error_json("invalid_json");
    }
    match gateway.dispatch(FFI_WINDOW_LABEL, &command, args) {
        Ok(value) if json_contains_secret(&value) => error_json("secret_blocked"),
        Ok(value) => json!({ "ok": value }).to_string(),
        // Every `CoreError::code` is a fixed string, for example `IpcRefused { code }`.
        Err(e) => error_json(e.code()),
    }
}

fn error_json(code: &str) -> String {
    json!({ "error": code }).to_string()
}

fn into_c_string(s: String) -> *mut c_char {
    // serde_json escapes NUL as \u0000, so `CString::new` cannot fail; keep a safe fallback.
    CString::new(s)
        .unwrap_or_else(|_| CString::from(c"{\"error\":\"internal\"}"))
        .into_raw()
}

#[cfg(test)]
mod tests {
    #[test]
    fn arc_1_ffi_crate_links_mog_core() {
        assert_eq!(super::linked_core_version(), mog_core::core_version());
    }
}
