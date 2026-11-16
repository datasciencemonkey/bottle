//! AT-17 / ARC-6 contract test for the C ABI (#224): no reply of `mog_call` carries a token
//! pattern of section 4.6 rule 7, whatever the request holds.
//!
//! Fake secrets are built at run time, so secret scanners do not flag the repo.

use std::ffi::{CStr, CString};

use mog_core::token_guard::{contains_secret, PATTERNS};
use mog_core_ffi::{mog_call, mog_free_string};

fn call(request: &str) -> String {
    let owned = CString::new(request).expect("no NUL in request");
    // SAFETY: `owned` is a valid NUL-terminated string that outlives the call.
    let out = unsafe { mog_call(owned.as_ptr()) };
    assert!(!out.is_null());
    // SAFETY: `out` is a non-null string that `mog_call` just returned.
    let text = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `out` came from `mog_call` and is freed once.
    unsafe { mog_free_string(out) };
    text
}

fn fakes() -> Vec<String> {
    let hex = "0123456789abcdef".repeat(2);
    vec![
        format!("{}{}.{}.sig", "ey", "JhbGciOiJub25lIn0", "eyJzdWIiOiJ4In0"),
        format!("{}{}{hex}", "dap", "i"),
        format!("{}{}a1b2c3", "do", "se"),
        format!("{}{}ABCDEFGHIJ234567", "AK", "IA"),
        format!("{}{}FakeFakeFakeFakeFakeFakeFakeFake0000", "gh", "p_"),
        format!("{}{} PRIVATE KEY-----", "-----", "BEGIN"),
        format!("{}{}hunter2", "pass", "word="),
    ]
}

#[test]
fn at_17_c_abi_replies_never_carry_a_token_pattern() {
    assert_eq!(fakes().len(), PATTERNS.len());
    for secret in fakes() {
        let quoted = serde_json::to_string(&secret).unwrap();
        let requests = [
            format!(r#"{{"command":{quoted},"args":{{}}}}"#),
            format!(
                r#"{{"command":"mcp_call","args":{{"profile":{quoted},"args":{{"q":{quoted}}}}}}}"#
            ),
            format!(
                r#"{{"command":"config_write","args":{{"id":"m","json":{{"token":{quoted}}}}}}}"#
            ),
            format!(r#"{{"command":"dbx_http","args":{{{quoted}:1}}}}"#),
            format!(r#"{{{quoted}:{quoted}}}"#),
            format!("not json {secret}"),
            quoted.clone(),
        ];
        for req in requests {
            let reply = call(&req);
            assert!(
                serde_json::from_str::<serde_json::Value>(&reply).is_ok(),
                "reply is not JSON: {reply}"
            );
            assert!(!contains_secret(&reply), "the C ABI leaked: {reply}");
        }
    }
}
