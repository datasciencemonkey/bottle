//! Contract tests for the R15 C ABI stubs (NFR-18a, NFR-18c). The real ABI is #176.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::ptr;

use mog_core_ffi::{
    mog_abi_version, mog_call, mog_free_string, mog_init, mog_set_event_callback, MOG_ABI_VERSION,
    MOG_STATUS_NOT_IMPLEMENTED,
};

/// Call `mog_call` and return the JSON reply as a Rust string, then free it.
fn call(request: Option<&str>) -> String {
    let owned = request.map(|r| CString::new(r).expect("no NUL in request"));
    let input = owned.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    // SAFETY: `input` is null or a valid NUL-terminated string that outlives the call.
    let out = unsafe { mog_call(input) };
    assert!(!out.is_null(), "mog_call must always return a string");
    // SAFETY: `out` is a non-null string that `mog_call` just returned.
    let text = unsafe { CStr::from_ptr(out) }
        .to_str()
        .expect("UTF-8 reply")
        .to_owned();
    // SAFETY: `out` came from `mog_call` and is freed once.
    unsafe { mog_free_string(out) };
    text
}

/// The outbound secret patterns of section 4.6 rule 7, as plain substrings.
fn looks_like_a_token(s: &str) -> bool {
    [
        "eyJ",
        "dapi",
        "dose",
        "AKIA",
        "ghp_",
        "-----BEGIN",
        "password=",
    ]
    .iter()
    .any(|p| s.contains(p))
}

#[test]
fn nfr_18c_abi_version_returns_a_number() {
    let v: u32 = mog_abi_version();
    assert_eq!(v, 1);
    assert_eq!(v, MOG_ABI_VERSION);
}

#[test]
fn nfr_18a_mog_call_returns_json() {
    let reply = call(Some(r#"{"command":"cli_status","args":{}}"#));
    let json: serde_json::Value = serde_json::from_str(&reply).expect("reply is JSON");
    assert!(json.is_object());
}

#[test]
fn r15_mog_call_returns_not_implemented_through_r12() {
    let reply = call(Some(
        r#"{"command":"profiles_list","args":{"refresh":true}}"#,
    ));
    let json: serde_json::Value = serde_json::from_str(&reply).expect("reply is JSON");
    assert_eq!(json, serde_json::json!({"error": "not_implemented"}));
}

#[test]
fn r15_mog_call_with_null_returns_an_error_json() {
    let json: serde_json::Value = serde_json::from_str(&call(None)).expect("reply is JSON");
    assert_eq!(json["error"], "invalid_argument");
}

#[test]
fn r15_mog_call_with_bad_json_returns_an_error_json() {
    let json: serde_json::Value = serde_json::from_str(&call(Some("not json"))).expect("JSON");
    assert_eq!(json["error"], "invalid_json");
    let json: serde_json::Value = serde_json::from_str(&call(Some("[1]"))).expect("JSON");
    assert_eq!(json["error"], "invalid_json");
}

#[test]
fn r15_mog_free_string_accepts_null() {
    // SAFETY: null is documented as a no-op.
    unsafe { mog_free_string(ptr::null_mut()) };
}

#[test]
fn r15_mog_init_reports_not_implemented() {
    // SAFETY: null config is allowed (defaults).
    assert_eq!(unsafe { mog_init(ptr::null()) }, MOG_STATUS_NOT_IMPLEMENTED);
    let cfg = CString::new("{}").expect("cfg");
    // SAFETY: `cfg` is a valid NUL-terminated string for the whole call.
    assert_eq!(
        unsafe { mog_init(cfg.as_ptr()) },
        MOG_STATUS_NOT_IMPLEMENTED
    );
}

extern "C" fn on_event(_event_json: *const c_char, _user_data: *mut c_void) {}

#[test]
fn r15_mog_set_event_callback_reports_not_implemented() {
    assert_eq!(
        mog_set_event_callback(Some(on_event), ptr::null_mut()),
        MOG_STATUS_NOT_IMPLEMENTED
    );
    assert_eq!(
        mog_set_event_callback(None, ptr::null_mut()),
        MOG_STATUS_NOT_IMPLEMENTED
    );
}

#[test]
fn nfr_18a_mog_call_returns_no_token_like_string() {
    let fake = format!("dapi{}", "0123456789abcdef0123456789abcdef");
    let requests = [
        format!(r#"{{"command":"profile_status","args":{{"profile":"{fake}"}}}}"#),
        // Built at run time so no secret-shaped literal sits in the source.
        format!(
            r#"{{"command":"{}.e30.sig","args":{{"{}":"{}"}}}}"#,
            ["ey", "J", "hbGciOiJIUzI1NiJ9"].concat(),
            ["pass", "word="].concat(),
            ["AK", "IA", "ABCDEFGHIJKLMNOP"].concat()
        ),
        format!(
            "{} {}",
            ["-----", "BEGIN KEY-----"].concat(),
            ["gh", "p_abc"].concat()
        ),
    ];
    for request in &requests {
        let reply = call(Some(request));
        assert!(
            !looks_like_a_token(&reply),
            "reply echoes a secret: {reply}"
        );
    }
    assert!(!looks_like_a_token(&call(None)));
}
