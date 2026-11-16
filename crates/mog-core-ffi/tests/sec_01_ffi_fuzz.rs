//! #36: FFI fuzz test for the C ABI (SEC-01, SEC-03, AT-17).
//!
//! Sends seeded, deterministic `mog_call` envelopes that hold fake secrets in the command name,
//! the argument keys and the values, and scans every reply for the rule-7 patterns of 4.6.
//! It uses only the ABI that is on main. TODO(#176): when `mog_call` dispatches through the
//! hardened R12 gateway, this test covers that path with no change.
//!
//! Every fake secret is built at run time from pieces. No test reads a real profile or token.

use std::ffi::{CStr, CString};

use mog_core::ipc::COMMANDS;
use mog_core::token_guard::contains_secret;
use mog_core_ffi::{mog_call, mog_free_string, mog_init};
use serde_json::{json, Map, Value};

fn fakes() -> Vec<String> {
    vec![
        format!("{}{}.{}.sig", "ey", "JhbGciOiJub25lIn0", "eyJzdWIiOiJ4In0"),
        format!("{}{}", "dap", "i") + &"0123456789abcdef".repeat(2),
        format!("{}{}", "do", "se") + "a1b2c3d4e5f6",
        format!("{}{}", "AK", "IA") + "ABCDEFGHIJ234567",
        format!("{}{}", "gh", "p_") + "FakeFakeFakeFakeFakeFakeFakeFake0000",
        format!("{}{} PRIVATE KEY-----\nMIIfake", "-----", "BEGIN"),
        format!("{}{}", "pass", "word=") + "hunter2",
    ]
}

/// splitmix64, seeded, so CI sees the same cases on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn text(&mut self, secrets: &[String]) -> String {
        match self.below(3) {
            0 => secrets[self.below(secrets.len())].clone(),
            _ => (0..self.below(10))
                .map(|_| (b'a' + self.below(26) as u8) as char)
                .collect(),
        }
    }
    fn json(&mut self, secrets: &[String], depth: u32) -> Value {
        match self.below(if depth == 0 { 3 } else { 5 }) {
            0 => json!(self.below(100)),
            1 | 2 => json!(self.text(secrets)),
            3 => Value::Array(
                (0..self.below(3))
                    .map(|_| self.json(secrets, depth - 1))
                    .collect(),
            ),
            _ => {
                let mut m = Map::new();
                for _ in 0..self.below(4) {
                    m.insert(self.text(secrets), self.json(secrets, depth - 1));
                }
                Value::Object(m)
            }
        }
    }
}

/// Call `mog_call` with `request` and return the reply text.
fn call(request: &str) -> String {
    let c = CString::new(request).unwrap();
    // SAFETY: `c` is a valid NUL-terminated string for the call; the reply is freed once.
    unsafe {
        let out = mog_call(c.as_ptr());
        assert!(!out.is_null());
        let text = CStr::from_ptr(out).to_str().unwrap().to_owned();
        mog_free_string(out);
        text
    }
}

#[test]
fn sec_01_at_17_ffi_fuzz_finds_no_secret() {
    let secrets = fakes();
    let mut rng = Rng(36);
    for i in 0..3000 {
        let command = match rng.below(3) {
            0 => rng.text(&secrets),
            _ => COMMANDS[rng.below(COMMANDS.len())].name.to_owned(),
        };
        let request = match rng.below(6) {
            // Not JSON, or JSON of the wrong shape, with a secret inside.
            0 => format!("{{\"command\": \"{}", secrets[rng.below(secrets.len())]),
            1 => rng.json(&secrets, 3).to_string(),
            _ => json!({"command": command, "args": rng.json(&secrets, 3)}).to_string(),
        };
        let reply = call(&request);
        assert!(!contains_secret(&reply), "case {i}: {reply}");
        let parsed: Value = serde_json::from_str(&reply).expect("the reply is JSON");
        assert!(parsed.is_object(), "case {i}: {reply}");
    }
}

#[test]
fn sec_01_at_17_ffi_init_with_a_secret_config_returns_only_a_status() {
    for secret in fakes() {
        let config = CString::new(json!({ "token": secret }).to_string()).unwrap();
        // SAFETY: `config` is a valid NUL-terminated string for the call.
        let status = unsafe { mog_init(config.as_ptr()) };
        // An integer status cannot carry a secret; it must be one of the ABI codes.
        assert!((0..256).contains(&status), "{status}");
    }
}
