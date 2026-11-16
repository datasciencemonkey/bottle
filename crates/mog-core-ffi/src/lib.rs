//! `mog-core-ffi`: the C ABI over `mog-core` (ARC-1, R15).
//!
//! R15 responsibilities (section 4.3): C ABI: `mog_init`, `mog_call` (JSON in, JSON out), `mog_set_event_callback`, `mog_abi_version`.
//!
//! These are stubs. TODO(#176): the real ABI (init, event callback, thread safety, NFR-18b).
//!
//! Rules that hold already:
//! - `mog_call` takes one JSON envelope `{"command": "...", "args": {...}}` as a C string and
//!   always returns a JSON C string. The caller frees it with `mog_free_string`.
//! - Every command goes through R12 `IpcGateway::dispatch`. Errors cross the ABI only as a
//!   content-free code, for example `{"error":"not_implemented"}`, so no secret crosses (ARC-6,
//!   NFR-18a).
//! - No function panics across the ABI; null pointers give an error, not a crash.

#![deny(unsafe_op_in_unsafe_fn)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use mog_core::ipc_gateway::{IpcGateway, StubIpcGateway};

/// The ABI version that `mog_abi_version` returns (NFR-18c).
pub const MOG_ABI_VERSION: u32 = 1;

/// Status: success.
pub const MOG_STATUS_OK: i32 = 0;
/// Status: an argument is not valid (for example not UTF-8).
pub const MOG_STATUS_INVALID_ARGUMENT: i32 = 1;
/// Status: the function is a stub.
pub const MOG_STATUS_NOT_IMPLEMENTED: i32 = 2;

/// The window label that R12 sees for calls through the C ABI.
/// TODO(#176): agree the label and its capability set with the #223 command table.
pub const FFI_WINDOW_LABEL: &str = "ffi";

/// The event callback type: a JSON event as a C string, plus the caller's `user_data`.
pub type MogEventCallback = extern "C" fn(event_json: *const c_char, user_data: *mut c_void);

/// Re-export the core version so the FFI crate proves it links `mog-core`.
pub fn linked_core_version() -> &'static str {
    mog_core::core_version()
}

/// Return the ABI version (NFR-18c).
#[no_mangle]
pub extern "C" fn mog_abi_version() -> u32 {
    MOG_ABI_VERSION
}

/// Start the core with a JSON config. `config_json` may be null (defaults).
///
/// Returns a `MOG_STATUS_*` code. The stub returns `MOG_STATUS_NOT_IMPLEMENTED`.
///
/// # Safety
///
/// `config_json` must be null or point to a NUL-terminated string that stays valid for the call.
#[no_mangle]
pub unsafe extern "C" fn mog_init(config_json: *const c_char) -> i32 {
    if !config_json.is_null() {
        // SAFETY: the caller guarantees a valid NUL-terminated string (see `# Safety`).
        let config = unsafe { CStr::from_ptr(config_json) };
        if config.to_str().is_err() {
            return MOG_STATUS_INVALID_ARGUMENT;
        }
    }
    // TODO(#176): start the core from the config.
    MOG_STATUS_NOT_IMPLEMENTED
}

/// Run one command. `request_json` is `{"command": "...", "args": {...}}`.
///
/// Always returns a non-null JSON C string that the caller must free with `mog_free_string`.
///
/// # Safety
///
/// `request_json` must be null or point to a NUL-terminated string that stays valid for the call.
#[no_mangle]
pub unsafe extern "C" fn mog_call(request_json: *const c_char) -> *mut c_char {
    let reply = if request_json.is_null() {
        error_json("invalid_argument")
    } else {
        // SAFETY: the caller guarantees a valid NUL-terminated string (see `# Safety`).
        let request = unsafe { CStr::from_ptr(request_json) };
        match request.to_str() {
            Ok(text) => call_json(text),
            Err(_) => error_json("invalid_argument"),
        }
    };
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

/// Set (or clear, with `None`) the event callback. The stub stores nothing.
///
/// Returns a `MOG_STATUS_*` code. The stub returns `MOG_STATUS_NOT_IMPLEMENTED`.
#[no_mangle]
pub extern "C" fn mog_set_event_callback(
    _callback: Option<MogEventCallback>,
    _user_data: *mut c_void,
) -> i32 {
    // TODO(#176): one thread-safe event callback that returns in 1 ms or less (NFR-18b).
    MOG_STATUS_NOT_IMPLEMENTED
}

/// Parse the envelope and send it through R12. Only content-free codes come back on error.
fn call_json(text: &str) -> String {
    let Ok(serde_json::Value::Object(mut envelope)) = serde_json::from_str(text) else {
        return error_json("invalid_json");
    };
    let Some(serde_json::Value::String(command)) = envelope.remove("command") else {
        return error_json("invalid_json");
    };
    let args = envelope.remove("args").unwrap_or(serde_json::Value::Null);
    match StubIpcGateway.dispatch(FFI_WINDOW_LABEL, &command, args) {
        Ok(value) => value.to_string(),
        Err(e) => error_json(e.code()),
    }
}

fn error_json(code: &str) -> String {
    serde_json::json!({ "error": code }).to_string()
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
