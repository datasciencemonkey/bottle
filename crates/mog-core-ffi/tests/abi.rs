//! Contract tests for the R15 C ABI: JSON in and out, init, version (NFR-18a, NFR-18c, ARC-6).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use mog_core_ffi::{
    emit_event, mog_abi_version, mog_call, mog_free_string, mog_init, MOG_ABI_VERSION,
    MOG_STATUS_INVALID_ARGUMENT, MOG_STATUS_OK,
};

/// Start the core once. `mog_init` is idempotent, so every test may call it.
fn init() {
    // SAFETY: null config is allowed (defaults).
    assert_eq!(unsafe { mog_init(ptr::null()) }, MOG_STATUS_OK);
}

/// Call `mog_call` with raw bytes (or null) and return the reply as a Rust string, then free it.
fn call_raw(request: Option<&[u8]>) -> String {
    let owned = request.map(|r| CString::new(r).expect("no NUL in request"));
    let input: *const c_char = owned.as_ref().map_or(ptr::null(), |c| c.as_ptr());
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

fn call(request: Option<&str>) -> String {
    init();
    call_raw(request.map(str::as_bytes))
}

fn json(reply: &str) -> serde_json::Value {
    serde_json::from_str(reply).expect("reply is JSON")
}

fn init_with(config: &[u8]) -> i32 {
    let cfg = CString::new(config).expect("no NUL in config");
    // SAFETY: `cfg` is a valid NUL-terminated string for the whole call.
    unsafe { mog_init(cfg.as_ptr()) }
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
    assert!(json(&reply).is_object());
}

#[test]
fn nfr_18a_mog_call_returns_not_implemented_through_r12() {
    let reply = call(Some(
        r#"{"command":"profiles_list","args":{"refresh":true}}"#,
    ));
    assert_eq!(
        json(&reply),
        serde_json::json!({"error": "not_implemented"})
    );
}

#[test]
fn sec_02a_ffi_caller_has_the_brain_window_class() {
    // A brain command passes the window check; a view-only command does not. Refusals cross
    // only as fixed codes, with no command name or window label (ADR 0019 decision 2).
    let reply = call(Some(r#"{"command":"capture_status","args":{}}"#));
    assert_eq!(
        json(&reply),
        serde_json::json!({"error": "not_implemented"})
    );
    let reply = call(Some(
        r#"{"command":"view_emit","args":{"mogId":"m1","kind":"poked"}}"#,
    ));
    assert_eq!(
        json(&reply),
        serde_json::json!({"error": "window_not_allowed"})
    );
}

#[test]
fn sec_02b_ffi_args_with_an_extra_field_are_rejected() {
    let reply = call(Some(
        r#"{"command":"profiles_list","args":{"refresh":true,"extra":1}}"#,
    ));
    assert_eq!(json(&reply), serde_json::json!({"error": "invalid_args"}));
}

#[test]
fn nfr_18a_unknown_command_is_not_echoed() {
    let reply = call(Some(r#"{"command":"no_such_command","args":{}}"#));
    assert_eq!(
        json(&reply),
        serde_json::json!({"error": "unknown_command"})
    );
}

#[test]
fn nfr_18a_brain_subscribe_is_not_supported_over_the_abi() {
    // The event callback replaces the brain Channel of the Tauri host.
    let reply = call(Some(
        r#"{"command":"brain_subscribe","args":{"onEvent":"__CHANNEL__:1"}}"#,
    ));
    assert_eq!(json(&reply), serde_json::json!({"error": "not_supported"}));
}

#[test]
fn nfr_18a_mog_call_with_null_returns_an_error_json() {
    assert_eq!(json(&call(None))["error"], "invalid_argument");
}

#[test]
fn nfr_18a_mog_call_with_invalid_utf8_returns_an_error_json() {
    init();
    let reply = call_raw(Some(b"{\"command\":\"\xff\xfe\",\"args\":{}}"));
    assert_eq!(
        json(&reply),
        serde_json::json!({"error": "invalid_argument"})
    );
}

#[test]
fn nfr_18a_mog_call_with_bad_json_returns_an_error_json() {
    for bad in ["not json", "[1]", "null", "", "{\"command\":1,\"args\":{}}"] {
        assert_eq!(
            json(&call(Some(bad)))["error"],
            "invalid_json",
            "input {bad:?}"
        );
    }
}

#[test]
fn nfr_18a_mog_call_takes_only_command_and_args() {
    // The envelope of the IPC surface: `command` plus `args`, and no other key.
    for bad in [
        r#"{"command":"cli_status"}"#,
        r#"{"args":{}}"#,
        r#"{"command":"cli_status","args":{},"headers":{}}"#,
    ] {
        assert_eq!(
            json(&call(Some(bad)))["error"],
            "invalid_json",
            "input {bad}"
        );
    }
}

#[test]
fn nfr_18a_mog_free_string_accepts_null() {
    // SAFETY: null is documented as a no-op.
    unsafe { mog_free_string(ptr::null_mut()) };
}

#[test]
fn nfr_18a_mog_init_accepts_null_and_an_empty_object_and_is_idempotent() {
    init();
    init();
    assert_eq!(init_with(b"{}"), MOG_STATUS_OK);
    assert_eq!(init_with(b" { } "), MOG_STATUS_OK);
}

#[test]
fn nfr_18a_mog_init_rejects_bad_config() {
    for bad in [&b"not json"[..], b"[1]", b"null", b"", b"\xff\xfe"] {
        assert_eq!(
            init_with(bad),
            MOG_STATUS_INVALID_ARGUMENT,
            "config {bad:?}"
        );
    }
}

#[test]
fn arc_6_mog_init_rejects_any_config_key_so_no_token_enters() {
    // ABI v1 defines no config key. A token, a host or a header can never enter this way.
    let fake = format!("dapi{}", "0123456789abcdef0123456789abcdef");
    for bad in [
        format!(r#"{{"token":"{fake}"}}"#),
        r#"{"host":"https://example.cloud.databricks.com"}"#.to_owned(),
        r#"{"profile":"bottle-dev"}"#.to_owned(),
    ] {
        assert_eq!(
            init_with(bad.as_bytes()),
            MOG_STATUS_INVALID_ARGUMENT,
            "config {bad}"
        );
    }
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

/// A small deterministic generator, so the fuzz run is the same on every machine.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[(self.next() % items.len() as u64) as usize]
    }
}

#[test]
fn sec_01_fuzzed_ffi_calls_return_json_without_secrets() {
    // AT-17: malicious FFI calls. Every reply is JSON, never null, and never echoes a secret.
    let dapi = format!("dapi{}", "0123456789abcdef0123456789abcdef");
    let jwt = ["ey", "JhbGciOiJIUzI1NiJ9", ".e30.sig"].concat();
    let akia = ["AK", "IA", "ABCDEFGHIJKLMNOP"].concat();
    let ghp = ["gh", "p_", "abcdefghijklmnopqrstuvwxyz0123456789"].concat();
    let pem = ["-----", "BEGIN PRIVATE KEY-----"].concat();
    let pwd = ["pass", "word=hunter2"].concat();
    let dose = ["do", "se0123abcd"].concat();
    let secrets = [&dapi[..], &jwt, &akia, &ghp, &pem, &pwd, &dose];
    let commands = [
        "cli_status",
        "profile_status",
        "dbx_http",
        "mcp_call",
        "config_write",
    ];
    let pieces = [
        "{", "}", "[", "]", ":", ",", "\"", "\\", "\\u0000", "null", "1e999",
    ];
    let mut rng = XorShift(0x5EED_0176);
    for i in 0..2_000 {
        let secret = rng.pick(&secrets);
        let request = match i % 4 {
            0 => format!(
                r#"{{"command":"{}","args":{{"profile":"{secret}","token":"{secret}"}}}}"#,
                rng.pick(&commands)
            ),
            1 => format!(r#"{{"command":"{secret}","args":{{}}}}"#),
            2 => format!(r#"{{"command":"cli_status","args":{{}},"{secret}":1}}"#),
            _ => (0..8).map(|_| rng.pick(&pieces)).collect::<String>() + secret,
        };
        let reply = call(Some(&request));
        assert!(
            json(&reply).is_object(),
            "reply is not a JSON object: {reply}"
        );
        assert!(
            !looks_like_a_token(&reply),
            "reply echoes a secret: {reply}"
        );
    }
}

#[test]
fn arc_6_password_in_any_ascii_case_is_blocked_over_the_abi() {
    // ADR 0013: `password=` matches in any ASCII case. An event that holds it never leaves.
    for key in ["PASSWORD=", "Password=", "pAsSwOrD="] {
        let value = format!("{key}hunter2");
        assert!(
            !emit_event("auth:status", serde_json::json!({ "note": value })),
            "{key} in the payload passed"
        );
        assert!(
            !emit_event(&value, serde_json::json!({})),
            "{key} in the name passed"
        );
    }
}
