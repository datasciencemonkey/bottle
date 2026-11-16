//! `mog_call` before `mog_init` (NFR-18a). A separate test binary, so no other test has
//! started the core in this process.

use std::ffi::{CStr, CString};

use mog_core_ffi::{mog_call, mog_free_string};

#[test]
fn nfr_18a_mog_call_before_mog_init_returns_not_initialized() {
    let request = CString::new(r#"{"command":"cli_status","args":{}}"#).expect("request");
    // SAFETY: `request` is a valid NUL-terminated string for the whole call.
    let out = unsafe { mog_call(request.as_ptr()) };
    assert!(!out.is_null());
    // SAFETY: `out` is a non-null string that `mog_call` just returned.
    let reply = unsafe { CStr::from_ptr(out) }
        .to_str()
        .expect("UTF-8")
        .to_owned();
    // SAFETY: `out` came from `mog_call` and is freed once.
    unsafe { mog_free_string(out) };
    let json: serde_json::Value = serde_json::from_str(&reply).expect("reply is JSON");
    assert_eq!(json, serde_json::json!({"error": "not_initialized"}));
}
