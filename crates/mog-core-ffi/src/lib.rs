//! `mog-core-ffi`: the C ABI over `mog-core` (ARC-1, R15).
//!
//! R15 responsibilities (section 4.3): C ABI: `mog_init`, `mog_call` (JSON in, JSON out), `mog_set_event_callback`, `mog_abi_version`.
//! The C header is `include/mog_core.h` (cbindgen, checked by `tests/header.rs`).
//!
//! Rules:
//! - `mog_call` takes the IPC call envelope `{"command": "...", "args": {...}}` as a C string,
//!   with no other key. It always returns a JSON C string: `{"ok": <result>}` on success (no
//!   command has a result yet) or `{"error": "<code>", ...}` on failure. The caller frees it
//!   with `mog_free_string`.
//! - Every command goes through R12 `mog_core::ipc::handle`, the same gateway as the Tauri host
//!   (ADR 0003, ADR 0004). The FFI caller has the brain window class. Errors are the serialized
//!   `IpcError` (for example `{"error":"not_implemented","command":"cli_status"}`), except that
//!   an unknown command name is never echoed.
//! - Events reach the host as `{"event": "<name>", "payload": {...}}`, the message shape of the
//!   brain Channel of the IPC surface (#223). One delivery thread calls the one callback, so the
//!   core side never waits for the host (NFR-18b).
//! - No token or secret crosses the ABI (ARC-6, NFR-18a): `mog_init` takes no config key, and
//!   every reply and event passes the 4.6 rule 7 patterns before it leaves.
//! - No function panics across the ABI (`catch_unwind`); null and non-UTF-8 input give an error.

#![deny(unsafe_op_in_unsafe_fn)]

use std::cell::Cell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;

use mog_core::ipc::{self, Handled, IpcError};
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
/// the brain window, so it gets the brain window class (ADR 0004).
pub const FFI_WINDOW_LABEL: &str = "brain";

/// The event callback: a JSON event as a C string, plus the host's `user_data`. Null clears it.
///
/// The string is valid only during the call; the host must copy it and must not free it.
/// The callback must return in 1 ms or less (NFR-18b). It may call any ABI function.
pub type MogEventCallback =
    Option<extern "C" fn(event_json: *const c_char, user_data: *mut c_void)>;

/// Re-export the core version so the FFI crate proves it links `mog-core`.
pub fn linked_core_version() -> &'static str {
    mog_core::core_version()
}

/// Set by `mog_init`. The components behind `ipc::handle` start with their feature issues.
static STARTED: OnceLock<()> = OnceLock::new();

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
        STARTED.get_or_init(|| ());
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
    // SAFETY: `s` came from `CString::into_raw` in `mog_call` and is freed only once.
    drop(unsafe { CString::from_raw(s) });
}

/// Set the one event callback, or clear it with null. `user_data` goes back to each call.
///
/// When this returns, the old callback is not running and is never called again, except when
/// a callback calls this function itself. Returns a `MOG_STATUS_*` code.
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
/// Never waits for the host (NFR-18b). Returns `false` if the event looks like it holds a secret
/// (it is dropped, ARC-6) or if the delivery thread is not available.
pub fn emit_event(event: &str, payload: Value) -> bool {
    let text = json!({ "event": event, "payload": payload }).to_string();
    if contains_secret(&text) {
        return false;
    }
    match CString::new(text) {
        Ok(c) => events().send(c).is_ok(),
        Err(_) => false,
    }
}

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
fn events() -> &'static Sender<CString> {
    static QUEUE: OnceLock<Sender<CString>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<CString>();
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

/// Parse the envelope and send it through R12. The reply passes the secret guard (ARC-6).
fn call_json(text: &str) -> String {
    let reply = route(text);
    if contains_secret(&reply) {
        error_json("secret_blocked")
    } else {
        reply
    }
}

fn route(text: &str) -> String {
    if STARTED.get().is_none() {
        return error_json("not_initialized");
    }
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
    match ipc::handle(FFI_WINDOW_LABEL, &command, args) {
        // The event callback replaces the brain Channel; no view sends through the ABI.
        Ok(Handled::SubscribeBrain { .. } | Handled::RelayToBrain { .. }) => {
            json!({ "error": "not_supported", "command": command }).to_string()
        }
        // The name came from the caller and is not in the table, so it is not echoed.
        Err(IpcError::UnknownCommand { .. }) => error_json("unknown_command"),
        Err(e) => serde_json::to_string(&e).unwrap_or_else(|_| error_json("internal")),
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

/// The outbound secret patterns of section 4.6 rule 7: `eyJ[\w-]+\.`, `dapi[0-9a-f]{32}`,
/// `dose[0-9a-f]+`, `AKIA[0-9A-Z]{16}`, `ghp_…`, `-----BEGIN`, `password=`.
/// TODO(#36): use the shared R12 scrubber when it lands.
fn contains_secret(s: &str) -> bool {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-';
    let hex = |c: u8| c.is_ascii_digit() || (b'a'..=b'f').contains(&c);
    let upper = |c: u8| c.is_ascii_digit() || c.is_ascii_uppercase();
    let alnum = |c: u8| c.is_ascii_alphanumeric();
    s.contains("-----BEGIN")
        || s.contains("password=")
        || followed_by(s, "eyJ", |rest| {
            let n = run_len(rest, word);
            n > 0 && rest.get(n) == Some(&b'.')
        })
        || followed_by(s, "dapi", |rest| run_len(rest, hex) >= 32)
        || followed_by(s, "dose", |rest| run_len(rest, hex) >= 1)
        || followed_by(s, "AKIA", |rest| run_len(rest, upper) >= 16)
        || followed_by(s, "ghp_", |rest| run_len(rest, alnum) >= 1)
}

/// True if some `prefix` in `s` is followed by bytes that `rest_ok` accepts.
fn followed_by(s: &str, prefix: &str, rest_ok: impl Fn(&[u8]) -> bool) -> bool {
    s.match_indices(prefix)
        .any(|(i, _)| rest_ok(&s.as_bytes()[i + prefix.len()..]))
}

/// The number of leading bytes of `rest` that `pred` accepts.
fn run_len(rest: &[u8], pred: impl Fn(u8) -> bool) -> usize {
    rest.iter().take_while(|&&c| pred(c)).count()
}

#[cfg(test)]
mod tests {
    use super::contains_secret;

    #[test]
    fn arc_1_ffi_crate_links_mog_core() {
        assert_eq!(super::linked_core_version(), mog_core::core_version());
    }

    #[test]
    fn arc_6_outbound_guard_matches_the_4_6_patterns() {
        // Built at run time so no secret-shaped literal sits in the source.
        let hits = [
            ["ey", "JhbGciOiJIUzI1NiJ9", ".e30"].concat(),
            ["da", "pi", "0123456789abcdef0123456789abcdef"].concat(),
            ["do", "se", "0a1b"].concat(),
            ["AK", "IA", "ABCDEFGHIJ012345"].concat(),
            ["gh", "p_", "abc123"].concat(),
            ["-----", "BEGIN RSA PRIVATE KEY-----"].concat(),
            ["x pass", "word=hunter2"].concat(),
        ];
        for s in &hits {
            assert!(contains_secret(s), "not detected: {s}");
        }
        let misses = [
            "eyJ no dot",
            "dapi0123",
            "doses",
            "AKIAshort",
            "ghp_",
            "BEGIN",
            "password",
        ];
        for s in misses {
            assert!(!contains_secret(s), "false hit: {s}");
        }
    }
}
